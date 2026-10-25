//! Monster species: what the combat rules read from `monsters.dbs`
//! (`docs/research/combat.md` §6).

use bevy::prelude::*;

use super::items::Dice;

/// How far an attack reaches (retail range category).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Reflect)]
pub enum Reach {
    Touch,
    Short,
    Long,
    Extreme,
}

impl Reach {
    pub fn from_retail(category: u8) -> Self {
        match category {
            0 => Reach::Touch,
            1 => Reach::Short,
            2 => Reach::Long,
            _ => Reach::Extreme,
        }
    }

    /// The furthest a target may be, in metres: 2, 4, 25 or 50.
    pub fn metres(self) -> f32 {
        match self {
            Reach::Touch => 2.0,
            Reach::Short => 4.0,
            Reach::Long => 25.0,
            Reach::Extreme => 50.0,
        }
    }
}

/// One of a species' attacks.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct MonsterAttack {
    pub reach: Reach,
    pub score: u8,
    /// Bit per attack mode: swing, thrust, bash, berserk, throw, punch,
    /// kick, lash, shoot.
    pub modes: u16,
    pub damage: Dice,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct Species {
    pub name: String,
    /// `data/monsters/<model>.mls`.
    pub model: String,
    pub kind: u8,
    /// An NPC's record: its loot is its NPC's, not the treasure table.
    pub npc: bool,
    /// Percent taken off every hit.
    pub damage_reduction: u8,
    /// Strength, intelligence, dexterity, speed, senses.
    pub attributes: [u8; 5],
    pub hit_points: Dice,
    pub stamina: Dice,
    pub flee_chance: u8,
    pub initiative: u8,
    pub attacks_per_round: u8,
    pub swings_per_round: u8,
    pub attacks: Vec<MonsterAttack>,
    /// Where its hits land on a character: head, body, leg, hand, foot (%).
    pub hits_on_party: [u8; 5],
    /// Higher is easier to hit.
    pub evasion: i8,
    /// Where hits land on it (%), by its own seven locations.
    pub hit_locations: [u8; 7],
    /// Armour by location and by attack mode; negative is better.
    pub armour_by_location: [i8; 7],
    pub armour_by_mode: [i8; 9],
    pub experience: u32,
    /// The level the rules use.
    pub level: u8,
    pub gold: Dice,
    pub prefers_ranged: bool,
}

/// Every species, by retail record number.
#[derive(Asset, TypePath, Debug, Default)]
pub struct MonsterCatalog(pub Vec<Species>);
