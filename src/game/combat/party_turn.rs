//! A member's accepted turn: blows, a breath or a cast.

use super::*;
use crate::game::magic::pointcasts;

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
            breathe(battle, index, target, combatants, party, magic, news);
        }
        RoundAction::Magic(choice) => {
            battle.this_round.party[index].end();
            cast(commands, battle, index, choice, combatants, party, magic, news);
        }
        RoundAction::Attack { .. } => {
            let BattleMagic { field, moving, .. } = magic;
            let mut fight = blows::Fight {
                combatants,
                party,
                armours: party_armours(battle, field),
                moving,
                waits: false,
            };
            blows::member_attacks(battle, index, field, &mut fight, news);
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
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let result = member_acts(combatants, party, magic, |situation, field| {
        breath::execute(member, target, situation, field)
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
/// defending (retail `ExecuteCharacterAction`), its turn spent.
pub(super) fn cast(
    commands: &mut Commands,
    battle: &mut Battle,
    index: usize,
    choice: MagicChoice,
    combatants: &[Entity],
    party: &PartyView,
    magic: &mut BattleMagic,
    news: &mut MessageWriter<CombatNews>,
) {
    let member = battle.this_round.party[index].actor;
    let field = &magic.field;
    let point_spell = choice
        .source
        .spell_id(field.members.get(member).unwrap().2, &field.inventory, field.items())
        .filter(|&id| pointcasts::spell(id));
    if let Some(spell) = point_spell {
        let BattleMagic { field, ground, .. } = magic;
        match pointcasts::point_cast(member, choice, spell, ground, field, commands) {
            Ok(report) => {
                field.news.write(MagicNews::Done(report));
            }
            Err(refusal) => {
                battle.this_round.party[index].action = RoundAction::Defend;
                field.news.write(MagicNews::Refused { caster: member, refusal });
            }
        }
        return;
    }
    let cast = member_acts(combatants, party, magic, |situation, field| {
        casting::perform(member, choice, situation, field)
    });
    match cast {
        Ok((report, flights)) => {
            battle.this_round.attacked = true;
            battle.credit_kills(&report.effects, news);
            let field = &mut magic.field;
            field.launch(commands, flights);
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
    ) -> Result<R, casting::Refusal>,
) -> Result<R, casting::Refusal> {
    let BattleMagic { field, groups, group_members, .. } = magic;
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
    act(&situation, field)
}
