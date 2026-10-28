//! Authored faction identities (retail Targeting.cpp's FindFactionByName).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Faction {
    Unaligned = 0,
    Party = 1,
    DarkSavant = 2,
    CosmicLords = 3,
    Umpani = 4,
    Trang = 5,
    Mook = 6,
    RattkinCommon = 7,
    RattkinMafia = 8,
    Brotherhood = 9,
    HigardiBank = 10,
    HigardiLunarLegion = 11,
    HigardiCommon = 12,
    Trynnie = 13,
    MadMarten = 14,
    RapaxCommon = 15,
    RapaxTemplar = 16,
    RapaxArmy = 17,
    KingsAssassins = 18,
}
impl Faction {
    pub fn from_script_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_uppercase().as_str() {
            "UNALIGNED" => Self::Unaligned,
            "FACTION_PARTY" => Self::Party,
            "FACTION_DARK_SAVANT" => Self::DarkSavant,
            "FACTION_COSMIC_LORDS" => Self::CosmicLords,
            "FACTION_UMPANI" => Self::Umpani,
            "FACTION_TRANG" => Self::Trang,
            "FACTION_MOOK" => Self::Mook,
            "FACTION_RATTKIN_COMMON" => Self::RattkinCommon,
            "FACTION_RATTKIN_MAFIA" => Self::RattkinMafia,
            "FACTION_BROTHERHOOD" => Self::Brotherhood,
            "FACTION_HIGARDI_BANK" => Self::HigardiBank,
            "FACTION_HIGARDI_HLL" => Self::HigardiLunarLegion,
            "FACTION_HIGARDI_COMMON" => Self::HigardiCommon,
            "FACTION_TRYNNIE" => Self::Trynnie,
            "FACTION_MAD_MARTEN" => Self::MadMarten,
            "FACTION_RAPAX_COMMON" => Self::RapaxCommon,
            "FACTION_RAPAX_TEMPLAR" => Self::RapaxTemplar,
            "FACTION_RAPAX_ARMY" => Self::RapaxArmy,
            "FACTION_KINGS_ASSASINS" => Self::KingsAssassins,
            _ => return None,
        })
    }
}
