//! The factions whose standing scripts change by name (retail
//! `W8Faction`).
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
