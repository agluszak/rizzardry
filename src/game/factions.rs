//! The party's standing with retail's factions (`Factions.cpp`): a score per
//! faction, read in three bands, and the offences that lower it.

use bevy::prelude::*;

pub const UNALIGNED: i32 = 0;
pub const PARTY: i32 = 1;
const MOOK: usize = 6;
const RATTKIN_MAFIA: usize = 8;
const HIGARDI: [usize; 3] = [10, 11, 12];

/// How a faction stands toward the party.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Hostile,
    Neutral,
    Friendly,
}

/// Scores 0..99 per faction, and how often the party has offended them.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct Factions {
    scores: [u8; 21],
    offences: [u8; 21],
}

impl Default for Factions {
    /// Retail `ResetFactions`.
    fn default() -> Self {
        let scores =
            [0, 100, 0, 100, 80, 80, 80, 50, 80, 80, 80, 80, 80, 80, 70, 25, 50, 50, 80, 0, 0];
        Factions { scores, offences: [0; 21] }
    }
}

impl Factions {
    /// Hostile under 34, neutral under 67, friendly above; the party is
    /// always friendly to itself.
    pub fn band(&self, faction: i32) -> Band {
        if faction == PARTY {
            return Band::Friendly;
        }
        match usize::try_from(faction).ok().and_then(|f| self.scores.get(f)) {
            Some(&score) if score < 34 => Band::Hostile,
            Some(&score) if score < 67 => Band::Neutral,
            _ => Band::Friendly,
        }
    }

    pub fn score(&self, faction: i32) -> Option<u8> {
        usize::try_from(faction).ok().and_then(|f| self.scores.get(f)).copied()
    }

    /// The party killed one of the faction's (retail `RecordFactionOffense`).
    /// Returns whether the band worsened.
    pub fn record_offence(&mut self, faction: i32, npc: bool) -> bool {
        let Ok(index) = usize::try_from(faction) else { return false };
        if index <= PARTY as usize || index >= self.scores.len() {
            return false;
        }
        let before = self.band(faction);
        self.offences[index] = self.offences[index].saturating_add(1).min(250);
        if index == MOOK || index == RATTKIN_MAFIA || HIGARDI.contains(&index) {
            if before != Band::Hostile {
                self.scores[index] = 25;
            }
        } else {
            let loss = if npc {
                20
            } else {
                match self.offences[index] {
                    0..=20 => 2,
                    41.. => 10,
                    _ => 6,
                }
            };
            self.scores[index] = self.scores[index].saturating_sub(loss).min(99);
        }
        self.band(faction) != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_starting_bands_and_offences() {
        let mut factions = Factions::default();
        assert_eq!(factions.band(4), Band::Friendly, "Umpani");
        assert_eq!(factions.band(15), Band::Hostile, "common Rapax");
        assert_eq!(factions.band(7), Band::Neutral, "common Rattkin");
        assert!(factions.record_offence(MOOK as i32, false), "Mook turn at once");
        assert_eq!(factions.band(MOOK as i32), Band::Hostile);
        factions.record_offence(4, false);
        assert_eq!(factions.score(4), Some(78));
        factions.record_offence(4, true);
        assert_eq!(factions.score(4), Some(58));
        assert_eq!(factions.band(4), Band::Neutral);
    }
}
