//! Monster species read from `monsters.dbs`.

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::characters::BodyPart;
use super::items::{AttackEffects, AttackMode, Dice};

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

/// How an attack is announced: by its mode's verb, or by a word of its own
/// ("Claws", "Bites"), retail string 0x619 + 2 × the word's number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackWord {
    Verb,
    Word(u8),
}

/// One of a species' attacks.
#[derive(Clone, Debug, PartialEq)]
pub struct MonsterAttack {
    pub word: AttackWord,
    /// The attack's weapon name (retail `ubWeaponNameIndex`), 0 for none.
    /// A named attack adds a preposition to its announcement.
    pub weapon_name: u8,
    pub reach: Reach,
    pub score: u8,
    pub modes: EnumSet<AttackMode>,
    pub damage: Dice,
    /// What its blows may do besides damage.
    pub effects: AttackEffects,
}

impl MonsterAttack {
    pub fn named(&self) -> bool {
        self.weapon_name != 0
    }

    /// Natural weapons that tire their user less: fatigue counts half
    /// against their aim (retail `g_low_fatigue_attack_verbs`).
    pub fn tires_little(&self) -> bool {
        const LOW_FATIGUE: [u8; 18] = [
            0x07, 0x0d, 0x0e, 0x0f, 0x15, 0x19, 0x21, 0x2b, 0x31, 0x32, 0x34, 0x35, 0x36, 0x38,
            0x3b, 0x55, 0x56, 0x57,
        ];
        LOW_FATIGUE.contains(&self.weapon_name)
    }
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
    /// Sees the party further by its level (retail species kind 4; what
    /// the kinds mean is not established).
    pub level_sight: bool,
    /// Retail's species kind (record 0xcb): 20 is undead; kinds 17, 20 and
    /// 22 are immune to some conditions.
    pub kind: u8,
    /// An NPC's record: its loot is its NPC's, not the treasure table.
    pub npc: Option<super::npcs::NpcId>,
    /// Percent taken off every hit.
    pub damage_reduction: u8,
    /// Easier to hit from behind, and it blocks blows that barely land
    /// (retail monster flag 0x02).
    pub vulnerable_from_behind: bool,
    /// Nothing kills it outright.
    pub death_immune: bool,
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
