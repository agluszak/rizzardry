//! Spell definitions (`data/databases/spelltables.dbs`): what each spell
//! costs, whom it may target and the authored numbers its effect scales.

use bevy::prelude::*;
use enumset::{EnumSet, EnumSetType};

use super::characters::Realm;
use super::items::Dice;

/// A retail spell number (`W8SpellId`); 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpellId(pub u8);

/// Declares the spells' numbers once, and their names to match: the
/// constants of [`SpellId`], and the names tables are written in.
macro_rules! spell_ids {
    ($($name:ident = $number:expr,)*) => {
        impl SpellId {
            $(pub const $name: Self = Self($number);)*
        }

        /// Every spell the game names, by the name of its constant.
        const NAMED: &[(&str, SpellId)] = &[$((stringify!($name), SpellId::$name),)*];
    };
}

// Retail identities: decomp include/wiz8/spell_ids.h. Keep names shared by all rules.
spell_ids! {
    NONE = 0,
    ACID_SPLASH = 1,
    BLESS = 2,
    CHARM = 3,
    FROST = 4,
    ENERGY_BLAST = 5,
    HEAL_WOUNDS = 6,
    ITCHING_SKIN = 7,
    LIGHT = 8,
    MAKE_WOUNDS = 9,
    MIND_STAB = 10,
    PARALYZE = 11,
    SLEEP = 12,
    STAMINA = 13,
    TERROR = 14,
    BLINDING_FLASH = 15,
    CURE_LESSER_COND = 16,
    DETECT_SECRETS = 17,
    DIVINE_TRAP = 18,
    DRACON_BREATH = 19,
    ENCHANTED_BLADE = 20,
    GUARDIAN_ANGEL = 21,
    HOLY_WATER = 22,
    IDENTIFY_ITEM = 23,
    INSANITY = 24,
    MAGIC_MISSILES = 25,
    MISSILE_SHIELD = 26,
    RAZOR_CLOAK = 27,
    SHRILL_SOUND = 28,
    SLOW = 29,
    SONIC_BOOM = 30,
    WEB = 31,
    ARMORPLATE = 32,
    CHAMELEON = 33,
    CURE_PARALYSIS = 34,
    CURE_POISON = 35,
    FIREBALL = 36,
    FREEZE_FLESH = 37,
    HYPNOTIC_LURE = 38,
    KNOCK_KNOCK = 39,
    MAGIC_SCREEN = 40,
    MINDREAD = 41,
    NOXIOUS_FUMES = 42,
    PSIONIC_FIRE = 43,
    REST_ALL = 44,
    SHADOW_HOUND = 45,
    SILENCE = 46,
    WHIPPING_ROCKS = 47,
    ACID_BOMB = 48,
    ARMORMELT = 49,
    CRUSH = 50,
    CURE_DISEASE = 51,
    EGO_WHIP = 52,
    ELEMENT_SHIELD = 53,
    EYE_FOR_AN_EYE = 54,
    FIRE_BOMB = 55,
    HASTE = 56,
    ICEBALL = 57,
    REMOVE_CURSE = 58,
    SOUL_SHIELD = 59,
    SUMMON_ELEMENTAL = 60,
    SUPERMAN = 61,
    RING_OF_FIRE = 62,
    WHIRLWIND = 63,
    X_RAY = 64,
    BODY_OF_STONE = 65,
    DEHYDRATE = 66,
    FREEZE_ALL = 67,
    HEAL_ALL = 68,
    HEX = 69,
    INSTANT_DEATH = 70,
    PSIONIC_BLAST = 71,
    PURIFY_AIR = 72,
    RETURN_TO_PORTAL = 73,
    SANE_MIND = 74,
    SET_PORTAL = 75,
    TOXIC_CLOUD = 76,
    BANISH = 77,
    BLIZZARD = 78,
    BOILING_BLOOD = 79,
    DRAINING_CLOUD = 80,
    FIRESTORM = 81,
    LIFESTEAL = 82,
    LIGHTNING = 83,
    MIGHT_TO_MAGIC = 84,
    PANDEMONIUM = 85,
    PRISMIC_RAY = 86,
    QUICKSAND = 87,
    RESURRECTION = 88,
    TURNCOAT = 89,
    ASPHYXIATION = 90,
    CEREBRAL_HEMORRHAGE = 91,
    CONCUSSION = 92,
    DEATH_CLOUD = 93,
    DEATH_WISH = 94,
    EARTHQUAKE = 95,
    FALLING_STARS = 96,
    MIND_FLAY = 97,
    NUCLEAR_BLAST = 98,
    PRISMIC_CHAOS = 99,
    RESTORATION = 100,
    TSUNAMI = 101,
    RESTORE_HEALTH = 114,
    RESTORE_MAGIC = 115,
    SMELLING_SALTS = 116,
    ROUT = 117,
    BOILING_BLOOD_EXPLOSION = 118,
    SPECIAL_ATTACK_CONE = 119,
    RENEWAL = 120,
    DAGGER_SCATTER = 121,
    POISON_DARTS = 122,
    ALARM = 123,
    PHILOSOPHERS_BANE = 124,
    VAMPIRIC_VORTEX = 125,
    HEATWAVE = 126,
    ANTI_MAGIC = 127,
    DEATH_CURSE = 128,
    DISPEL_UNDEAD = 129,
    BOOZE = 130,
    ROCKET_BLAST = 131,
}

/// Written as the name of its constant (`ENERGY_BLAST`), as tables name spells.
impl<'de> serde::Deserialize<'de> for SpellId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Name;
        impl serde::de::Visitor<'_> for Name {
            type Value = SpellId;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("the name of a SpellId constant")
            }
            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<SpellId, E> {
                NAMED
                    .iter()
                    .find_map(|&(named, id)| (named == name).then_some(id))
                    .ok_or_else(|| E::custom(format!("no spell is named {name}")))
            }
        }
        deserializer.deserialize_identifier(Name)
    }
}

