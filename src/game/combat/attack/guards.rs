//! Who steps in to take a blow meant for another (`docs/rules/attack.md`
//! section 4, "Guards intercepting"; retail `ResolveGuardianInterception`).

use bevy::prelude::{Entity, Query};

use super::{FighterReadOnly, FighterReadOnlyItem, Surprise};
use crate::game::actors::Places;
use crate::game::rng::GameRng;

/// How well a fighter presses a blow past a guard, or guards against one:
/// its press less its fatigue, which tells half as much on one who tires
/// little.
fn pressing(fighter: &FighterReadOnlyItem) -> i32 {
    let attacks = fighter.body.attacks;
    let fatigue = fighter.fatigue();
    attacks.press - if attacks.tires_little { fatigue / 2 } else { fatigue }
}

/// The chance a guard steps in, in percent: its press against the attacker's,
/// the attacker's raised by 10 for an unaware blow and 10 for a natural one,
/// `guard - attacker + 50` held to 5 to 95.
pub fn stepping_chance(
    guard: &FighterReadOnlyItem,
    attacker: &FighterReadOnlyItem,
    surprise: Surprise,
) -> u32 {
    let pressed =
        pressing(attacker) + 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
    (pressing(guard) - pressed + 50).clamp(5, 95) as u32
}

/// The guard that takes a blow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stepping {
    pub guard: Entity,
    /// The stamina it loses to it.
    pub tires: u32,
}

/// Which of a ward's guards steps in for a swing that has reached it: those
/// able (alive, and nothing as bad as Blind), with the ward within reach of
/// their first arm and interceptions left this round each step in with their
/// chance, and one of those at random takes the blow, tired `below(3) + 2`
/// by it.
pub fn steps_in(
    rng: &mut GameRng,
    ward: Entity,
    attacker: &FighterReadOnlyItem,
    surprise: Surprise,
    fighters: &Query<FighterReadOnly>,
    places: &Places,
) -> Option<Stepping> {
    let at = places.of(ward);
    let guards = fighters.get(ward).ok()?.guards?;
    let stepping: Vec<Entity> = guards
        .entities()
        .iter()
        .copied()
        .filter(|&guard| {
            fighters.get(guard).is_ok_and(|guard| {
                guard.body.can_guard
                    && guard.body.attacks.arms.first().is_some_and(|arm| {
                        places.of(guard.body.entity).gap(&at) <= arm.reach.metres()
                    })
                    && guard.interceptions.is_some_and(|made| made.taken < made.allowed)
                    && rng.below(100) < stepping_chance(&guard, attacker, surprise)
            })
        })
        .collect();
    if stepping.is_empty() {
        return None;
    }
    let guard = stepping[rng.below(stepping.len() as u32) as usize];
    Some(Stepping { guard, tires: rng.below(3) + 2 })
}
