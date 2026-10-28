//! A new character's looks and manner: retail derives the initial
//! portrait, personality and voice from the recipe, and the player then
//! steps through the portraits a group at a time.

use super::CharacterDraft;
use crate::content::characters::{Personality, Voice};

pub const PORTRAITS: &[&str] = &[
    "hummf", "hummn", "hummm", "humm1", "humm2", "humm3", "humff", "humfn", "humfm", "humf1",
    "humf2", "humf3", "elfmf", "elfmn", "elfmm", "elfff", "elffn", "elffm", "dwarfmf", "dwarfmn",
    "dwarfmm", "dwarfff", "dwarffn", "dwarffm", "gnomemn", "gnomemm", "gnomeff", "gnomefn",
    "hobmf", "hobmn", "hobff", "hobfn", "fairymf", "fairymm", "fairyff", "fairyfn", "lizmf",
    "lizmn", "lizff", "lizfn", "dracmf", "dracmn", "dracff", "dracfn", "felpmf", "felpmn",
    "felpff", "felpfn", "rawmn", "rawmm", "rawff", "rawfn", "mookmf", "mookmn", "mookff", "mookfn",
    "ninmf", "ninff", "mook", "trynm1", "trynm2", "trynm3", "trang", "umpani", "sexus", "urq",
    "madras", "sparkle", "myles", "vi", "drazic", "tantris", "rodan", "glumph", "saxx", "humm4",
    "humf4", "elfm1", "elff1",
];

// Retail portrait defaults by gender, playable race, and profession.
const DEFAULT_PORTRAITS: [[[usize; 15]; 11]; 2] = [
    [
        [3, 1, 0, 0, 3, 56, 4, 2, 1, 2, 5, 4, 24, 4, 5],
        [13, 12, 0, 12, 13, 56, 13, 12, 12, 12, 14, 14, 14, 14, 14],
        [19, 19, 0, 19, 19, 56, 20, 20, 20, 20, 18, 18, 18, 18, 18],
        [24, 24, 0, 24, 24, 56, 24, 24, 25, 25, 25, 25, 25, 25, 25],
        [28, 28, 0, 28, 28, 56, 28, 28, 29, 29, 29, 29, 29, 29, 29],
        [32, 32, 0, 32, 32, 56, 32, 32, 33, 33, 33, 33, 33, 33, 33],
        [36, 36, 0, 36, 36, 56, 36, 36, 37, 37, 37, 37, 37, 37, 37],
        [40, 40, 0, 40, 40, 56, 41, 40, 41, 40, 41, 41, 41, 41, 41],
        [45, 45, 0, 45, 45, 56, 44, 44, 44, 44, 44, 44, 44, 44, 44],
        [48, 48, 0, 48, 48, 56, 48, 48, 49, 49, 49, 49, 49, 49, 49],
        [52, 52, 0, 52, 52, 56, 52, 52, 53, 53, 53, 53, 53, 53, 53],
    ],
    [
        [9, 11, 9, 10, 9, 57, 10, 11, 7, 7, 7, 8, 8, 8, 8],
        [15, 15, 15, 17, 15, 57, 17, 17, 16, 16, 17, 16, 16, 17, 16],
        [22, 21, 22, 22, 22, 57, 22, 21, 21, 23, 23, 23, 23, 23, 23],
        [26, 26, 26, 26, 26, 57, 26, 26, 27, 27, 27, 27, 27, 27, 27],
        [30, 30, 30, 30, 30, 57, 30, 30, 31, 31, 31, 31, 31, 31, 31],
        [35, 35, 35, 35, 35, 57, 35, 35, 34, 34, 34, 34, 34, 34, 34],
        [38, 38, 38, 38, 38, 57, 38, 38, 39, 39, 39, 39, 39, 39, 39],
        [42, 42, 42, 42, 42, 57, 42, 42, 43, 43, 43, 43, 43, 43, 43],
        [47, 47, 47, 47, 47, 57, 47, 47, 46, 46, 46, 46, 46, 46, 46],
        [50, 50, 50, 50, 50, 57, 50, 50, 51, 51, 51, 51, 51, 51, 51],
        [55, 55, 55, 55, 55, 57, 55, 55, 52, 52, 52, 52, 52, 52, 52],
    ],
];

const GROUPS: [&[usize]; 12] = [
    &[0, 1, 2, 3, 76, 4, 5, 6, 7, 8, 9, 77, 10, 11],
    &[12, 13, 14, 78, 15, 16, 17, 79],
    &[18, 19, 20, 21, 22, 23],
    &[24, 25, 26, 27],
    &[28, 29, 30, 31],
    &[32, 33, 34, 35],
    &[36, 37, 38, 39],
    &[40, 41, 42, 43],
    &[44, 45, 46, 47],
    &[48, 49, 50, 51],
    &[52, 53, 54, 55],
    &[56, 57],
];

/// Retail derives the initial portrait, personality and voice from the recipe.
pub fn initialize(draft: &mut CharacterDraft) {
    use crate::content::characters::{Gender, Profession, Race};
    let portraits = DEFAULT_PORTRAITS[draft.gender() as usize][draft.race() as usize];
    draft.portrait = PORTRAITS[portraits[draft.profession() as usize]].into();
    let class = if draft.gender() == Gender::Male
        && matches!(
            draft.profession(),
            Profession::Fighter | Profession::Lord | Profession::Ranger | Profession::Rogue
        ) {
        match draft.race() {
            Race::Dwarf => 16,
            Race::Lizardman => 15,
            _ => draft.profession() as usize,
        }
    } else {
        draft.profession() as usize
    };
    let table = [
        (0, 2),
        (0, 1),
        (6, 1),
        (0, 1),
        (0, 1),
        (2, 2),
        (0, 1),
        (8, 1),
        (7, 2),
        (7, 2),
        (7, 2),
        (7, 2),
        (7, 2),
        (7, 2),
        (4, 1),
        (4, 1),
        (1, 1),
        (5, 2),
        (7, 1),
        (6, 2),
        (6, 2),
        (6, 1),
        (3, 2),
        (3, 2),
        (5, 1),
        (1, 1),
        (1, 2),
        (5, 2),
        (1, 1),
        (1, 2),
        (2, 1),
        (2, 1),
        (2, 2),
        (2, 2),
    ];
    let (personality, voice) = table[class * 2 + draft.gender() as usize];
    let choices = [
        Personality::Aggressive,
        Personality::Intellectual,
        Personality::Burly,
        Personality::Chaotic,
        Personality::Cunning,
        Personality::Eccentric,
        Personality::Kindly,
        Personality::Laidback,
        Personality::Loner,
    ];
    draft.personality = choices[personality];
    draft.voice = if voice == 1 { Voice::First } else { Voice::Second };
}
fn position(current: &str) -> (usize, usize) {
    let index = PORTRAITS.iter().position(|&p| p == current).unwrap_or(0);
    GROUPS
        .iter()
        .enumerate()
        .find_map(|(group, items)| items.iter().position(|&i| i == index).map(|i| (group, i)))
        .unwrap_or((0, 0))
}
pub fn next_portrait(current: &str, direction: i32) -> &'static str {
    let (group, index) = position(current);
    let items = GROUPS[group];
    PORTRAITS[items[(index as i32 + direction).rem_euclid(items.len() as i32) as usize]]
}
pub fn next_portrait_group(current: &str, direction: i32) -> &'static str {
    let (group, _) = position(current);
    let group = (group as i32 + direction).rem_euclid(GROUPS.len() as i32) as usize;
    PORTRAITS[GROUPS[group][0]]
}
