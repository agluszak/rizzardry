//! The party's standing with retail's factions (`Factions.cpp`): a score per
//! faction, read in three bands, and the offences that lower it.

use bevy::prelude::*;

use crate::content::factions::Faction;
use crate::content::species::{Disposition, Species};

pub const UNALIGNED: i32 = Faction::Unaligned as i32;
pub const PARTY: i32 = Faction::Party as i32;
pub const DARK_SAVANT: i32 = Faction::DarkSavant as i32;
pub const COSMIC_LORDS: i32 = Faction::CosmicLords as i32;
pub const UMPANI: i32 = Faction::Umpani as i32;
pub const TRANG: i32 = Faction::Trang as i32;
pub const MOOK: i32 = Faction::Mook as i32;
pub const RATTKIN_COMMON: i32 = Faction::RattkinCommon as i32;
pub const RATTKIN_MAFIA: i32 = Faction::RattkinMafia as i32;
pub const BROTHERHOOD: i32 = Faction::Brotherhood as i32;
pub const HIGARDI_BANK: i32 = Faction::HigardiBank as i32;
pub const HIGARDI_LUNAR_LEGION: i32 = Faction::HigardiLunarLegion as i32;
pub const HIGARDI_COMMON: i32 = Faction::HigardiCommon as i32;
pub const TRYNNIE: i32 = Faction::Trynnie as i32;
pub const MAD_MARTEN: i32 = Faction::MadMarten as i32;
pub const RAPAX_COMMON: i32 = Faction::RapaxCommon as i32;
pub const RAPAX_TEMPLAR: i32 = Faction::RapaxTemplar as i32;
pub const RAPAX_ARMY: i32 = Faction::RapaxArmy as i32;
pub const KINGS_ASSASSINS: i32 = Faction::KingsAssassins as i32;
const HIGARDI: [i32; 3] = [HIGARDI_BANK, HIGARDI_LUNAR_LEGION, HIGARDI_COMMON];

/// How a faction stands toward the party.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Hostile,
    Neutral,
    Friendly,
}

/// Scores 0..99 per faction, how often the party has offended them, and
/// when (game milliseconds) each last moved into another band.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct Factions {
    encountered: [bool; 21],
    scores: [u8; 21],
    offences: [u8; 21],
    band_changed: [u64; 21],
}

impl Default for Factions {
    /// Retail `ResetFactions`.
    fn default() -> Self {
        let scores =
            [0, 100, 0, 100, 80, 80, 80, 50, 80, 80, 80, 80, 80, 80, 70, 25, 50, 50, 80, 0, 0];
        Factions { scores, offences: [0; 21], encountered: [false; 21], band_changed: [0; 21] }
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

    /// When the faction's band last changed (retail `band_changed_clock`).
    pub fn band_changed_at(&self, faction: i32) -> u64 {
        usize::try_from(faction).ok().and_then(|f| self.band_changed.get(f)).copied().unwrap_or(0)
    }

    pub fn score(&self, faction: i32) -> Option<u8> {
        usize::try_from(faction).ok().and_then(|f| self.scores.get(f)).copied()
    }

    /// A scripted change at `now` (retail `ApplyFactionChange` mode 3):
    /// relative, not an offence, and never to the unaligned or the party
    /// itself.
    pub fn adjust(&mut self, faction: Faction, delta: i8, now: u64) -> Option<FactionNews> {
        if matches!(faction, Faction::Unaligned | Faction::Party) {
            return None;
        }
        let index = faction as usize;
        let band = self.band(index as i32);
        let news = self.shift(index, i32::from(delta));
        if self.band(index as i32) != band {
            self.band_changed[index] = now;
        }
        news
    }

    /// The party killed one of the faction's (retail `RecordFactionOffense`).
    /// Mook, the Rattkin mafia and the Higardi turn hostile at once; anyone
    /// already hostile, and every other faction, loses standing by the
    /// victim and how often the party has offended.
    pub fn record_offence(&mut self, faction: i32, npc: bool) -> Option<FactionNews> {
        let index = usize::try_from(faction).ok()?;
        if index <= PARTY as usize || index >= self.scores.len() {
            return None;
        }
        self.offences[index] = self.offences[index].saturating_add(1).min(250);
        if (faction == MOOK || faction == RATTKIN_MAFIA || HIGARDI.contains(&faction))
            && self.band(faction) != Band::Hostile
        {
            // Retail `SetFactionDispositionBand`: the hostile band's anchor.
            self.scores[index] = 25;
            return Some(FactionNews { faction, improved: false });
        }
        let loss = if npc {
            20
        } else {
            match self.offences[index] {
                0..=20 => 2,
                41.. => 10,
                _ => 6,
            }
        };
        self.shift(index, -loss)
    }

    /// Move a score by `delta`, clamped to 0..99 (retail
    /// `AdjustFactionDisposition`); any movement is news.
    fn shift(&mut self, index: usize, delta: i32) -> Option<FactionNews> {
        let score = &mut self.scores[index];
        let before = *score;
        *score = (i32::from(before) + delta).clamp(0, 99) as u8;
        (*score != before)
            .then_some(FactionNews { faction: index as i32, improved: *score > before })
    }
}

/// A faction's standing toward the party moved (retail's notice naming the
/// faction and whether it worsened or improved).
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactionNews {
    pub faction: i32,
    pub improved: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_adjustments_clamp_without_recording_an_offence() {
        let mut factions = Factions::default();
        let worse = Some(FactionNews { faction: UMPANI, improved: false });
        assert_eq!(factions.adjust(Faction::Umpani, -100, 5), worse);
        assert_eq!((factions.score(UMPANI), factions.band(UMPANI)), (Some(0), Band::Hostile));
        assert_eq!(factions.band_changed_at(UMPANI), 5);
        assert_eq!(factions.adjust(Faction::Umpani, 10, 6).map(|n| n.improved), Some(true));
        assert_eq!(factions.band_changed_at(UMPANI), 5, "still hostile");
        assert_eq!(factions.adjust(Faction::Umpani, 100, 7).map(|n| n.improved), Some(true));
        assert_eq!((factions.score(UMPANI), factions.band_changed_at(UMPANI)), (Some(99), 7));
        assert_eq!(factions.adjust(Faction::Umpani, 20, 8), None, "no movement, no news");
        assert_eq!(factions.offences[UMPANI as usize], 0);
        assert_eq!(factions.adjust(Faction::Party, -100, 9), None);
        assert_eq!(factions.score(PARTY), Some(100));
    }

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
        assert!(factions.record_offence(MOOK, false).is_some(), "Mook turn at once");
        assert_eq!(factions.score(MOOK), Some(25));
        factions.record_offence(MOOK, false);
        assert_eq!(factions.score(MOOK), Some(23), "hostile Mook lose standing as others do");
        factions.record_offence(4, false);
        assert_eq!(factions.score(4), Some(78));
        factions.record_offence(4, true);
        assert_eq!(factions.score(4), Some(58));
        assert_eq!(factions.band(4), Band::Neutral);
    }
}
