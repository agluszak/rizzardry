//! Moving in a fight (`docs/rules/monsters.md` section 6): how far one moves
//! in an action, whether it charges, where a flight heads and when a walk to
//! strike has arrived.

use bevy::prelude::*;

use crate::content::species::Reach;
use crate::game::actors::{Modifiers, Nature};
use crate::game::characters::{Condition, Conditions, Vitals};

/// Metres per unit of a species' `combat_move_range` (retail `SetUpMonsterTurn`).
pub const METRES_PER_MOVE_RANGE: f32 = 20.0;

/// Stop this far inside an attack's reach (retail `Combat.cpp:2486-2551`).
const ARRIVAL_MARGIN: f32 = 0.3;

/// How far it moves in one action in a fight (retail `SetUpMonsterTurn`,
/// `Combat.cpp:2855-2909`): its stride walking, half again at a charge, plus
/// the share of its stride Haste adds, or halved when slowed. Only one that
/// strides has it.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub walking: f32,
    pub charging: f32,
}

/// Hurt enough to charge rather than walk up: at 95 percent of its health
/// or less (retail `SetUpMonsterTurn`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Charges;

/// How far it moves follows its stride, Haste and being slowed.
pub fn moves(
    actors: Query<(Entity, &Nature, &Modifiers, &Conditions, Option<&Move>)>,
    mut commands: Commands,
) {
    for (actor, nature, modifiers, conditions, was) in &actors {
        let now = nature.stride.map(|stride| {
            let haste = modifiers.laid.stride;
            let slowed = conditions.has(Condition::Slowed);
            let by = |budget: f32| {
                if haste > 0.0 {
                    budget + stride * haste
                } else if slowed {
                    budget * 0.5
                } else {
                    budget
                }
            };
            Move { walking: by(stride), charging: by(stride * 1.5) }
        });
        match (now, was) {
            (Some(now), Some(&was)) if now == was => {}
            (Some(now), _) => {
                commands.entity(actor).insert(now);
            }
            (None, Some(_)) => {
                commands.entity(actor).remove::<Move>();
            }
            (None, None) => {}
        }
    }
}

/// Charging follows its health.
pub fn charging(actors: Query<(Entity, &Vitals, Has<Charges>)>, mut commands: Commands) {
    for (actor, vitals, was) in &actors {
        let now = vitals.health.current * 100 <= vitals.health.max * 95;
        if now && !was {
            commands.entity(actor).insert(Charges);
        } else if !now && was {
            commands.entity(actor).remove::<Charges>();
        }
    }
}

/// Whether it has reached its target: its reach less 0.3 m covers the gap.
pub fn arrived(reach: Reach, gap: f32) -> bool {
    gap <= reach.metres() - ARRIVAL_MARGIN
}

/// Whether a walker `gap` from its foe still needs to close to attack range.
pub fn needs_approach(walks: bool, reach: Reach, gap: f32) -> bool {
    walks && !arrived(reach, gap)
}

/// Where a monster running from the party heads: straight away from the
/// party's eye over the ground, as far as its move takes it.
pub fn withdraws_to(feet: Vec3, party_eye: Vec3, metres: f32) -> Vec3 {
    let away = (feet - party_eye).with_y(0.0).normalize_or(Vec3::X);
    feet + away * metres
}
