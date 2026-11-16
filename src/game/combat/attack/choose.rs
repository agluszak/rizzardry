//! Choosing whom to strike and with what (`docs/rules/attack.md` section 2),
//! and what an actor with nobody to strike does instead.

use bevy::prelude::{Entity, Query};

use super::{FighterReadOnly, FighterReadOnlyItem, choose_mode};
use crate::content::characters::Skill;
use crate::content::items::AttackMode;
use crate::game::actors::{Arm, Place, Places, reaches};
use crate::game::combat::approach::MoveGoal;
use crate::game::combat::monster::close::{self, MoveBudget};
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
            let stealth = their.body.skills[Skill::Stealth];
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
            (their.body.conditions.worst(), !reached, at.gap(there))
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
    let Some(walker) = walker.filter(|_| attacks.stride.is_some() && !turn.moved) else {
        return Chosen::Stand;
    };
    let walks = !vitals.is_dead() && walker.speed > 0.0;
    let going = match own.body.stance() {
        Stance::Friendly => sought
            .iter()
            .min_by(|a, b| {
                at.feet.distance_squared(a.1.feet).total_cmp(&at.feet.distance_squared(b.1.feet))
            })
            .filter(|_| walker.speed > 0.0)
            .map(|(foe, ..)| MoveGoal::Attack(foe.body.entity)),
        Stance::Hostile => roster
            .iter()
            .copied()
            .find(|&member| fighters.contains(member))
            .filter(|&member| {
                close::needs_approach(walks, attacks.reach(), at.gap(&places.of(member)))
            })
            .map(|_| MoveGoal::Party),
        Stance::Neutral => None,
    };
    let budget = MoveBudget::of(&own.body);
    let again = (now * 2).min(100);
    going.map_or(Chosen::Stand, |goal| Chosen::Advance { goal, budget, again })
}
