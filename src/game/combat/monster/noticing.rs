//! Whether a watcher makes out someone watched (`docs/rules/monsters.md`
//! section 2): a monster watching the party, a monster watching a monster and
//! the party watching a monster are one rule. What differs is who watches,
//! how they face, and what hides the watched, and those are the facts.

use std::f32::consts::FRAC_PI_4;

/// What noticing weighs.
#[derive(Clone, Copy, Debug)]
pub struct Noticing {
    /// The watcher's eyes are open: it can take a combat action, and is not
    /// Blind unless it fights blind.
    pub open: bool,
    /// Nothing solid stands between the two pairs of eyes. Only worked out
    /// for a watcher whose eyes are open and who is within the far clip, since
    /// nothing else can come of it.
    pub line: bool,
    /// The watcher's own Senses, spells counted.
    pub senses: i32,
    /// The percent its kind (or a party member's Scouting) adds to its sight.
    pub bonus: u32,
    /// Between the way the watcher faces and the direction to the watched, in
    /// radians. The party's is 0 under Shadow Hound.
    pub angle: f32,
    /// The gap between their bodies.
    pub gap: f32,
    /// The level the watched hides by: a monster's own, the party's lowest able.
    pub hider_level: i32,
    /// What the watched adds to its hiding: Chameleon, or a species' camouflage
    /// outside a fight.
    pub concealment: u32,
    /// The level's far clip.
    pub far: f32,
    /// The party is searching, which narrows what it makes out.
    pub searching: bool,
}

/// How far a watcher sees (retail `ComputeSightThreshold`, `Sight.cpp:197-243`).
///
/// ```text
/// percent = 100                                    if angle < 45 degrees, or the gap is 0
/// percent = (senses - 2 * level) * ((far - gap) / gap), cut to a whole number, otherwise
/// percent = percent - 10 * concealment
/// percent = (bonus + 100) * percent / 100          if bonus is not 0
/// percent = clamp(percent, 2, 100)
/// threshold = percent * far / 100
/// ```
pub fn threshold(facts: &Noticing) -> f32 {
    let Noticing { angle, senses, bonus, gap, hider_level, concealment, far, .. } = *facts;
    let mut percent = if angle < FRAC_PI_4 || gap == 0.0 {
        100
    } else {
        ((senses - hider_level * 2) as f32 * ((far - gap) / gap)) as i32
    };
    percent -= concealment as i32 * 10;
    if bonus != 0 {
        percent = (bonus as i32 + 100) * percent / 100;
    }
    percent.clamp(2, 100) as f32 * far * 0.01
}

