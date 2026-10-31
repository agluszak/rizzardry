//! A member's accepted turn: blows, a breath or a cast.

use super::*;

/// The party's armour as this round's blows meet it: the spells on each
/// member and the party's, and whether they defend (retail
/// `CalcArmorClasses`).
pub(super) fn party_armours(battle: &Battle, field: &Field) -> Vec<(Entity, Armour)> {
    let items = field.items();
    field
        .party
        .iter()
        .map(|member| {
            let (character, vitals, carried, burden) = field.members.get(member).unwrap();
            let armour = attack::enchanted_armour(
                &character.stats(&vitals.enchantments),
                carried,
                items,
                burden.load,
                battle.defends(member),
                (&vitals.enchantments, Some(&field.enchantments.0)),
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
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let work = battle.this_round.party[index];
    let member = work.actor;
    let able_now = magic.field.party.contains(&member)
        && magic.field.members.get(member).is_ok_and(|(_, vitals, carried, ..)| {
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
            breathe(battle, index, target, combatants, rng, party, magic, news);
        }
        RoundAction::Magic(choice) => {
            battle.this_round.party[index].end();
            cast(commands, battle, index, choice, combatants, rng, party, magic, news);
        }
        RoundAction::Attack { .. } => {
            let BattleMagic { field, points, moving, .. } = magic;
            let mut fight = blows::Fight {
                combatants,
                party,
                armours: party_armours(battle, field),
                moving,
                points,
                attacks: &|_| false,
                waits: false,
            };
            field.improved.clear();
            blows::member_attacks(battle, index, rng, field, &mut fight, news);
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
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let result = member_acts(combatants, party, magic, |situation, field, points| {
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
/// cast is carried out by its own system.
pub(super) fn cast(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    choice: MagicChoice,
    combatants: &[Entity],
    rng: &mut GameRng,
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let field = &magic.field;
    let point_cast = choice
        .source
        .spell_id(field.members.get(member).unwrap().2, &field.inventory, field.items())
        .filter(|&id| crate::game::magic::pointcasts::spell(id));
    if let Some(spell) = point_cast {
        commands.write_message(crate::game::magic::utilities::UtilityCast {
            caster: member,
            choice,
            spell,
        });
        return;
    }
    match member_casts(combatants, member, choice, rng, party, magic) {
        Ok((report, flights)) => {
            battle.this_round.attacked = true;
            battle.credit_kills(&report.effects, news);
            let field = &mut magic.field;
            let level = field.level();
            field.flight_order.launch(commands, flights, level);
            field.news.write(MagicNews::Done(report));
        }
        Err(refusal) => {
            battle.this_round.party[index].action = RoundAction::Defend;
            magic.field.news.write(MagicNews::Refused { caster: member, refusal });
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
    party: &PartyView,
    magic: &mut BattleMagic,
) -> Result<(casting::Report, Vec<casting::SpellInFlight>), casting::Refusal> {
    member_acts(combatants, party, magic, |situation, field, points| {
        casting::perform(member, choice, situation, points, rng, field)
    })
}

/// A member's magic or breath at their initiative, in the fight as it
/// stands now: the monsters it may aim at are in the fight, targetable,
/// and in the party's sight, with their groups (retail
/// `CharacterActionReachesTarget`).
pub(super) fn member_acts<R>(
    combatants: &[Entity],
    party: &PartyView,
    magic: &mut BattleMagic,
    act: impl FnOnce(
        &casting::Situation,
        &mut crate::game::magic::effects::Field,
        &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    ) -> Result<R, casting::Refusal>,
) -> Result<R, casting::Refusal> {
    let BattleMagic { field, points, groups, group_members, .. } = magic;
    let candidates = visible_monsters(combatants, &field.monsters, &field.visibility, party.eye);
    let group_of = |monster: Entity| -> Vec<Entity> {
        let group = groups.get(monster).ok().map(|group| group.0);
        match group.and_then(|g| group_members.get(g).ok()) {
            Some(mates) => mates.iter().collect(),
            None => vec![monster],
        }
    };
    let roster: Vec<Entity> = field.party.iter().collect();
    let situation = casting::Situation {
        context: casting::UseContext::Combat,
        party: &roster,
        party_at: party.eye,
        facing: crate::game::party::body::heading(party.yaw),
        monsters: &candidates,
        group_of: &group_of,
    };
    field.improved.clear();
    act(&situation, field, points)
}
