//! Spell definitions (`data/databases/spelltables.dbs`): what each spell
//! costs, whom it may target and the authored numbers its effect scales.

use std::collections::HashMap;

use bevy::prelude::*;
use enumset::{EnumSet, EnumSetType};
use serde::Deserialize;

use super::characters::{Attribute, Realm};
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
#[derive(Debug, EnumSetType, enum_map::Enum, serde::Deserialize)]
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
    /// Aimed at enemies: an enemy, a group, an area or all of them.
    fn at_enemies(self) -> bool {
        matches!(
            self,
            Targeting::Enemy
                | Targeting::EnemyGroup
                | Targeting::Cone
                | Targeting::Radius
                | Targeting::AllEnemies
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
    /// What the game says of the spell besides its record, from
    /// `assets/tables/spells_laid.ron`.
    pub facts: SpellFacts,
}

impl SpellDefinition {
    /// Whether it is aimed at enemies, which provokes them (retail
    /// `MonsterCanAimSpell`, `CharacterActionTargetsEnemies`): an enemy,
    /// group, area or all-enemies spell, unless its row says otherwise.
    pub fn aimed_at_enemies(&self) -> bool {
        self.facts.aimed_at_enemies.unwrap_or(self.targeting.at_enemies())
    }

    /// Spell points a cast at `power` costs.
    pub fn cost_at(&self, power: u32) -> u32 {
        self.cost * power
    }

    /// Retail `CanSpellBackfire` (`Spells.cpp:1483-1531`): a spell aimed at
    /// enemies can, and any other cannot, unless its row says otherwise.
    pub fn can_backfire(&self) -> bool {
        self.facts.can_backfire.unwrap_or(self.targeting.at_enemies())
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
    /// The spells, each with the facts the game's table gives it.
    pub fn new(mut spells: Vec<SpellDefinition>) -> Self {
        let mut table = shipped_facts();
        for spell in &mut spells {
            spell.facts = table.remove(&spell.id).unwrap_or_default();
        }
        Self(spells)
    }

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

/// The table the game ships, by spell.
fn shipped_facts() -> HashMap<SpellId, SpellFacts> {
    ron::from_str(include_str!("../../assets/tables/spells_laid.ron"))
        .expect("assets/tables/spells_laid.ron is a table of spell facts")
}

/// What is true of a spell besides its record: one row of
/// `assets/tables/spells_laid.ron`, merged into the spell when the catalog is
/// made. Each field is explained at the head of that file.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpellFacts {
    /// Laid again, it replaces itself only if at least as strong.
    pub personal: bool,
    /// Laid, it lasts only a fight.
    pub only_in_a_fight: bool,
    /// Laid, it strikes as each round ends.
    pub pulse: Option<Pulse>,
    /// Whether it is aimed at enemies, where that is not what its targeting says.
    pub aimed_at_enemies: Option<bool>,
    /// Whether it can backfire, where that is not what its targeting says.
    pub can_backfire: Option<bool>,
    /// A death spell: what it takes off the difficulty its victims save against.
    pub death_floor: u32,
    /// What it adds while laid.
    pub laid: WhileLaid,
    /// What it does besides landing on bodies, if it is a utility spell: it is
    /// cast like any other, and then does this.
    pub working: Option<Working>,
    /// Cast by a member on themselves, the figure its failure chance gains.
    pub harder_on_oneself: u32,
}

/// What a utility spell does once it comes off (`docs/rules/casting.md`
/// section 8): not a deed on bodies, but an act of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Working {
    /// Reveals an item the party has.
    Identify,
    /// Saves where the party stands.
    SetPortal,
    /// Takes the party back to the place saved.
    ReturnToPortal,
    /// Wins over the one the party is talking with.
    Charm,
    /// Opens the mind of the one the party is talking with.
    Mindread,
    /// Draws the monsters near a spot.
    Lure,
    /// Calls an elemental to a spot.
    Summon,
    /// Moves a lock's tumblers.
    Knock,
    /// Reads the trap being disarmed.
    DivineTrap,
}

impl Working {
    /// Cast at the one the party is talking with.
    pub fn speaks(self) -> bool {
        matches!(self, Working::Charm | Working::Mindread)
    }

    /// Cast at a lock being picked.
    pub fn at_lock(self) -> bool {
        self == Working::Knock
    }

    /// Cast at a trap being disarmed.
    pub fn at_trap(self) -> bool {
        matches!(self, Working::DivineTrap | Working::Knock)
    }

    /// Cast at a spot the party sees, which in a fight is an order like any
    /// other.
    pub fn at_a_spot(self) -> bool {
        matches!(self, Working::Lure | Working::Summon)
    }
}

/// How a pulsing spell strikes as each round ends.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pulse {
    /// Where it comes among the pulses of the same bearer.
    pub order: u32,
    #[serde(default)]
    pub on: Pulsing,
}

/// Whom a pulse strikes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Pulsing {
    /// Whoever the spell is laid on: the living members of a party it hangs
    /// over, or the monster.
    #[default]
    Bearer,
    /// The monsters close around the party, sampled anew each pulse.
    AroundParty,
}

/// How a spell's boost raises an amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Boost {
    /// Not at all.
    #[default]
    Unboosted,
    /// Raised by the boost percent, rounded.
    Rounded,
    /// Raised by the boost percent, rounding down.
    Scaled,
}

/// An amount a spell adds: `per_power` times the power it was cast at, plus
/// `plus`, halved (rounding up) if `halved_up`, raised by the boost, then held
/// to `cap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Amount {
    pub per_power: i32,
    pub plus: i32,
    pub halved_up: bool,
    pub boost: Boost,
    pub cap: Option<i32>,
}