/// Whether the watcher makes out the watched: its eyes are open, the watched
/// is within the far clip, the line is clear, and the threshold (times 0.6667
/// while the party searches, retail `Sight.cpp`) reaches the gap.
pub fn notices(facts: &Noticing) -> bool {
    let searching = if facts.searching { 0.6667 } else { 1.0 };
    facts.open && facts.gap <= facts.far && facts.line && threshold(facts) * searching >= facts.gap
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    use super::*;

    /// A watcher with Senses 40 looking away (a right angle), from `gap`
    /// metres, at someone of level 5, in a level whose far clip is 100 m,
    /// with nothing else adding or taking away.
    fn looking_away(gap: f32) -> Noticing {
        Noticing {
            open: true,
            line: true,
            senses: 40,
            bonus: 0,
            angle: FRAC_PI_2,
            gap,
            hider_level: 5,
            concealment: 0,
            far: 100.0,
            searching: false,
        }
    }

    fn assert_near(got: f32, want: f32) {
        assert!((got - want).abs() < 0.001, "{got} is not {want}");
    }

    #[test]
    fn one_facing_the_watched_within_45_degrees_sees_to_the_far_clip() {
        // percent 100 whatever the Senses: threshold 100 * 100 / 100 = 100.
        let facing = |angle, gap| Noticing { angle, senses: 1, gap, ..looking_away(gap) };
        assert!(notices(&facing(0.0, 100.0)));
        assert!(notices(&facing(FRAC_PI_4 - 0.01, 99.0)));
        // At 45 degrees exactly it is not "under 45": 1 - 10 = -9 -> clamp 2,
        // threshold 2 m.
        assert!(!notices(&facing(FRAC_PI_4, 50.0)));
        assert!(notices(&facing(FRAC_PI_4, 2.0)));
    }

    #[test]
    fn the_far_clip_ends_sight_whatever_the_threshold() {
        // Facing: threshold 100, gap 100.5 is past the clip.
        let facing = Noticing { angle: 0.0, ..looking_away(100.5) };
        assert!(!notices(&facing));
        assert!(notices(&Noticing { gap: 100.0, ..facing }));
    }

    #[test]
    fn from_the_side_the_senses_less_twice_the_level_scale_with_the_gap_left() {
        // (40 - 2 * 5) * ((100 - gap) / gap) = 30 * ((100 - gap) / gap).
        // gap 20: 30 * 4 = 120 -> clamp 100, threshold 100 >= 20.
        assert!(notices(&looking_away(20.0)));
        // gap 30: 30 * (70 / 30) = 70, threshold 70 >= 30.
        assert!(notices(&looking_away(30.0)));
        // gap 50: 30 * 1 = 30, threshold 30 < 50.
        assert!(!notices(&looking_away(50.0)));
        // gap 70: 30 * (30 / 70) = 12.86 -> 12, threshold 12 < 70.
        assert!(!notices(&looking_away(70.0)));
        // gap 0 is the whole clip whichever way it faces.
        assert!(notices(&looking_away(0.0)));
        assert_near(threshold(&looking_away(30.0)), 70.0);
        assert_near(threshold(&looking_away(70.0)), 12.0);
    }

    #[test]
    fn the_level_of_the_watched_takes_twice_itself_off_the_senses() {
        // Level 15: (40 - 30) * (70 / 30) = 23.3 -> 23 at gap 30; level 5 sees 70.
        let hardy = Noticing { hider_level: 15, ..looking_away(30.0) };
        assert_near(threshold(&hardy), 23.0);
        assert!(!notices(&hardy));
        // Level 20: 40 - 40 = 0 -> percent 0 -> clamp 2 -> threshold 2.
        let hidden = Noticing { hider_level: 20, ..looking_away(30.0) };
        assert_near(threshold(&hidden), 2.0);
    }

    #[test]
    fn concealment_takes_ten_percent_a_point() {
        // gap 30: percent 70. Concealment 2 -> 50 >= 30 sees; 5 -> 20 < 30 does not.
        assert!(notices(&Noticing { concealment: 2, ..looking_away(30.0) }));
        assert_near(threshold(&Noticing { concealment: 5, ..looking_away(30.0) }), 20.0);
        assert!(!notices(&Noticing { concealment: 5, ..looking_away(30.0) }));
        // Facing, 100 - 10 * 3 = 70.
        let facing = Noticing { angle: 0.0, concealment: 3, ..looking_away(60.0) };
        assert_near(threshold(&facing), 70.0);
    }

    #[test]
    fn a_kind_that_sees_further_adds_its_bonus_percent() {
        // gap 40: 30 * (60 / 40) = 45. Bonus 50: 150 * 45 / 100 = 67 (whole).
        let keen = Noticing { bonus: 50, ..looking_away(40.0) };
        assert_near(threshold(&keen), 67.0);
        assert!(notices(&keen));
        // gap 50: percent 30, bonus 50 -> 150 * 30 / 100 = 45 < 50.
        assert!(!notices(&Noticing { bonus: 50, ..looking_away(50.0) }));
        // The bonus is taken after concealment: 70 - 20 = 50, 150 * 50 / 100 = 75.
        let both = Noticing { bonus: 50, concealment: 2, ..looking_away(30.0) };
        assert_near(threshold(&both), 75.0);
    }

    #[test]
    fn a_percent_is_held_to_between_two_and_a_hundred() {
        // Senses below twice the level: negative, held to 2 -> 2 m.
        let blind_to_it = Noticing { senses: 5, hider_level: 10, ..looking_away(10.0) };
        assert_near(threshold(&blind_to_it), 2.0);
        assert!(notices(&Noticing { gap: 2.0, ..blind_to_it }));
        assert!(!notices(&Noticing { gap: 3.0, ..blind_to_it }));
        // A bonus cannot take it past 100.
        let keen = Noticing { bonus: 125, ..looking_away(20.0) };
        assert_near(threshold(&keen), 100.0);
    }

    #[test]
    fn shut_eyes_or_a_blocked_line_see_nothing() {
        let facing = Noticing { angle: 0.0, ..looking_away(5.0) };
        assert!(notices(&facing));
        assert!(!notices(&Noticing { open: false, ..facing }));
        assert!(!notices(&Noticing { line: false, ..facing }));
    }

    #[test]
    fn a_party_searching_sees_two_thirds_as_far() {
        // Facing: threshold 100 * 0.6667 = 66.67 m.
        let facing = Noticing { angle: 0.0, searching: true, ..looking_away(66.0) };
        assert!(notices(&facing));
        assert!(!notices(&Noticing { gap: 67.0, ..facing }));
        assert!(notices(&Noticing { searching: false, gap: 67.0, ..facing }));
    }
}
