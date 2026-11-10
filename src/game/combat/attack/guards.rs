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
                guard.body.vitals.can_guard()
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

#[cfg(test)]
mod tests {
    use super::super::arena::{arena, duel, field, monster};
    use super::*;
    use crate::content::species::Reach;
    use crate::game::actors::Attacks;
    use crate::game::characters::{Condition, Lasting, Vitals};
    use crate::game::combat::turn::{Guarding, Interceptions};
    use crate::game::world::hostility::Stance;
    use bevy::prelude::{Vec3, World};

    fn press(world: &mut World, fighter: Entity, press: i32) {
        world.get_mut::<Attacks>(fighter).unwrap().press = press;
    }

    #[test]
    fn the_chance_is_the_guards_press_less_the_attackers_plus_50_held_to_5_to_95() {
        let mut world = arena();
        let guard = monster(&mut world, Vec3::ZERO, Stance::Hostile);
        let attacker = monster(&mut world, Vec3::NEG_Z, Stance::Friendly);
        press(&mut world, attacker, 100);
        let mut chance = |pressing| {
            press(&mut world, guard, pressing);
            duel(&mut world, (guard, attacker), |guard, attacker| {
                stepping_chance(guard, attacker, Surprise::default())
            })
        };
        assert_eq!([100, 120, 80].map(&mut chance), [50, 70, 30]);
        // Held: 200 - 100 + 50 = 150 -> 95; 0 - 100 + 50 = -50 -> 5.
        assert_eq!([200, 0].map(&mut chance), [95, 5]);
        // The edge: 145 - 100 + 50 = 95 exactly, and above it still 95.
        assert_eq!([145, 146].map(&mut chance), [95, 95]);
    }

    /// A ward at the origin, its attacker, and a guard `gap` metres from the
    /// ward's edge that is as likely as can be to step in.
    fn guarded(gap: f32) -> (World, [Entity; 3]) {
        let mut world = arena();
        let ward = monster(&mut world, Vec3::ZERO, Stance::Hostile);
        let attacker = monster(&mut world, Vec3::NEG_Z, Stance::Friendly);
        let guard = monster(&mut world, Vec3::X * (gap + 1.0), Stance::Hostile);
        world.entity_mut(guard).insert((Guarding(ward), Interceptions { taken: 0, allowed: 2 }));
        press(&mut world, guard, 1000);
        (world, [ward, attacker, guard])
    }

    /// How often the guard steps in over 300 draws.
    fn steps(world: &mut World, [ward, attacker, _]: [Entity; 3]) -> usize {
        field(world, |fighters, places| {
            let attacker = fighters.get(attacker).unwrap();
            (0..300)
                .filter(|&seed| {
                    let rng = &mut GameRng::seeded(seed);
                    steps_in(rng, ward, &attacker, Surprise::default(), fighters, places).is_some()
                })
                .count()
        })
    }

    #[test]
    fn a_guard_must_be_able_in_reach_of_its_ward_and_have_interceptions_left() {
        // Its first arm's reach in metres at least the gap: touch is 2.
        let (mut world, all) = guarded(2.0);
        let guard = all[2];
        assert!(steps(&mut world, all) > 200);
        world.get_mut::<Interceptions>(guard).unwrap().taken = 1;
        assert!(steps(&mut world, all) > 200);
        world.get_mut::<Interceptions>(guard).unwrap().taken = 2;
        assert_eq!(steps(&mut world, all), 0, "none left");
        world.get_mut::<Interceptions>(guard).unwrap().taken = 0;
        world.get_mut::<Vitals>(guard).unwrap().afflict(Condition::Blind, Lasting::Rounds(9));
        assert_eq!(steps(&mut world, all), 0, "blind");
        let (mut world, all) = guarded(2.5);
        assert_eq!(steps(&mut world, all), 0, "out of reach");
        world.get_mut::<Attacks>(all[2]).unwrap().arms[0].reach = Reach::Long;
        assert!(steps(&mut world, all) > 200);
        world.get_mut::<Attacks>(all[2]).unwrap().arms.clear();
        assert_eq!(steps(&mut world, all), 0, "unarmed");
    }
}
