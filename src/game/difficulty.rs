//! The campaign's difficulty and what it scales (retail `g_settings.difficulty`,
//! `DifficultyScaling.cpp`, `Game Difficulty.cpp`, `ScaleByCombatPace`).

use bevy::prelude::*;

/// Retail's three settings; new games start on Normal (`Gameplay Init.cpp:494`).
#[derive(
    Resource, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum Difficulty {
    Novice,
    #[default]
    Normal,
    Expert,
}

/// How a monster stands toward the party, for difficulty scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Hostile,
    Neutral,
    Friendly,
}

impl Difficulty {
    /// A party member's figure (retail `ScaleValueForCharacterDifficulty`):
    /// easier on Novice, harder on Expert; a turncoat's the other way round.
    pub fn for_character(self, value: i32, turncoat: bool) -> i32 {
        match (self, turncoat) {
            (Difficulty::Normal, _) => value,
            (Difficulty::Novice, false) | (Difficulty::Expert, true) => value * 7 * 20 / 100,
            (Difficulty::Expert, false) | (Difficulty::Novice, true) => value * 3 * 20 / 100,
        }
    }

    /// A monster's figure (retail `ScaleValueForMonsterDifficulty`): hostile
    /// ones are weaker on Novice and stronger on Expert, friendly ones the
    /// reverse, neutral ones unscaled.
    pub fn for_monster(self, value: i32, side: Side) -> i32 {
        match (self, side) {
            (Difficulty::Normal, _) | (_, Side::Neutral) => value,
            (Difficulty::Novice, Side::Hostile) | (Difficulty::Expert, Side::Friendly) => {
                value * 3 * 20 / 100
            }
            (Difficulty::Expert, Side::Hostile) | (Difficulty::Novice, Side::Friendly) => {
                value * 7 * 20 / 100
            }
        }
    }

    /// Retail `ScaleByCombatPace`: an action re-committed late in a round
    /// (`stamp`, the round's phase then) is harder, past 80/60/40 by setting.
    pub fn scale_by_pace(self, value: u32, stamp: u32) -> u32 {
        let pace = match self {
            Difficulty::Novice => 80,
            Difficulty::Normal => 60,
            Difficulty::Expert => 40,
        };
        if pace > stamp { value } else { (stamp + 50 - pace) * value * 2 / 100 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_follows_retail_factors() {
        assert_eq!(Difficulty::Novice.for_character(50, false), 70);
        assert_eq!(Difficulty::Expert.for_character(50, false), 30);
        assert_eq!(Difficulty::Expert.for_character(50, true), 70);
        assert_eq!(Difficulty::Novice.for_monster(50, Side::Hostile), 30);
        assert_eq!(Difficulty::Expert.for_monster(50, Side::Hostile), 70);
        assert_eq!(Difficulty::Expert.for_monster(50, Side::Neutral), 50);
        assert_eq!(Difficulty::Normal.scale_by_pace(40, 0), 40);
        assert_eq!(Difficulty::Normal.scale_by_pace(40, 60), 40);
        assert_eq!(Difficulty::Normal.scale_by_pace(50, 100), 90);
    }
}
