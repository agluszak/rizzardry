//! Striking, and closing in to (`docs/rules/monsters.md` sections 4 and 6):
//! whether a monster's arms reach the party, whether it would close in rather
//! than cast or breathe, whether it can strike anyone at all, and how far it
//! moves.

use bevy::prelude::Vec3;

use super::sight::MindItem;
use crate::content::species::Reach;
use crate::game::actors::Attacks;
use crate::game::characters::Condition;
use crate::game::hits::effects::BodyReadOnlyItem;

/// Metres per unit of a species' `combat_move_range` (retail `SetUpMonsterTurn`).
pub const METRES_PER_MOVE_RANGE: f32 = 20.0;

/// Stop this far inside an attack's reach (retail `Combat.cpp:2486-2551`).
const ARRIVAL_MARGIN: f32 = 0.3;

/// Whether one of its arms reaches the party, `gap` metres off.
pub fn reaches_party(attacks: &Attacks, gap: f32) -> bool {
    attacks.arms.iter().any(|arm| gap <= arm.reach.metres())
}

/// Whether it would close in to strike rather than consider a special attack
/// or a spell: its kind prefers blows, it has some, and none reaches the
/// party.
pub fn closes_in(own: &BodyReadOnlyItem, mind: &MindItem, gap: f32) -> bool {
    !mind.temper.0.prefers_range && !own.attacks.arms.is_empty() && !reaches_party(own.attacks, gap)
}

/// Whether it can strike nobody and withdraws instead (retail
/// `UpdateMonsterAI`'s `RETURN_TO_START`): it has no arm, or it is blind,
/// does not fight blind, and has nobody in reach.
pub fn cannot_strike(own: &BodyReadOnlyItem, gap: f32) -> bool {
    let blind = own.vitals.conditions.has(Condition::Blind) && own.attacks.blind_fighting.is_none();
    own.attacks.arms.is_empty() || (blind && !reaches_party(own.attacks, gap))
}

/// Hurt enough to charge rather than walk up: at 95 percent of its health or
/// less (retail `SetUpMonsterTurn`).
pub fn charges(health: u32, most: u32) -> bool {
    health * 100 <= most * 95
}

/// How far a monster moves in one action: its stride, half again when it
/// charges, plus the share of it Haste adds, or halved when slowed (retail
/// `SetUpMonsterTurn`, `Combat.cpp:2855-2909`).
pub fn metres(attacks: &Attacks, slowed: bool, charge: bool) -> f32 {
    let stride = attacks.stride.unwrap_or(0.0);
    let budget = if charge { stride * 1.5 } else { stride };
    if attacks.hasted > 0.0 {
        budget + stride * attacks.hasted
    } else if slowed {
        budget * 0.5
    } else {
        budget
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

/// A monster's move this round: how far it may go, and whether it charges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveBudget {
    pub metres: f32,
    pub charge: bool,
}

impl MoveBudget {
    /// What a monster may move now: it charges when hurt.
    pub fn of(own: &BodyReadOnlyItem) -> Self {
        let charge = charges(own.vitals.health.current, own.vitals.health.max);
        let slowed = own.vitals.conditions.has(Condition::Slowed);
        MoveBudget { metres: metres(own.attacks, slowed, charge), charge }
    }
}

/// Where a monster running from the party heads: straight away from the
/// party's eye over the ground, as far as its move takes it.
pub fn withdraws_to(feet: Vec3, party_eye: Vec3, metres: f32) -> Vec3 {
    let away = (feet - party_eye).with_y(0.0).normalize_or(Vec3::X);
    feet + away * metres
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monster_charges_at_95_percent_of_its_health_or_less() {
        assert!(charges(95, 100) && !charges(96, 100));
        // 19 of 20 is exactly 95 percent.
        assert!(charges(19, 20) && !charges(20, 20));
    }

    #[test]
    fn haste_adds_a_share_of_the_stride_and_slowing_halves_the_move() {
        let stride = Attacks { stride: Some(40.0), ..Default::default() };
        assert_eq!((metres(&stride, false, false), metres(&stride, false, true)), (40.0, 60.0));
        assert_eq!((metres(&stride, true, false), metres(&stride, true, true)), (20.0, 30.0));
        // Haste 0.3 (power 3): 40 + 40 * 0.3 = 52; charging, 60 + 12 = 72. It
        // wins over slowing.
        let hasted = Attacks { hasted: 0.3, ..stride };
        assert!((metres(&hasted, true, false) - 52.0).abs() < 0.001);
        assert!((metres(&hasted, false, true) - 72.0).abs() < 0.001);
    }
}
