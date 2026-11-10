//! Choosing whom to strike and with what (`docs/rules/attack.md` section 2),
//! and what an actor with nobody to strike does instead.

use bevy::prelude::{Entity, Query};

use super::{FighterReadOnly, FighterReadOnlyItem, choose_mode};
use crate::content::characters::Skill;
use crate::content::items::AttackMode;
use crate::game::actors::{Arm, Place, Places, reaches};
use crate::game::characters::Condition;
use crate::game::combat::approach::MoveGoal;
use crate::game::combat::monster::close::{self, MoveBudget, Moving};
use crate::game::combat::turn::{Fights, Turn};
use crate::game::inventory::WeaponSlot;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// What an actor does with its moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chosen {
    /// Strike `target` with the arm at `arm` in `mode`, from `slot`, which
    /// then has `left` attacks.
    Strike { slot: WeaponSlot, arm: usize, mode: AttackMode, target: Entity, left: u32 },
    /// Close in: go as far as the budget allows, and act again at `again`.
    Advance { goal: MoveGoal, budget: MoveBudget, again: u32 },
    /// Hold back warily: done for the round.
    Wait,
    /// Done for the round.
    Stand,
}

/// What choosing came to, and the members a wild attacker's eye missed who
/// may learn Stealth from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub chosen: Chosen,
    pub learns_stealth: Vec<Entity>,
}

/// The swings of an attack, and whether a readied Lightning Strike is used up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swings {
    pub count: u32,
    pub lightning_spent: bool,
}

/// How many swings an attack makes: `below(swings) + 1`, the weapon's extra
/// swings added (they may be negative), at least 1; with a sword in the main
/// slot, a Samurai's Lightning Strike readied this round (by their Samurai
/// level), and not slowed, 3 more if `below(11)` plus their level is under
/// 20, otherwise 4 (retail `BeginCombatExecution`). The readied Strike is
/// used up by the first attack of the main slot, sword or not.
pub fn swing_count(rng: &mut GameRng, arm: &Arm, lightning: Option<u8>, slowed: bool) -> Swings {
    let mut count = (rng.below(arm.swings) as i32 + 1 + arm.extra_swings).max(1) as u32;
    let readied = lightning.filter(|_| arm.slot == WeaponSlot::Primary);
    if let Some(level) = readied
        && arm.sword
        && !slowed
    {
        count += if rng.below(11) + u32::from(level) < 20 { 3 } else { 4 };
    }
    Swings { count, lightning_spent: readied.is_some() }
}

/// Whether someone on `side` of the party is "in reach of the party": armed,
/// and on the rear side, or on a side nobody holds while the center rank is
/// held.
fn in_reach_of_party(armed: bool, side: Row, formation: &Formation) -> bool {
    armed && (side == Row::Rear || (!formation.occupied(side) && formation.occupied(Row::Center)))
}

/// A way to strike: an arm, one of its modes, someone it reaches.
#[derive(Clone, Copy)]
struct Way {
    arm: usize,
    mode: AttackMode,
    target: usize,
}

