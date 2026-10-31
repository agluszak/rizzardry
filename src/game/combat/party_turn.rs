//! A member's accepted turn: blows, a breath or a cast.

use super::*;

/// The party's armour as this round's blows meet it: the spells on each
/// member and the party's, and whether they defend (retail
/// `CalcArmorClasses`).
pub(super) fn party_armours(
    battle: &Battle,
    members: &Members,
    context: &MagicContext,
) -> Vec<(Entity, Armour)> {
    let items = &context.content.items;
    context
        .roster
        .iter()
        .map(|member| {
            let (character, vitals, carried, burden) = members.get(member).unwrap();
            let armour = attack::enchanted_armour(
                &character.stats(&vitals.enchantments),
                carried,
                items,
                burden.load,
                battle.defends(member),
                (&vitals.enchantments, Some(&context.enchantments.0)),
            );
            (member, armour)
        })
        .collect()
}

/// A member's accepted turn comes: revalidated, then their spell or attack.
/// Returns whether they acted; an actor who can no longer act loses the turn.
pub(super) fn party_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let work = battle.this_round.party[index];
    let member = work.actor;
    let able_now = magic.context.roster.contains(&member)
        && members.get(member).is_ok_and(|(_, vitals, carried, ..)| {
            vitals.can_fight()
                && match work.action {
                    // The weapons accepted, or nothing where one was used up.
                    RoundAction::Attack { weapons, .. } => {
                        [EquipSlot::Primary, EquipSlot::Secondary].into_iter().zip(weapons).all(
                            |(slot, accepted)| {
                                let now = carried.worn(slot).map(|i| i.id);
                                now.is_none() || now == accepted
                            },
                        )
                    }
                    RoundAction::Magic(_) | RoundAction::Breathe(_) | RoundAction::Defend => true,
                }
        });
    if !able_now {
        battle.this_round.party[index].end();
        return false;
    }
    match work.action {
        RoundAction::Breathe(target) => {
            battle.this_round.party[index].end();
            breathe(battle, index, target, combatants, rng, (members, damage), party, magic, news);
        }
        RoundAction::Magic(choice) => {
            battle.this_round.party[index].end();
            cast(
                commands,
                battle,
                index,
                choice,
                combatants,
                rng,
                (members, damage),
                party,
                magic,
                news,
            );
        }
        RoundAction::Attack { .. } => {
            let BattleMagic { context, points, moving, .. } = magic;
            let mut fight = blows::Fight {
                combatants,
                party,
                armours: party_armours(battle, members, context),
                moving,
                points,
                attacks: &|_| false,
                waits: false,
            };
            context.resolve(party.now, members, damage, |field| {
                blows::member_attacks(battle, index, rng, field, &mut fight, news);
            });
        }
        // Defending takes no turn.
        RoundAction::Defend => {
            battle.this_round.party[index].end();
            return false;
        }
    }
    true
}

/// A member breathes at their chosen target; a breath that cannot be made
/// falls back to defending.
pub(super) fn breathe(
    battle: &mut Battle,
    index: usize,
    target: Entity,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let result =
        member_acts(combatants, (members, damage), party, magic, |situation, field, points| {
            breath::execute(member, target, situation, rng, field, points)
        });
    match result {
        Ok(effects) => {
            battle.this_round.attacked = true;
            news.write(CombatNews::Breathed { actor: member });
            battle.credit_kills(&effects, news);
            news.write(CombatNews::Effects(effects));
        }
        Err(_) => battle.this_round.party[index].action = RoundAction::Defend,
    }
}

/// A member casts or uses an item. A refused action falls back to
/// defending (retail `ExecuteCharacterAction`), its turn spent; a point
/// cast needs the whole world and settles once this step is done.
pub(super) fn cast(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    choice: MagicChoice,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let point_cast = choice
        .source
        .spell_id(
            members.get(member).unwrap().2,
            &magic.context.inventory,
            &magic.context.content.items,
        )
        .is_some_and(crate::game::magic::pointcasts::spell);
    if point_cast {
        commands.queue(move |world: &mut World| {
            if crate::game::magic::execute_in_world(world, member, choice).is_err() {
                let mut battle = world.query::<&mut Battle>().single_mut(world).unwrap();
                let work = battle.this_round.party.iter_mut().find(|w| w.actor == member).unwrap();
                work.action = RoundAction::Defend;
            }
        });
        return;
    }
    match member_casts(combatants, member, choice, rng, members, damage, party, magic) {
        Ok((report, flights)) => {
            battle.this_round.attacked = true;
            battle.credit_kills(&report.effects, news);
            let level = magic.context.level();
            magic.context.flight_order.launch(commands, flights, level);
            magic.context.news.write(MagicNews::Done(report));
        }
        Err(refusal) => {
            battle.this_round.party[index].action = RoundAction::Defend;
            magic.context.news.write(MagicNews::Refused { caster: member, refusal });
        }
    }
}

/// Live, active combatants with unobstructed scenery sight, shared by casts
/// and breath. Accepted aim never redirects to a replacement actor.
pub(super) fn visible_monsters(
    combatants: &[Entity],
    damage: &MonsterDamage,
    obstacles: &WorldObstacles,
    party_eye: Vec3,
) -> Vec<casting::Candidate> {
    combatants
        .iter()
        .filter_map(|&entity| {
            let (monster, at, walker, ..) = damage.monsters().get(entity).unwrap();
            if !damage.targetable(entity) {
                return None;
            }
            let feet = feet(at.translation(), walker);
            let eye = monster.eye(feet);
            obstacles.clear(party_eye, eye).then(|| casting::Candidate {
                entity,
                at: feet,
                distance: party_distance(party_eye, monster, feet),
            })
        })
        .collect()
}

/// A member casts or uses an item at their initiative.
pub(super) fn member_casts(
    combatants: &[Entity],
    member: Entity,
    choice: MagicChoice,
    rng: &mut GameRng,
    members: &mut Members,
    damage: &mut MonsterDamage,
    party: &PartyView,
    magic: &mut BattleMagic,
) -> Result<(casting::Report, Vec<casting::SpellInFlight>), casting::Refusal> {
    member_acts(combatants, (members, damage), party, magic, |situation, field, points| {
        casting::perform(member, choice, situation, points, rng, field)
    })
}

/// A member's magic or breath at their initiative, in the fight as it
/// stands now: the monsters it may aim at are in the fight, targetable,
/// and in the party's sight, with their groups (retail
/// `CharacterActionReachesTarget`).
pub(super) fn member_acts<R>(
    combatants: &[Entity],
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    act: impl FnOnce(
        &casting::Situation,
        &mut crate::game::magic::effects::Field,
        &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    ) -> Result<R, casting::Refusal>,
) -> Result<R, casting::Refusal> {
    let BattleMagic { context, points, groups, group_members, .. } = magic;
    let candidates = visible_monsters(combatants, damage, &context.visibility, party.eye);
    let group_of = |monster: Entity| -> Vec<Entity> {
        let group = groups.get(monster).ok().map(|group| group.0);
        match group.and_then(|g| group_members.get(g).ok()) {
            Some(mates) => mates.iter().collect(),
            None => vec![monster],
        }
    };
    context.resolve(party.now, members, damage, |field| {
        let situation = casting::Situation {
            context: casting::UseContext::Combat,
            party: field.party,
            party_at: party.eye,
            facing: crate::game::party::body::heading(party.yaw),
            monsters: &candidates,
            group_of: &group_of,
        };
        act(&situation, field, points)
    })
}
