//! The party's standing with retail's factions (`Factions.cpp`): a score per
//! faction, read in three bands, and the offences that lower it.

use bevy::prelude::*;

use crate::content::species::{Disposition, Species};

pub const UNALIGNED: i32 = 0;
pub const PARTY: i32 = 1;
pub const DARK_SAVANT: i32 = 2;
pub const COSMIC_LORDS: i32 = 3;
pub const UMPANI: i32 = 4;
pub const TRANG: i32 = 5;
pub const MOOK: i32 = 6;
pub const RATTKIN_COMMON: i32 = 7;
pub const RATTKIN_MAFIA: i32 = 8;
pub const BROTHERHOOD: i32 = 9;
pub const HIGARDI_BANK: i32 = 10;
pub const HIGARDI_LUNAR_LEGION: i32 = 11;
pub const HIGARDI_COMMON: i32 = 12;
pub const TRYNNIE: i32 = 13;
pub const MAD_MARTEN: i32 = 14;
pub const RAPAX_COMMON: i32 = 15;
pub const RAPAX_TEMPLAR: i32 = 16;
pub const RAPAX_ARMY: i32 = 17;
pub const KINGS_ASSASSINS: i32 = 18;
const HIGARDI: [i32; 3] = [HIGARDI_BANK, HIGARDI_LUNAR_LEGION, HIGARDI_COMMON];

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
    encountered: [bool; 21],
    scores: [u8; 21],
    offences: [u8; 21],
}

impl Default for Factions {
    /// Retail `ResetFactions`.
    fn default() -> Self {
        let scores =
            [0, 100, 0, 100, 80, 80, 80, 50, 80, 80, 80, 80, 80, 80, 70, 25, 50, 50, 80, 0, 0];
        Factions { scores, offences: [0; 21], encountered: [false; 21] }
    }
}

impl Factions {
    pub fn encounter(&mut self, faction: i32) {
        if let Ok(index) = usize::try_from(faction)
            && let Some(known) = self.encountered.get_mut(index)
        {
            *known = true;
        }
    }
    pub fn encountered(&self, faction: i32) -> bool {
        usize::try_from(faction)
            .ok()
            .and_then(|i| self.encountered.get(i))
            .copied()
            .unwrap_or(false)
    }

    /// Whether a species fights the party at `distance` metres before
    /// anything has provoked it (retail `MonsterGroupCalcDefaultDisposition`):
    /// unaligned species by their hostility radius, others by their
    /// faction's band.
    pub fn hostile_on_sight(&self, species: &Species, distance: f32) -> bool {
        if species.faction != UNALIGNED {
            return self.band(species.faction) == Band::Hostile;
        }
        match species.disposition {
            Disposition::Hostile => true,
            Disposition::Neutral => false,
            Disposition::HostileWithin(radius) => distance <= radius,
        }
    }

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
        if faction == MOOK || faction == RATTKIN_MAFIA || HIGARDI.contains(&faction) {
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

    /// Retail's crabs keep to themselves until the party comes within their
    /// fifteen metres; aligned species follow their faction instead.
    #[test]
    fn a_hostility_radius_makes_an_unaligned_species_hostile_only_up_close() {
        let factions = Factions::default();
        let crab = Species { disposition: Disposition::HostileWithin(15.0), ..default() };
        assert!(!factions.hostile_on_sight(&crab, 20.0) && factions.hostile_on_sight(&crab, 10.0));
        let neutral = Species { disposition: Disposition::Neutral, ..default() };
        assert!(!factions.hostile_on_sight(&neutral, 0.0));
        let rapax = Species { faction: 15, ..crab.clone() };
        assert!(factions.hostile_on_sight(&rapax, 20.0), "common Rapax start hostile");
        let umpani = Species { faction: 4, ..crab };
        assert!(!factions.hostile_on_sight(&umpani, 0.0), "Umpani are friends");
    }

    #[test]
    fn retail_starting_bands_and_offences() {
        let mut factions = Factions::default();
        assert_eq!(factions.band(4), Band::Friendly, "Umpani");
        assert_eq!(factions.band(15), Band::Hostile, "common Rapax");
        assert_eq!(factions.band(7), Band::Neutral, "common Rattkin");
        assert!(factions.record_offence(MOOK, false), "Mook turn at once");
        assert_eq!(factions.band(MOOK), Band::Hostile);
        factions.record_offence(4, false);
        assert_eq!(factions.score(4), Some(78));
        factions.record_offence(4, true);
        assert_eq!(factions.score(4), Some(58));
        assert_eq!(factions.band(4), Band::Neutral);
    }
}