/// Choose whom the actor strikes and how, or what it does instead, among
/// the living `combatants`.
///
/// It looks for foes, or for friends when insanity has turned it on its own
/// this round. A member under orders never strikes an NPC and needs no
/// sight of whom they strike; anyone else needs a clear line, and misses a
/// party member with chance `Stealth * 75 / 100` percent, who then learns
/// from it half the time. The slot due is used if it has attacks left and
/// an arm that reaches someone, otherwise the other.
///
/// A member under orders strikes whoever is in the lowest condition (a sound
/// one before a hurt one), then one in reach of the party, then the nearest,
/// ties keeping the fight's order (retail `CompareMonsterTargetCandidates`),
/// in the berserk mode under a Berserk order and otherwise in one of the
/// arm's modes. For anyone else every way to strike has an equal chance, and
/// so has waiting for one that may hold back (retail `ChooseMonsterTarget`,
/// `BuildMonsterActionQueue`).
pub fn choose(
    rng: &mut GameRng,
    actor: Entity,
    (turn, fights, now): (&Turn, Fights, u32),
    walker: Option<&Walker>,
    combatants: &[Entity],
    (fighters, places): (&Query<FighterReadOnly>, &Places),
    (formation, roster): (&Formation, &PartyRoster),
    obstacles: &WorldObstacles,
) -> Choice {
    let nothing = Choice { chosen: Chosen::Stand, learns_stealth: Vec::new() };
    let Ok(own) = fighters.get(actor) else { return nothing };
    let ordered = own.ordered.is_some();
    let at = places.of(actor);
    let seeks = if turn.berserk { Stance::Friendly } else { Stance::Hostile };
    // Those it looks for, and whether its eye misses each this time.
    let sought: Vec<(FighterReadOnlyItem, Place, bool)> = combatants
        .iter()
        .filter(|&&other| other != actor)
        .filter_map(|&other| fighters.get(other).ok())
        .filter(|their| {
            !their.body.vitals.is_dead()
                && hostility::between(own.body.standing(), their.body.standing()) == seeks
        })
        .map(|their| {
            let stealth = their.body.character.map_or(0, |c| c.skill(Skill::Stealth));
            let lost = !ordered
                && roster.contains(&their.body.entity)
                && rng.below(100) < stealth * 75 / 100;
            let there = places.of(their.body.entity);
            (their, there, lost)
        })
        .collect();
    let mut evaded = Vec::new();
    let mut found = None;
    for slot in [turn.due, turn.due.other()] {
        if turn.attacks[slot] == 0 {
            continue;
        }
        let mut ways = Vec::new();
        let arms = own.body.attacks.arms.iter().enumerate().filter(|(_, arm)| arm.slot == slot);
        for (arm, held) in arms {
            for (target, (their, there, lost)) in sought.iter().enumerate() {
                let other = their.body.entity;
                if (ordered && their.npc)
                    || !reaches(formation, (actor, &at), (other, there), held.reach)
                    || (!ordered && !obstacles.clear(at.eye, there.centre))
                {
                    continue;
                }
                if *lost {
                    if !evaded.contains(&other) {
                        evaded.push(other);
                    }
                    continue;
                }
                ways.extend(held.modes.iter().map(|mode| Way { arm, mode, target }));
            }
        }
        if !ways.is_empty() {
            found = Some((slot, ways));
            break;
        }
    }
    let learns_stealth: Vec<Entity> = evaded.into_iter().filter(|_| rng.below(2) == 0).collect();
    let Some((slot, ways)) = found else {
        let chosen = close_in(&own, &at, (turn, now), walker, &sought, (fighters, places), roster);
        return Choice { chosen, learns_stealth };
    };
    let picked = if ordered {
        let key = |way: &Way| {
            let (their, there, _) = &sought[way.target];
            let armed = !their.body.attacks.arms.is_empty();
            let reached = in_reach_of_party(armed, at.quadrant(there.feet), formation);
            (their.body.vitals.conditions.worst(), !reached, at.gap(there))
        };
        let best = ways
            .iter()
            .min_by(|a, b| {
                let (a, b) = (key(a), key(b));
                a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2))
            })
            .copied()
            .unwrap();
        let mode = if fights.berserk {
            AttackMode::Berserk
        } else {
            choose_mode(rng, own.body.attacks.arms[best.arm].modes)
        };
        Some(Way { mode, ..best })
    } else {
        let waits = usize::from(fights.waits);
        let pick = rng.below((ways.len() + waits) as u32) as usize;
        (pick >= waits).then(|| ways[pick - waits])
    };
    let chosen = match picked {
        Some(Way { arm, mode, target }) => Chosen::Strike {
            slot,
            arm,
            mode,
            target: sought[target].0.body.entity,
            left: turn.attacks[slot] - 1,
        },
        None => Chosen::Wait,
    };
    Choice { chosen, learns_stealth }
}

