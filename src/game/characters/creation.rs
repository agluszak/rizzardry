//! What making a character offers: the playable races, the party's size,
//! a Valkyrie's gender, and the looks a recipe gives ([`identity`]). The
//! character being made is a draft ([`super::draft`]).

pub mod identity;
use super::*;

pub const PLAYABLE_RACES: [Race; 11] = [
    Race::Human,
    Race::Elf,
    Race::Dwarf,
    Race::Gnome,
    Race::Hobbit,
    Race::Faerie,
    Race::Lizardman,
    Race::Dracon,
    Race::Felpurr,
    Race::Rawulf,
    Race::Mook,
];

/// The most characters a new party starts with.
pub const PARTY_LIMIT: usize = 6;
/// The gender a profession gives a character whose gender is not chosen
/// yet: Valkyries are women.
pub fn default_gender(profession: Profession, gender: Gender, chosen: bool) -> Gender {
    match profession.row().only_gender {
        Some(only) if !chosen => only,
        _ => gender,
    }
}
