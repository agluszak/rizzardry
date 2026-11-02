//! A member's accepted turn: blows, a breath or a cast.

use super::*;

/// A member's accepted turn comes: revalidated, then their spell or attack.
/// Returns whether they acted; an actor who can no longer act loses the turn.
pub(super) fn party_turn(
    commands: &mut Commands,
    battle: &mut Battle,
    member: Entity,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) -> bool {
    let work = *magic.party_turns.get(member).unwrap();
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
        magic.party_turns.get_mut(member).unwrap().end();
        return false;
    }
    match work.action {
        RoundAction::Breathe(target) => {
            magic.party_turns.get_mut(member).unwrap().end();
            breathe(
                commands,
                battle,
                member,
                target,
                combatants,
                (members, damage),
                party,
                magic,
                news,
            );
        }
        RoundAction::Magic(choice) => {
            magic.party_turns.get_mut(member).unwrap().end();
            cast(commands, battle, member, choice, combatants, rng, (members, damage), party, magic);
        }
        // The attack is the attack systems' to make.
        RoundAction::Attack { .. } => return false,
        // Defending takes no turn.
        RoundAction::Defend => {
            magic.party_turns.get_mut(member).unwrap().end();
            return false;
        }
    }
    true
}

/// A member breathes at their chosen target; a breath that cannot be made
/// falls back to defending.
pub(super) fn breathe(
    commands: &mut Commands,
    battle: &mut Battle,
    member: Entity,
    target: Entity,
    combatants: &[Entity],
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let result = member_acts(combatants, (members, damage), party, magic, |situation, field, _| {
        breath::execute(commands, member, target, situation, field)
    });
    match result {
        Ok(()) => {
            battle.attacked(true);
            news.write(CombatNews::Breathed { actor: member });
        }
        Err(_) => magic.party_turns.get_mut(member).unwrap().action = RoundAction::Defend,
    }
}

/// A member casts or uses an item. A refused action falls back to
/// defending (retail `ExecuteCharacterAction`), its turn spent; a point
/// cast needs the whole world and settles once this step is done.
pub(super) fn cast(
    commands: &mut Commands,
    battle: &mut Battle,
    member: Entity,
    choice: MagicChoice,
    combatants: &[Entity],
    rng: &mut GameRng,
    (members, damage): (&mut Members, &mut MonsterDamage),
    party: &PartyView,
    magic: &mut BattleMagic,
) {
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
                world.get_mut::<PartyTurn>(member).unwrap().action = RoundAction::Defend;
            }
        });
        return;
    }
    match member_casts(commands, combatants, member, choice, rng, members, damage, party, magic) {
        Ok(flights) => {
            battle.attacked(true);
            let level = magic.context.level();
            magic.context.flight_order.launch(commands, flights, level);
        }
        Err(refusal) => {
            magic.party_turns.get_mut(member).unwrap().action = RoundAction::Defend;
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
    commands: &mut Commands,
    combatants: &[Entity],
    member: Entity,
    choice: MagicChoice,
    rng: &mut GameRng,
    members: &mut Members,
    damage: &mut MonsterDamage,
    party: &PartyView,
    magic: &mut BattleMagic,
) -> Result<Vec<casting::SpellInFlight>, casting::Refusal> {
    member_acts(combatants, (members, damage), party, magic, |situation, field, points| {
        casting::perform(commands, member, choice, situation, points, rng, field)
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