/// What an actor with nobody to strike does: one that walks and has not
/// moved this round closes in, on the party if it is hostile to it, on the
/// nearest of those it looks for if it stands with the party, and acts
/// again at twice the moment now, no later than 100 (retail
/// `SetUpMonsterTurn`). Hurt, it charges. Anything else stands.
fn close_in(
    own: &FighterReadOnlyItem,
    at: &Place,
    (turn, now): (&Turn, u32),
    walker: Option<&Walker>,
    sought: &[(FighterReadOnlyItem, Place, bool)],
    (fighters, places): (&Query<FighterReadOnly>, &Places),
    roster: &PartyRoster,
) -> Chosen {
    let (attacks, vitals) = (own.body.attacks, own.body.vitals);
    let Some((stride, walker)) = attacks.stride.zip(walker).filter(|_| !turn.moved) else {
        return Chosen::Stand;
    };
    let hurt = close::charges(vitals.health.current, vitals.health.max);
    let moving = Moving {
        walks: !vitals.is_dead() && walker.speed > 0.0,
        reach: attacks.reach(),
        stride,
        hasted: attacks.hasted,
        slowed: own.has(Condition::Slowed),
    };
    let going = match own.body.stance() {
        Stance::Friendly => sought
            .iter()
            .min_by(|a, b| {
                at.feet.distance_squared(a.1.feet).total_cmp(&at.feet.distance_squared(b.1.feet))
            })
            .filter(|_| walker.speed > 0.0)
            .map(|(foe, ..)| {
                let budget = MoveBudget { metres: close::metres(&moving, hurt), charge: hurt };
                (MoveGoal::Attack(foe.body.entity), budget)
            }),
        Stance::Hostile => roster
            .iter()
            .copied()
            .find(|&member| fighters.contains(member))
            .and_then(|member| close::budget(&moving, hurt, at.gap(&places.of(member))))
            .map(|budget| (MoveGoal::Party, budget)),
        Stance::Neutral => None,
    };
    let again = (now * 2).min(100);
    going.map_or(Chosen::Stand, |(goal, budget)| Chosen::Advance { goal, budget, again })
}

#[cfg(test)]
mod tests {
    use super::super::Fighter;
    use super::super::arena::{arena, arm, monster, party};
    use super::*;
    use crate::content::species::Reach;
    use crate::game::actors::Attacks;
    use crate::game::characters::{Lasting, Vitals};
    use crate::game::combat::turn::{Action, Ordered};
    use crate::game::npcs::NpcBody;
    use bevy::ecs::system::SystemState;
    use bevy::prelude::{Single, Vec3, World};
    use enum_map::EnumMap;

    /// A turn with two attacks left in each slot, the main one due.
    fn turn() -> Turn {
        Turn {
            when: Some(1),
            attacks: EnumMap::from_fn(|_| 2),
            due: WeaponSlot::Primary,
            weapons: EnumMap::default(),
            lightning: None,
            moved: false,
            special: false,
            berserk: false,
            waited: false,
        }
    }

    /// What the actor chooses at moment 30 among everyone in the arena.
    fn pick(world: &mut World, actor: Entity, turn: &Turn) -> Chosen {
        let mut everyone = world.query_filtered::<Entity, bevy::prelude::With<Vitals>>();
        let combatants: Vec<Entity> = everyone.iter(world).collect();
        let fights = Fights { waits: false, berserk: false };
        let mut state = SystemState::<(
            Query<Fighter>,
            Places,
            Single<(&Formation, &PartyRoster)>,
            WorldObstacles,
            Query<&Walker>,
        )>::new(world);
        let (fighters, places, party, obstacles, walkers) = state.get_mut(world).unwrap();
        choose(
            &mut GameRng::seeded(0),
            actor,
            (turn, fights, 30),
            walkers.get(actor).ok(),
            &combatants,
            (&fighters.as_readonly(), &places),
            *party,
            &obstacles,
        )
        .chosen
    }

    fn target(chosen: Chosen) -> Entity {
        let Chosen::Strike { target, .. } = chosen else { panic!("{chosen:?}") };
        target
    }

    /// A party of `members` under orders to attack, each with an arm that
    /// reaches everything; the first is returned.
    fn ordered_party(world: &mut World, members: usize) -> Entity {
        let (_, members) = party(world, members);
        for &member in &members {
            world.entity_mut(member).insert(Ordered(Action::Attack { berserk: false }));
            world.get_mut::<Attacks>(member).unwrap().arms[0].reach = Reach::Extreme;
        }
        members[0]
    }

