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
    pub const ENERGY_BLAST: SpellId = SpellId(5);
    pub const HEAL_WOUNDS: SpellId = SpellId(6);
    pub const SLEEP: SpellId = SpellId(12);
    /// Spells a character can learn are numbered below this (retail
    /// `spell_learned` covers 0..0x72).
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
    /// Applies only when its visuals have played out.
    pub after_visuals: bool,
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
            | Targeting::AllEnemies => !matches!(self.id.0, 77 | 118 | 131),
            Targeting::LockOrTrap => self.id.0 == 39,
            _ => matches!(self.id.0, 3 | 13 | 16 | 34 | 44 | 56 | 58 | 74),
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
