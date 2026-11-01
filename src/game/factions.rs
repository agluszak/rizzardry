//! The party's standing with retail's factions (`Factions.cpp`): a score per
//! faction, read in three bands, and the offences that lower it.

use bevy::prelude::*;

use crate::content::factions::Faction;

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

/// Scores, offence counts and the accepted game time of the last band change.
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

    /// Accepted game time of the last band change; score changes within
    /// the same band do not reset a monster group's stance policy.
    pub fn band_changed_at(&self, faction: i32) -> u64 {
        usize::try_from(faction).ok().and_then(|f| self.band_changed.get(f)).copied().unwrap_or(0)
    }

    pub fn score(&self, faction: i32) -> Option<u8> {
        usize::try_from(faction).ok().and_then(|f| self.scores.get(f)).copied()
    }

    /// A scripted change (retail `ApplyFactionChange` mode 3): relative,
    /// not an offence, and never to the unaligned or the party itself.
    pub fn adjust(&mut self, faction: Faction, delta: i8, now: u64) -> Option<FactionNews> {
        if matches!(faction, Faction::Unaligned | Faction::Party) {
            return None;
        }
        self.shift(faction as usize, i32::from(delta), now)
    }

    /// The party killed one of the faction's (retail `RecordFactionOffense`).
    /// Mook, the Rattkin mafia and the Higardi turn hostile at once; anyone
    /// already hostile, and every other faction, loses standing by the
    /// victim and how often the party has offended.
    pub fn record_offence(&mut self, faction: i32, npc: bool, now: u64) -> Option<FactionNews> {
        let index = usize::try_from(faction).ok()?;
        if index <= PARTY as usize || index >= self.scores.len() {
            return None;
        }
        self.offences[index] = (self.offences[index] + 1).min(250);
        if (faction == MOOK || faction == RATTKIN_MAFIA || HIGARDI.contains(&faction))
            && self.band(faction) != Band::Hostile
        {
            // Retail `SetFactionDispositionBand`: the hostile band's anchor.
            return self.set_score(index, 25, now);
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
        self.shift(index, -loss, now)
    }

    /// Move a score by `delta`, clamped to 0..99 (retail
    /// `AdjustFactionDisposition`); any movement is news.
    fn shift(&mut self, index: usize, delta: i32, now: u64) -> Option<FactionNews> {
        self.set_score(index, (i32::from(self.scores[index]) + delta).clamp(0, 99) as u8, now)
    }

    /// Commit the score, band history and notification together.
    fn set_score(&mut self, index: usize, score: u8, now: u64) -> Option<FactionNews> {
        let before = self.scores[index];
        let band = self.band(index as i32);
        self.scores[index] = score;
        if self.band(index as i32) != band {
            self.band_changed[index] = now;
        }
        (score != before).then_some(FactionNews { faction: index as i32, improved: score > before })
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
        assert_eq!(factions.adjust(Faction::Umpani, -100, 10), worse);
        assert_eq!((factions.score(UMPANI), factions.band(UMPANI)), (Some(0), Band::Hostile));
        assert_eq!(factions.adjust(Faction::Umpani, 100, 10).map(|n| n.improved), Some(true));
        assert_eq!(factions.score(UMPANI), Some(99));
        assert_eq!(factions.band_changed_at(UMPANI), 10);
        factions.adjust(Faction::Umpani, -1, 20);
        assert_eq!(
            factions.band_changed_at(UMPANI),
            10,
            "same-band score changes preserve the clock"
        );
        factions.adjust(Faction::Umpani, -40, 30);
        assert_eq!(factions.band_changed_at(UMPANI), 30);
        factions.adjust(Faction::Umpani, 100, 40);
        assert_eq!(factions.adjust(Faction::Umpani, 20, 10), None, "no movement, no news");
        assert_eq!(factions.offences[UMPANI as usize], 0);
        assert_eq!(factions.adjust(Faction::Party, -100, 10), None);
        assert_eq!(factions.score(PARTY), Some(100));
    }

    #[test]
    fn retail_starting_bands_and_offences() {
        let mut factions = Factions::default();
        assert_eq!(factions.band(4), Band::Friendly, "Umpani");
        assert_eq!(factions.band(15), Band::Hostile, "common Rapax");
        assert_eq!(factions.band(7), Band::Neutral, "common Rattkin");
        assert!(factions.record_offence(MOOK, false, 7).is_some(), "Mook turn at once");
        assert_eq!(factions.score(MOOK), Some(25));
        assert_eq!(factions.band_changed_at(MOOK), 7);
        factions.record_offence(MOOK, false, 7);
        assert_eq!(factions.score(MOOK), Some(23), "hostile Mook lose standing as others do");
        factions.record_offence(4, false, 7);
        assert_eq!(factions.score(4), Some(78));
        factions.record_offence(4, true, 7);
        assert_eq!(factions.score(4), Some(58));
        assert_eq!(factions.band(4), Band::Neutral);
    }
}