    const AHEAD: Vec3 = Vec3::new(0.0, 0.0, -3.0);
    const BEHIND: Vec3 = Vec3::new(0.0, 0.0, 3.0);
    const LEFT: Vec3 = Vec3::new(-3.0, 0.0, 0.0);

    fn afflict(world: &mut World, actor: Entity, condition: Condition) {
        world.get_mut::<Vitals>(actor).unwrap().afflict(condition, Lasting::Rounds(9));
    }

    #[test]
    fn a_lightning_strike_needs_a_sword_the_main_slot_and_no_slowing_but_is_used_up_in_the_main_slot()
     {
        let rng = &mut GameRng::seeded(0);
        let mut sword = arm(80);
        sword.sword = true;
        let none = Swings { count: 1, lightning_spent: false };
        assert_eq!(swing_count(rng, &sword, None, false), none);
        // Level 5: below(11) + 5 is under 20, three more.
        assert_eq!(swing_count(rng, &sword, Some(5), false).count, 4);
        // Without a sword or slowed: no swings added, still used up.
        let spent = Swings { count: 1, lightning_spent: true };
        assert_eq!(swing_count(rng, &sword, Some(5), true), spent);
        assert_eq!(swing_count(rng, &arm(80), Some(5), false), spent);
        // The off hand's attack neither uses it nor gains.
        sword.slot = WeaponSlot::Secondary;
        assert_eq!(swing_count(rng, &sword, Some(5), false), none);
    }

