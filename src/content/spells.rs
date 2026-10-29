//! Spell definitions (`data/databases/spelltables.dbs`): what each spell
//! costs, whom it may target and the authored numbers its effect scales.

use bevy::prelude::*;
use enumset::{EnumSet, EnumSetType};

use super::characters::Realm;
use super::items::Dice;

/// A retail spell number (`W8SpellId`); 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpellId(pub u8);

impl SpellId {
    // Retail identities: decomp include/wiz8/spell_ids.h. Keep names shared by all rules.
    pub const NONE: Self = Self(0);
    pub const ACID_SPLASH: Self = Self(1);
    pub const BLESS: Self = Self(2);
    pub const CHARM: Self = Self(3);
    pub const FROST: Self = Self(4);
    pub const ENERGY_BLAST: Self = Self(5);
    pub const HEAL_WOUNDS: Self = Self(6);
    pub const ITCHING_SKIN: Self = Self(7);
    pub const LIGHT: Self = Self(8);
    pub const MAKE_WOUNDS: Self = Self(9);
    pub const MIND_STAB: Self = Self(10);
    pub const PARALYZE: Self = Self(11);
    pub const SLEEP: Self = Self(12);
    pub const STAMINA: Self = Self(13);
    pub const TERROR: Self = Self(14);
    pub const BLINDING_FLASH: Self = Self(15);
    pub const CURE_LESSER_COND: Self = Self(16);
    pub const DETECT_SECRETS: Self = Self(17);
    pub const DIVINE_TRAP: Self = Self(18);
    pub const DRACON_BREATH: Self = Self(19);
    pub const ENCHANTED_BLADE: Self = Self(20);
    pub const GUARDIAN_ANGEL: Self = Self(21);
    pub const HOLY_WATER: Self = Self(22);
    pub const IDENTIFY_ITEM: Self = Self(23);
    pub const INSANITY: Self = Self(24);
    pub const MAGIC_MISSILES: Self = Self(25);
    pub const MISSILE_SHIELD: Self = Self(26);
    pub const RAZOR_CLOAK: Self = Self(27);
    pub const SHRILL_SOUND: Self = Self(28);
    pub const SLOW: Self = Self(29);
    pub const SONIC_BOOM: Self = Self(30);
    pub const WEB: Self = Self(31);
    pub const ARMORPLATE: Self = Self(32);
    pub const CHAMELEON: Self = Self(33);
    pub const CURE_PARALYSIS: Self = Self(34);
    pub const CURE_POISON: Self = Self(35);
    pub const FIREBALL: Self = Self(36);
    pub const FREEZE_FLESH: Self = Self(37);
    pub const HYPNOTIC_LURE: Self = Self(38);
    pub const KNOCK_KNOCK: Self = Self(39);
    pub const MAGIC_SCREEN: Self = Self(40);
    pub const MINDREAD: Self = Self(41);
    pub const NOXIOUS_FUMES: Self = Self(42);
    pub const PSIONIC_FIRE: Self = Self(43);
    pub const REST_ALL: Self = Self(44);
    pub const SHADOW_HOUND: Self = Self(45);
    pub const SILENCE: Self = Self(46);
    pub const WHIPPING_ROCKS: Self = Self(47);
    pub const ACID_BOMB: Self = Self(48);
    pub const ARMORMELT: Self = Self(49);
    pub const CRUSH: Self = Self(50);
    pub const CURE_DISEASE: Self = Self(51);
    pub const EGO_WHIP: Self = Self(52);
    pub const ELEMENT_SHIELD: Self = Self(53);
    pub const EYE_FOR_AN_EYE: Self = Self(54);
    pub const FIRE_BOMB: Self = Self(55);
    pub const HASTE: Self = Self(56);
    pub const ICEBALL: Self = Self(57);
    pub const REMOVE_CURSE: Self = Self(58);
    pub const SOUL_SHIELD: Self = Self(59);
    pub const SUMMON_ELEMENTAL: Self = Self(60);
    pub const SUPERMAN: Self = Self(61);
    pub const RING_OF_FIRE: Self = Self(62);
    pub const WHIRLWIND: Self = Self(63);
    pub const X_RAY: Self = Self(64);
    pub const BODY_OF_STONE: Self = Self(65);
    pub const DEHYDRATE: Self = Self(66);
    pub const FREEZE_ALL: Self = Self(67);
    pub const HEAL_ALL: Self = Self(68);
    pub const HEX: Self = Self(69);
    pub const INSTANT_DEATH: Self = Self(70);
    pub const PSIONIC_BLAST: Self = Self(71);
    pub const PURIFY_AIR: Self = Self(72);
    pub const RETURN_TO_PORTAL: Self = Self(73);
    pub const SANE_MIND: Self = Self(74);
    pub const SET_PORTAL: Self = Self(75);
    pub const TOXIC_CLOUD: Self = Self(76);
    pub const BANISH: Self = Self(77);
    pub const BLIZZARD: Self = Self(78);
    pub const BOILING_BLOOD: Self = Self(79);
    pub const DRAINING_CLOUD: Self = Self(80);
    pub const FIRESTORM: Self = Self(81);
    pub const LIFESTEAL: Self = Self(82);
    pub const LIGHTNING: Self = Self(83);
    pub const MIGHT_TO_MAGIC: Self = Self(84);
    pub const PANDEMONIUM: Self = Self(85);
    pub const PRISMIC_RAY: Self = Self(86);
    pub const QUICKSAND: Self = Self(87);
    pub const RESURRECTION: Self = Self(88);
    pub const TURNCOAT: Self = Self(89);
    pub const ASPHYXIATION: Self = Self(90);
    pub const CEREBRAL_HEMORRHAGE: Self = Self(91);
    pub const CONCUSSION: Self = Self(92);
    pub const DEATH_CLOUD: Self = Self(93);
    pub const DEATH_WISH: Self = Self(94);
    pub const EARTHQUAKE: Self = Self(95);
    pub const FALLING_STARS: Self = Self(96);
    pub const MIND_FLAY: Self = Self(97);
    pub const NUCLEAR_BLAST: Self = Self(98);
    pub const PRISMIC_CHAOS: Self = Self(99);
    pub const RESTORATION: Self = Self(100);
    pub const TSUNAMI: Self = Self(101);
    pub const RESTORE_HEALTH: Self = Self(114);
    pub const RESTORE_MAGIC: Self = Self(115);
    pub const SMELLING_SALTS: Self = Self(116);
    pub const ROUT: Self = Self(117);
    pub const BOILING_BLOOD_EXPLOSION: Self = Self(118);
    pub const SPECIAL_ATTACK_CONE: Self = Self(119);
    pub const RENEWAL: Self = Self(120);
    pub const DAGGER_SCATTER: Self = Self(121);
    pub const POISON_DARTS: Self = Self(122);
    pub const ALARM: Self = Self(123);
    pub const PHILOSOPHERS_BANE: Self = Self(124);
    pub const VAMPIRIC_VORTEX: Self = Self(125);
    pub const HEATWAVE: Self = Self(126);
    pub const ANTI_MAGIC: Self = Self(127);
    pub const DEATH_CURSE: Self = Self(128);
    pub const DISPEL_UNDEAD: Self = Self(129);
    pub const BOOZE: Self = Self(130);
    pub const ROCKET_BLAST: Self = Self(131);

