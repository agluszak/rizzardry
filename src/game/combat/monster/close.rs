//! Striking, and closing in to (`docs/rules/monsters.md` sections 4 and 6):
//! whether a monster's arms reach the party, whether it would close in rather
//! than cast or breathe, whether it can strike anyone at all, and how far it
//! moves.

use bevy::prelude::Vec3;

use crate::content::species::Reach;

/// Metres per unit of a species' `combat_move_range` (retail `SetUpMonsterTurn`).
pub const METRES_PER_MOVE_RANGE: f32 = 20.0;

/// Stop this far inside an attack's reach (retail `Combat.cpp:2486-2551`).
const ARRIVAL_MARGIN: f32 = 0.3;

/// What a monster's reach to the party weighs.
#[derive(Clone, Copy, Debug)]
pub struct Striking<'a> {
    /// The reach of each of its arms.
    pub arms: &'a [Reach],
    /// The gap to the party.
    pub party_gap: f32,
    /// Its kind would stand off and cast or breathe rather than close in.
    pub prefers_range: bool,
    /// It is Blind and does not fight blind.
    pub blind: bool,
}

/// Whether one of its arms reaches the party.
pub fn reaches_party(facts: &Striking) -> bool {
    facts.arms.iter().any(|arm| facts.party_gap <= arm.metres())
}

/// Whether it would close in to strike rather than consider a special attack
/// or a spell: it prefers blows, has some, and none reaches the party.
pub fn closes_in(facts: &Striking) -> bool {
    !facts.prefers_range && !facts.arms.is_empty() && !reaches_party(facts)
}

/// Whether it can strike nobody and withdraws instead (retail
/// `UpdateMonsterAI`'s `RETURN_TO_START`): it has no arm, or it is blind and
/// has nobody in reach.
pub fn cannot_strike(facts: &Striking) -> bool {
    facts.arms.is_empty() || (facts.blind && !reaches_party(facts))
}

/// Hurt enough to charge rather than walk up: at 95 percent of its health or
/// less (retail `SetUpMonsterTurn`).
pub fn charges(health: u32, most: u32) -> bool {
    health * 100 <= most * 95
}

/// What a monster's move weighs.
#[derive(Clone, Copy, Debug)]
pub struct Moving {
    /// It is alive and walks.
    pub walks: bool,
    /// The reach of its furthest arm.
    pub reach: Reach,
    /// Its stride, in metres: its kind's combat move range times 20 m.
    pub stride: f32,
    /// The share of its stride Haste adds, a tenth a power.
    pub hasted: f32,
    pub slowed: bool,
}

