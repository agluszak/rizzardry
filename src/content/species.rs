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
    /// How the attack is announced: 1 names it by its mode's verb, others
    /// by a word of its own (string 0x619 + 2 × type, "Claws", "Bites").
    pub weapon_type: u8,
    /// The attack's weapon name, if any (0 for none).
    pub weapon_name: u8,
    pub reach: Reach,
    pub score: u8,
    /// Bit per attack mode: swing, thrust, bash, berserk, throw, punch,
    /// kick, lash, shoot.
    pub modes: u16,
    pub damage: Dice,
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct Species {
    /// What it is ("Ironskin Crocodile"), once the party knows.
    pub name: String,
    pub name_plural: String,
    /// What it looks like until then ("Crocodile").
    pub generic_name: String,
    pub generic_plural: String,
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
    /// The level the party is shown, which decides how much of its health
    /// they can tell.
    pub display_level: u8,
    pub gold: Dice,
    /// What it may leave behind, in retail's slot order.
    pub treasure: Vec<Treasure>,
    pub prefers_ranged: bool,
}

impl Species {
    /// What the party calls one of these, or a group of `count` (retail
    /// `GetMonsterName`, `GetMonsterGroupName`): its own name once its
    /// group is identified, its look before.
    pub fn shown_name(&self, identified: bool, count: usize) -> &str {
        match (identified, count == 1) {
            (true, true) => &self.name,
            (true, false) => &self.name_plural,
            (false, true) => &self.generic_name,
            (false, false) => &self.generic_plural,
        }
    }
}

/// One of a species' treasure slots (retail `W8MonsterTreasureEntry`): it
/// drops with `chance` percent, `dice` times.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct Treasure {
    pub drop: Drop,
    pub chance: u8,
    pub dice: Dice,
}

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub enum Drop {
    Item(super::items::ItemId),
    /// Items from this loot table.
    Table(u16),
    /// A slot of another type (176 retail slots are 205): rolled for, but
    /// drops nothing.
    Nothing,
}

/// Every species, by retail record number.
#[derive(Asset, TypePath, Debug, Default)]
pub struct MonsterCatalog(pub Vec<Species>);