    /// First spell outside the learnable spellbook range.
    pub const LEARNABLE: u8 = 0x72;
}

/// The highest power a caster may choose; retail's "8" means the highest
/// affordable one and is resolved before casting.
pub const MAX_POWER: u8 = 7;

/// The four spellbooks (retail mask bits: Wizardry 1, Divinity 2, Alchemy
/// 4, Psionics 8).
#[derive(Debug, EnumSetType)]
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

impl Targeting {
    pub fn hostile(self) -> bool {
        matches!(
            self,
            Targeting::Enemy
                | Targeting::EnemyGroup
                | Targeting::Cone
                | Targeting::Radius
                | Targeting::AllEnemies
                | Targeting::Point
        )
    }
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
    pub level: u8,
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
    /// `data/spells/sounds/<sound>`.
    pub sound: String,
}

impl SpellDefinition {
    /// Spell points a cast at `power` costs.
    pub fn cost_at(&self, power: u8) -> u32 {
        self.cost * u32::from(power)
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
pub fn min_caster_level(spell_level: u8) -> i32 {
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

/// A spell an item casts, and at what power (item record 0x63).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemSpell {
    pub spell: SpellId,
    pub power: u8,
}

/// Localized spell effect and description text, loaded independently of rules.
#[derive(Asset, TypePath, Debug)]
pub struct SpellText(pub Vec<Option<String>>);