/// How far a monster moves in one action: its stride, half again when it
/// charges, plus the share Haste adds, or halved when slowed (retail
/// `SetUpMonsterTurn`, `Combat.cpp:2855-2909`).
pub fn metres(facts: &Moving, charge: bool) -> f32 {
    let budget = if charge { facts.stride * 1.5 } else { facts.stride };
    if facts.hasted > 0.0 {
        budget + facts.stride * facts.hasted
    } else if facts.slowed {
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

/// How far a monster `gap` from the party may close in, and whether it charges
/// (`hurt`), if it needs to at all.
pub fn budget(facts: &Moving, hurt: bool, gap: f32) -> Option<MoveBudget> {
    needs_approach(facts.walks, facts.reach, gap)
        .then(|| MoveBudget { metres: metres(facts, hurt), charge: hurt })
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

    const CLAWS: [Reach; 1] = [Reach::Touch];
    const CLAWS_AND_SPIT: [Reach; 2] = [Reach::Touch, Reach::Long];

    fn striking(arms: &[Reach], party_gap: f32) -> Striking<'_> {
        Striking { arms, party_gap, prefers_range: false, blind: false }
    }

    #[test]
    fn an_arm_reaches_the_party_within_its_reach_in_metres() {
        // Touch 2 m, short 4 m, long 25 m, extreme 50 m (page, intro).
        assert!(reaches_party(&striking(&CLAWS, 2.0)));
        assert!(!reaches_party(&striking(&CLAWS, 2.1)));
        // The longest arm counts: 25 m reached, 25.1 not.
        assert!(reaches_party(&striking(&CLAWS_AND_SPIT, 25.0)));
        assert!(!reaches_party(&striking(&CLAWS_AND_SPIT, 25.1)));
        assert!(reaches_party(&striking(&[Reach::Short], 4.0)));
        assert!(reaches_party(&striking(&[Reach::Extreme], 50.0)));
        assert!(!reaches_party(&striking(&[], 0.0)), "no arm reaches");
    }

    #[test]
    fn one_that_prefers_blows_with_none_in_reach_closes_in() {
        assert!(closes_in(&striking(&CLAWS, 10.0)));
        assert!(!closes_in(&striking(&CLAWS, 1.0)), "in reach it strikes");
        let ranged = Striking { prefers_range: true, ..striking(&CLAWS, 10.0) };
        assert!(!closes_in(&ranged), "one that prefers range stands off");
        assert!(!closes_in(&striking(&[], 10.0)), "with no arm there is nothing to close in with");
    }

    #[test]
    fn one_with_no_arm_or_blind_with_nobody_in_reach_cannot_strike() {
        assert!(cannot_strike(&striking(&[], 1.0)));
        assert!(!cannot_strike(&striking(&CLAWS, 10.0)), "sighted, it closes in instead");
        let blind = Striking { blind: true, ..striking(&CLAWS, 10.0) };
        assert!(cannot_strike(&blind));
        assert!(!cannot_strike(&Striking { party_gap: 2.0, ..blind }), "blind but in reach");
    }

    #[test]
    fn a_monster_charges_at_95_percent_of_its_health_or_less() {
        // 95 * 100 <= 100 * 95: 95 of 100 charges; 96 does not.
        assert!(charges(95, 100));
        assert!(!charges(96, 100));
        assert!(charges(1, 100));
        // 19 of 20 is exactly 95 percent.
        assert!(charges(19, 20));
        assert!(!charges(20, 20));
    }

    fn moving() -> Moving {
        Moving { walks: true, reach: Reach::Touch, stride: 40.0, hasted: 0.0, slowed: false }
    }

    #[test]
    fn a_move_is_the_stride_half_again_when_charging() {
        assert_eq!(metres(&moving(), false), 40.0);
        assert_eq!(metres(&moving(), true), 60.0); // 40 * 1.5
    }

    #[test]
    fn haste_adds_a_share_of_the_stride_and_slowing_halves_the_move() {
        // Haste 0.3 (power 3): 40 + 40 * 0.3 = 52; charging, 60 + 12 = 72.
        let hasted = Moving { hasted: 0.3, ..moving() };
        assert!((metres(&hasted, false) - 52.0).abs() < 0.001);
        assert!((metres(&hasted, true) - 72.0).abs() < 0.001);
        // Slowed: 40 * 0.5 = 20; charging, 60 * 0.5 = 30.
        let slowed = Moving { slowed: true, ..moving() };
        assert_eq!(metres(&slowed, false), 20.0);
        assert_eq!(metres(&slowed, true), 30.0);
        // Haste wins over slowing: the share is added, not halved.
        let both = Moving { hasted: 0.3, slowed: true, ..moving() };
        assert!((metres(&both, false) - 52.0).abs() < 0.001);
    }

    #[test]
    fn it_arrives_within_its_reach_less_three_tenths_of_a_metre() {
        // Touch 2 m less 0.3: 1.7 m.
        assert!(arrived(Reach::Touch, 1.7));
        assert!(!arrived(Reach::Touch, 1.8));
        assert!(needs_approach(true, Reach::Touch, 1.8));
        assert!(!needs_approach(true, Reach::Touch, 1.7));
        assert!(!needs_approach(false, Reach::Touch, 30.0), "what cannot walk does not approach");
    }

    #[test]
    fn a_walker_short_of_the_party_may_close_in_by_its_move() {
        let walker = moving();
        assert_eq!(budget(&walker, false, 30.0), Some(MoveBudget { metres: 40.0, charge: false }));
        assert_eq!(budget(&walker, true, 30.0), Some(MoveBudget { metres: 60.0, charge: true }));
        assert_eq!(budget(&walker, false, 1.0), None, "already there");
        assert_eq!(budget(&Moving { walks: false, ..walker }, false, 30.0), None);
    }

    #[test]
    fn a_withdrawing_monster_heads_straight_away_over_the_ground() {
        // From (3, 5, 0) with the party's eye at (0, 1.5, 0): away is +x on the
        // ground, 10 m of it.
        let to = withdraws_to(Vec3::new(3.0, 5.0, 0.0), Vec3::new(0.0, 1.5, 0.0), 10.0);
        assert!((to - Vec3::new(13.0, 5.0, 0.0)).length() < 0.001);
        // Straight over the party's eye, it goes along +x.
        let to = withdraws_to(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 1.5, 0.0), 2.0);
        assert!((to - Vec3::new(2.0, 0.0, 0.0)).length() < 0.001);
    }
}