impl SpellId {
    /// First spell outside the learnable spellbook range.
    pub const LEARNABLE: u8 = 0x72;
}

/// The highest power a caster may choose; retail's "8" means the highest
/// affordable one and is resolved before casting.
pub const MAX_POWER: u32 = 7;

/// The four spellbooks (retail mask bits: Wizardry 1, Divinity 2, Alchemy
/// 4, Psionics 8).
#[derive(Debug, EnumSetType, enum_map::Enum)]
#[enumset(repr = "u8")]
pub enum Spellbook {
    Wizardry,
    Divinity,
    Alchemy,
    Psionics,
}

/// How the player chooses a spell's power (record 0x12b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PowerChoice {
    #[default]
    Chosen,
    /// Chosen, with a "repeat until healed" choice outside combat.
    ChosenOrRepeat,
    Maximum,
    Fixed,
}

/// Whom a spell is cast at (record 0x137).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Targeting {
    #[default]
    Caster,
    Ally,
    Party,
    Enemy,
    EnemyGroup,
    Cone,
    Radius,
    AllEnemies,
    Point,
    Item,
    LockOrTrap,
}

/// When a spell may be cast (record 0x13b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Usable {
    #[default]
    Always,
    InCombat,
    OutOfCombat,
    Camped,
    OnLockOrTrap,
}

/// How far a spell reaches (record 0x12f), as attacks reach.
pub use super::species::Reach;

#[derive(Clone, Debug, Default)]
pub struct SpellDefinition {
    pub id: SpellId,
    pub name: String,
    pub realm: Option<Realm>,
    /// 1 to 7; a caster needs a caster level of at least
    /// [`min_caster_level`] for it.
    pub level: u32,
    /// Spell points per power level.
    pub cost: u32,
    pub books: EnumSet<Spellbook>,
    pub power: PowerChoice,
    pub reach: Reach,
    pub targeting: Targeting,
    pub usable: Usable,
    /// Magnitude dice at power 1; power multiplies their count.
    pub dice: Option<Dice>,
    /// Retail info-panel flag (record 0x147), independent of having effect dice.
    pub show_effect_dice: bool,
    /// Rounds an inflicted condition lasts: `base + per_power × power`.
    pub duration_base: i32,
    pub duration_per_power: i32,
    /// Metres, before `radius_per_power × power`.
    pub radius: f32,
    pub radius_per_power: f32,
    /// Flies to its target as a missile (`MissileTables.dbs` row).
    pub missile: Option<u32>,
    pub monsters_cast: bool,
}

impl SpellDefinition {
    /// Whether it is aimed at enemies, which provokes them (retail
    /// `MonsterCanAimSpell`, `CharacterActionTargetsEnemies`): an enemy,
    /// group, area or all-enemies spell other than Charm and Mindread.
    pub fn aimed_at_enemies(&self) -> bool {
        matches!(
            self.targeting,
            Targeting::Enemy
                | Targeting::EnemyGroup
                | Targeting::Cone
                | Targeting::Radius
                | Targeting::AllEnemies
        ) && self.id != SpellId::CHARM
            && self.id != SpellId::MINDREAD
    }

    /// Spell points a cast at `power` costs.
    pub fn cost_at(&self, power: u32) -> u32 {
        self.cost * power
    }

    /// Retail `CanSpellBackfire` (`Spells.cpp:1483-1531`): spells aimed at
    /// enemies, but Banish and two creature spells; eight friendly ones; of
    /// the lock spells, Knock Knock.
    pub fn can_backfire(&self) -> bool {
        match self.targeting {
            Targeting::Enemy
            | Targeting::EnemyGroup
            | Targeting::Cone
            | Targeting::Radius
            | Targeting::AllEnemies => !matches!(
                self.id,
                SpellId::BANISH | SpellId::BOILING_BLOOD_EXPLOSION | SpellId::ROCKET_BLAST
            ),
            Targeting::LockOrTrap => self.id == SpellId::KNOCK_KNOCK,
            _ => matches!(
                self.id,
                SpellId::CHARM
                    | SpellId::STAMINA
                    | SpellId::CURE_LESSER_COND
                    | SpellId::CURE_PARALYSIS
                    | SpellId::REST_ALL
                    | SpellId::HASTE
                    | SpellId::REMOVE_CURSE
                    | SpellId::SANE_MIND
            ),
        }
    }
}

/// The caster level a spell level needs (retail `Spells.cpp`).
pub fn min_caster_level(spell_level: u32) -> i32 {
    match spell_level {
        0 | 1 => 1,
        2 => 3,
        3 => 5,
        4 => 8,
        5 => 11,
        6 => 14,
        _ => 18,
    }
}

#[derive(Asset, TypePath, Debug, Default)]
pub struct SpellCatalog(pub Vec<SpellDefinition>);

impl SpellCatalog {
    pub fn get(&self, id: SpellId) -> Option<&SpellDefinition> {
        self.0.get(usize::from(id.0)).filter(|_| id.0 != 0)
    }
}
/// The definition of a spell the game names.
impl std::ops::Index<SpellId> for SpellCatalog {
    type Output = SpellDefinition;
    fn index(&self, id: SpellId) -> &SpellDefinition {
        self.get(id).expect("a spell the catalog defines")
    }
}

/// A spell an item casts, and at what power (item record 0x63).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemSpell {
    pub spell: SpellId,
    pub power: u32,
}

/// Localized spell effect and description text, loaded independently of rules.
#[derive(Asset, TypePath, Debug)]
pub struct SpellText(pub Vec<Option<String>>);
