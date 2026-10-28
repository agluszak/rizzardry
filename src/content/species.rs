//! Monster species read from `monsters.dbs`.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::characters::BodyPart;
use super::items::{AttackMode, Dice};

/// Retail monster categories, named by StringData entries 0x578..=0x598.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
pub enum MonsterKind {
    #[default]
    Fighter = 0,
    Wizard = 1,
    Priest = 2,
    Rogue = 3,
    Ranger = 4,
    Alchemist = 5,
    Bard = 6,
    Psionic = 7,
    Valkyrie = 8,
    Bishop = 9,
    Lord = 10,
    Samurai = 11,
    Monk = 12,
    Ninja = 13,
    Giant = 14,
    Tiny = 15,
    Beast = 16,
    Plant = 17,
    Insect = 18,
    Vapor = 19,
    Undead = 20,
    Demon = 21,
    Android = 22,
    SeaMonster = 23,
    Dragon = 24,
    Myth = 25,
    Robot = 26,
    Deity = 27,
    Spirit = 28,
    Flyer = 29,
    Swimmer = 30,
    Gadgeteer = 31,
    Reptile = 32,
}

/// How far an attack reaches (retail range category).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    #[default]
    Touch,
    Short,
    Long,
    Extreme,
}

impl Reach {
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

/// Monster attack announcements, named by retail strings 0x618..=0x63d.
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
pub enum AttackWord {
    Silent = 0,
    Verb = 1,
    Throw = 2,
    Shoot = 3,
    Claw = 4,
    Bite = 5,
    TailSweep = 6,
    Sting = 7,
    Touch = 8,
    Grapple = 9,
    Punch = 10,
    Kick = 11,
    Stomp = 12,
    Stare = 14,
    Bash = 15,
    Charge = 16,
    Peck = 17,
    Spit = 18,
}

impl AttackWord {
    pub fn text_id(self) -> usize {
        const FIRST_VERB: usize = 0x619;
        FIRST_VERB + 2 * self as usize
    }
}

/// One of a species' attacks.
#[derive(Clone, Debug, PartialEq)]
pub struct MonsterAttack {
    pub word: AttackWord,
    /// The attack has a weapon name of its own, which adds a preposition
    /// to its announcement.
    pub named: bool,
    pub reach: Reach,
    pub score: u8,
    pub modes: EnumSet<AttackMode>,
    pub damage: Dice,
}

/// A species' attributes, which the attack rolls compare with a
/// character's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MonsterAttributes {
    pub strength: u8,
    pub intelligence: u8,
    pub dexterity: u8,
    pub speed: u8,
    pub senses: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Species {
    /// What it is ("Ironskin Crocodile"), once the party knows.
    pub name: String,
    pub name_plural: String,
    /// What it looks like until then ("Crocodile").
    pub generic_name: String,
    pub generic_plural: String,
    /// `data/monsters/<model>.mls`.
    pub model: String,
    /// Rangers see the party further according to their level.
    pub level_sight: bool,
    /// The creature category used for targeting, slaying and immunities.
    pub kind: MonsterKind,
    /// An NPC's record: its loot is its NPC's, not the treasure table.
    pub npc: Option<super::npcs::NpcId>,
    /// Percent taken off every hit.
    pub damage_reduction: u8,
    pub attributes: MonsterAttributes,
    pub hit_points: Dice,
    pub stamina: Dice,
    pub initiative: u8,
    pub attacks_per_round: u8,
    pub swings_per_round: u8,
    pub attacks: Vec<MonsterAttack>,
    /// Where its hits land on a character (%).
    pub hits_on_party: EnumMap<BodyPart, u8>,
    /// Higher is easier to hit.
    pub evasion: i8,
    /// Where hits land on it (%), by its own seven locations, whose names
    /// depend on its build (Wing, Tail, Head, Body, Leg, Hand, Foot for
    /// most).
    pub hit_locations: [u8; 7],
    /// Armour by location and by attack mode; negative is better.
    pub armour_by_location: [i8; 7],
    pub armour_by_mode: EnumMap<AttackMode, i8>,
    pub experience: u32,
    /// The level the rules use.
    pub level: u8,
    /// The level the party is shown, which decides how much of its health
    /// they can tell.
    pub display_level: u8,
    pub gold: Dice,
    /// What it may leave behind, in retail's slot order.
    pub treasure: Vec<Treasure>,
    /// Its own resistance per realm (%).
    pub resistances: EnumMap<super::characters::Realm, u8>,
    /// Authored disposition (retail `hostility_radius`): hostile on sight,
    /// neutral, or neutral until the party comes within this many metres.
    pub disposition: Disposition,
    /// Retail faction (0 is unaligned); only unaligned species use the
    /// hostility radius.
    pub faction: i32,
    /// How far it moves in one combat action: this many times 20 metres
    /// (retail `combat_move_range`, `MonsterManager.cpp:620-633`).
    pub combat_move_range: f32,
}

/// How a species meets the party before anything provokes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Disposition {
    #[default]
    Hostile,
    Neutral,
    /// Neutral until the party is within this many metres.
    HostileWithin(f32),
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
#[derive(Clone, Debug, PartialEq)]
pub struct Treasure {
    pub drop: Drop,
    pub chance: u8,
    pub dice: Dice,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

impl MonsterCatalog {
    /// The species of record number `species`.
    pub fn kind(&self, species: u32) -> Option<&Species> {
        self.0.get(species as usize)
    }
}