/// An amount added to everything of a kind, or only to those listed.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
pub struct Spread<T> {
    pub amount: Amount,
    #[serde(default)]
    pub only: Option<Vec<T>>,
}

impl<T: PartialEq> Spread<T> {
    pub fn covers(&self, kind: T) -> bool {
        self.only.as_ref().is_none_or(|only| only.contains(&kind))
    }
}

/// What a spell adds to its bearer while laid.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WhileLaid {
    pub armour: Option<Amount>,
    pub to_hit: Option<Amount>,
    pub resistance: Option<Spread<Realm>>,
    pub attributes: Option<Spread<Attribute>>,
    /// Percent chance a missile is turned aside.
    pub missiles: Option<Amount>,
    /// Percent cut off damage.
    pub reduction: Option<Amount>,
    pub concealment: Option<Amount>,
    /// The share of a stride added to a combat move, for each power.
    pub stride_per_power: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(id: SpellId, targeting: Targeting) -> SpellDefinition {
        SpellCatalog::new(vec![SpellDefinition { id, targeting, ..default() }]).0.remove(0)
    }

    /// Every row parses and names a spell the game names.
    #[test]
    fn every_row_parses_and_says_something_of_a_spell_that_exists() {
        let table = shipped_facts();
        assert!(!table.is_empty());
        for (id, facts) in &table {
            assert!(NAMED.iter().any(|&(_, named)| named == *id), "{id:?}");
            assert_ne!(*id, SpellId::NONE);
            assert_ne!(*facts, SpellFacts::default(), "{id:?} has an empty row");
        }
    }

    /// A spell is aimed at enemies, and can backfire, as its targeting says,
    /// unless its row says otherwise.
    #[test]
    fn who_is_aimed_at_and_who_backfires_follows_targeting_and_the_row() {
        let blast = with(SpellId::ENERGY_BLAST, Targeting::Enemy);
        assert!(blast.aimed_at_enemies() && blast.can_backfire());
        let charm = with(SpellId::CHARM, Targeting::Enemy);
        assert!(!charm.aimed_at_enemies() && charm.can_backfire());
        let mindread = with(SpellId::MINDREAD, Targeting::Enemy);
        assert!(!mindread.aimed_at_enemies());
        let banish = with(SpellId::BANISH, Targeting::Enemy);
        assert!(banish.aimed_at_enemies() && !banish.can_backfire());
        let knock = with(SpellId::KNOCK_KNOCK, Targeting::LockOrTrap);
        assert!(!knock.aimed_at_enemies() && knock.can_backfire());
        let heal = with(SpellId::HEAL_WOUNDS, Targeting::Ally);
        assert!(!heal.aimed_at_enemies() && !heal.can_backfire());
        assert!(with(SpellId::HASTE, Targeting::Caster).can_backfire());
    }

    /// A utility spell is named in its row; where each is cast is a fact of the
    /// kind of working; a spell harder on oneself names how much.
    #[test]
    fn a_utility_spell_names_what_it_does_and_where_it_is_cast() {
        let working = |id| with(id, Targeting::Enemy).facts.working;
        assert_eq!(
            [
                SpellId::IDENTIFY_ITEM,
                SpellId::SET_PORTAL,
                SpellId::RETURN_TO_PORTAL,
                SpellId::CHARM,
                SpellId::MINDREAD,
                SpellId::HYPNOTIC_LURE,
                SpellId::SUMMON_ELEMENTAL,
                SpellId::KNOCK_KNOCK,
                SpellId::DIVINE_TRAP,
            ]
            .map(working),
            [
                Some(Working::Identify),
                Some(Working::SetPortal),
                Some(Working::ReturnToPortal),
                Some(Working::Charm),
                Some(Working::Mindread),
                Some(Working::Lure),
                Some(Working::Summon),
                Some(Working::Knock),
                Some(Working::DivineTrap),
            ]
        );
        assert_eq!(working(SpellId::HEAL_WOUNDS), None);
        assert_eq!(working(SpellId::FIREBALL), None);
        let all = [
            Working::Identify,
            Working::SetPortal,
            Working::ReturnToPortal,
            Working::Charm,
            Working::Mindread,
            Working::Lure,
            Working::Summon,
            Working::Knock,
            Working::DivineTrap,
        ];
        let kinds = |f: fn(Working) -> bool| all.into_iter().filter(|&w| f(w)).collect::<Vec<_>>();
        assert_eq!(kinds(Working::speaks), [Working::Charm, Working::Mindread]);
        assert_eq!(kinds(Working::at_lock), [Working::Knock]);
        assert_eq!(kinds(Working::at_trap), [Working::Knock, Working::DivineTrap]);
        assert_eq!(kinds(Working::at_a_spot), [Working::Lure, Working::Summon]);
        let harder = |id| with(id, Targeting::Caster).facts.harder_on_oneself;
        assert_eq!((harder(SpellId::SANE_MIND), harder(SpellId::CURE_POISON)), (50, 0));
    }

    #[test]
    fn the_death_spells_take_their_floors_off() {
        let floor = |id| with(id, Targeting::Enemy).facts.death_floor;
        assert_eq!(
            [SpellId::INSTANT_DEATH, SpellId::QUICKSAND, SpellId::DEATH_CLOUD, SpellId::FIREBALL]
                .map(floor),
            [2, 4, 6, 0]
        );
    }
}