    #[test]
    fn a_body_looks_for_foes_or_for_friends_when_it_has_turned_on_its_own() {
        let mut world = arena();
        let member = ordered_party(&mut world, 1);
        let friend = monster(&mut world, AHEAD, Stance::Friendly);
        let neutral = monster(&mut world, AHEAD, Stance::Neutral);
        // A neutral is nobody's, and a friend no foe.
        assert_eq!(pick(&mut world, member, &turn()), Chosen::Stand);
        let foe = monster(&mut world, AHEAD, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), foe);
        let turned = Turn { berserk: true, ..turn() };
        assert_eq!(target(pick(&mut world, member, &turned)), friend);
        world.despawn(friend);
        world.despawn(foe);
        assert_eq!(pick(&mut world, member, &turned), Chosen::Stand, "{neutral:?} is no friend");
    }

    #[test]
    fn a_member_under_orders_strikes_the_soundest_foe_first() {
        let mut world = arena();
        let member = ordered_party(&mut world, 1);
        // No condition counts lowest, then the milder: Poisoned comes before
        // Asleep in the order of conditions.
        let asleep = monster(&mut world, AHEAD, Stance::Hostile);
        let poisoned = monster(&mut world, AHEAD, Stance::Hostile);
        afflict(&mut world, asleep, Condition::Asleep);
        afflict(&mut world, poisoned, Condition::Poisoned);
        assert_eq!(target(pick(&mut world, member, &turn())), poisoned);
        let sound = monster(&mut world, AHEAD, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), sound);
    }

    #[test]
    fn then_one_in_reach_of_the_party_and_then_the_nearer() {
        // Four members hold the front and the center. A foe ahead is not in
        // reach of the party; one behind is, and so is one on a side nobody
        // holds; of those the nearer is struck.
        let mut world = arena();
        let member = ordered_party(&mut world, 4);
        let _ahead = monster(&mut world, AHEAD, Stance::Hostile);
        let behind = monster(&mut world, BEHIND * 3.0, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), behind);
        let left = monster(&mut world, LEFT, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), left);
        // One with no arm is not in reach of the party, however near.
        world.get_mut::<Attacks>(left).unwrap().arms.clear();
        assert_eq!(target(pick(&mut world, member, &turn())), behind);
        // With nobody in the center, a side is not in reach, only the rear.
        let mut world = arena();
        let member = ordered_party(&mut world, 3);
        let behind = monster(&mut world, BEHIND * 3.0, Stance::Hostile);
        let left = monster(&mut world, LEFT, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), behind);
        // Nobody in reach of the party: the nearest.
        world.despawn(behind);
        let _far = monster(&mut world, AHEAD * 3.0, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), left);
    }

    #[test]
    fn a_member_under_orders_never_strikes_an_npc() {
        let mut world = arena();
        let member = ordered_party(&mut world, 1);
        let record = world.spawn_empty().id();
        let npc = monster(&mut world, AHEAD, Stance::Hostile);
        world.entity_mut(npc).insert(NpcBody(record));
        assert_eq!(pick(&mut world, member, &turn()), Chosen::Stand);
        let foe = monster(&mut world, AHEAD, Stance::Hostile);
        assert_eq!(target(pick(&mut world, member, &turn())), foe);
    }

    #[test]
    fn the_slot_due_is_used_if_it_has_a_candidate_and_attacks_left() {
        let mut world = arena();
        let member = ordered_party(&mut world, 1);
        let mut fist = arm(80);
        (fist.slot, fist.modes, fist.reach) =
            (WeaponSlot::Secondary, AttackMode::Punch.into(), Reach::Extreme);
        world.get_mut::<Attacks>(member).unwrap().arms.push(fist);
        monster(&mut world, AHEAD, Stance::Hostile);
        let mut due = Turn { due: WeaponSlot::Secondary, ..turn() };
        due.attacks[WeaponSlot::Primary] = 3;
        // The slot due, with the attacks left in it one fewer.
        assert!(matches!(
            pick(&mut world, member, &due),
            Chosen::Strike { slot: WeaponSlot::Secondary, left: 1, mode: AttackMode::Punch, .. }
        ));
        // Out of attacks in it: the other, the same way.
        due.attacks[WeaponSlot::Secondary] = 0;
        assert!(matches!(
            pick(&mut world, member, &due),
            Chosen::Strike { slot: WeaponSlot::Primary, left: 2, .. }
        ));
    }

    /// A monster that walks, six metres a move.
    fn walker(world: &mut World, at: Vec3, stance: Stance) -> Entity {
        let walker = monster(world, at, stance);
        world.entity_mut(walker).insert(Walker { speed: 1.0, float_height: 0.0 });
        world.get_mut::<Attacks>(walker).unwrap().stride = Some(6.0);
        walker
    }

    #[test]
    fn a_hostile_body_with_nobody_to_strike_closes_in_on_the_party() {
        let mut world = arena();
        let member = party(&mut world, 1).1[0];
        let hostile = walker(&mut world, AHEAD * 4.0, Stance::Hostile);
        // Twice the moment now, 30, is 60.
        let budget = MoveBudget { metres: 6.0, charge: false };
        assert_eq!(
            pick(&mut world, hostile, &turn()),
            Chosen::Advance { goal: MoveGoal::Party, budget, again: 60 }
        );
        // It has moved this round already: it stands.
        assert_eq!(pick(&mut world, hostile, &Turn { moved: true, ..turn() }), Chosen::Stand);
        // Hurt, it charges half as far again.
        world.get_mut::<Vitals>(hostile).unwrap().health.current = 50;
        let budget = MoveBudget { metres: 9.0, charge: true };
        assert_eq!(
            pick(&mut world, hostile, &turn()),
            Chosen::Advance { goal: MoveGoal::Party, budget, again: 60 }
        );
        // Beside a party with nobody left to strike, it stands.
        world.get_mut::<Vitals>(member).unwrap().slay();
        let beside = walker(&mut world, AHEAD * 0.5, Stance::Hostile);
        assert_eq!(pick(&mut world, beside, &turn()), Chosen::Stand);
    }

    #[test]
    fn one_who_stands_with_the_party_goes_toward_the_nearest_foe() {
        let mut world = arena();
        party(&mut world, 1);
        let ally = walker(&mut world, AHEAD, Stance::Friendly);
        // Nobody to go to: it stands.
        assert_eq!(pick(&mut world, ally, &turn()), Chosen::Stand);
        let _far = monster(&mut world, AHEAD * 9.0, Stance::Hostile);
        let near = monster(&mut world, AHEAD * 4.0, Stance::Hostile);
        let budget = MoveBudget { metres: 6.0, charge: false };
        assert_eq!(
            pick(&mut world, ally, &turn()),
            Chosen::Advance { goal: MoveGoal::Attack(near), budget, again: 60 }
        );
    }
}
