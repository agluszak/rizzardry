//! Item definitions: what every kind of item is, shared by every copy.

use bevy::prelude::*;
use enum_map::{Enum, EnumMap};
use enumset::{EnumSet, EnumSetType};
use num_enum::{IntoPrimitive, TryFromPrimitive};

use super::characters::{Attribute, Gender, Profession, Race, Realm, Skill};
use super::species::Reach;

/// An item definition's stable number (retail's item number).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub u16);

/// `count` d `sides` + `base`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dice {
    pub count: u8,
    pub sides: u8,
    pub base: i16,
}

/// Where an item can be worn or wielded, and what it is (retail equipment
/// class; labels 1087..1119 in the string table).
/// Declared in retail's order, so its retail number converts both ways.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum EquipClass {
    ShortWeapon,
    ExtendedWeapon,
    ThrownWeapon,
    RangedWeapon,
    Ammunition,
    Shield,
    Torso,
    Legs,
    Head,
    Hands,
    Feet,
    Misc,
    Cloak,
    Instrument,
    Gadget,
    MiscMagic,
    Potion,
    Bomb,
    Powder,
    Spellbook,
    Scroll,
    Food,
    Drink,
    Key,
    Writing,
    #[default]
    Other,
    Dagger,
    LongSword,
    Bipennis,
    BattleAxe,
    Flail,
    Mace,
}

/// How an item counts: a stack of identical things, or charges, uses or
/// shots left in one thing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuantityKind {
    #[default]
    None,
    Stack,
    Charges,
    Uses,
    Shots,
    /// Retail value 5, which has no label.
    Unlabelled,
}

/// How a blow is struck, in retail's order (its attack mode bits).
#[derive(Debug, Enum, EnumSetType)]
#[enumset(repr = "u16")]
pub enum AttackMode {
    Swing,
    Thrust,
    Bash,
    Berserk,
    Throw,
    Punch,
    Kick,
    Lash,
    Shoot,
}

/// What a blow may do besides its damage, in retail's order (its attack
/// effect slots).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enum)]
pub enum AttackEffect {
    Sleep,
    Paralyze,
    Poison,
    Hex,
    Disease,
    Kill,
    KnockOut,
    Blind,
    Frighten,
    Swallow,
    Possess,
    DrainHealth,
    DrainStamina,
    DrainSpellPoints,
    Nauseate,
    Insane,
}

/// The percent chance of each effect a blow lands with, and its strength
/// (retail `missile_values` and `missile_magnitude`, which a weapon, its
/// ammunition and a monster's attack all carry).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackEffects {
    pub chances: EnumMap<AttackEffect, u8>,
    pub strength: u8,
}

impl AttackEffects {
    /// A weapon's effects with its ammunition's, byte by byte as retail
    /// sums them.
    pub fn with(mut self, other: &AttackEffects) -> Self {
        for (effect, chance) in &mut self.chances {
            *chance = chance.wrapping_add(other.chances[effect]);
        }
        self.strength = self.strength.wrapping_add(other.strength);
        self
    }
}

/// What a launcher shoots, and what ammunition is (retail pairs them by
/// item kind in `CompatiblePartnerItems`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ammunition {
    Arrow,
    Quarrel,
    SlingStone,
}

#[derive(Clone, Debug, Default)]
pub struct ItemDefinition {
    pub id: ItemId,
    pub name: String,
    /// What it is called before it is identified ("Dagger (Unidentified)").
    pub unidentified_name: String,
    /// The item's picture, a retail path such as `data/items/dagger.sti`.
    pub image: String,
    /// Its model lying in the world, such as `data/items3d/dagger.itm`.
    pub model: String,
    pub class: EquipClass,
    /// Identified as soon as the party has it.
    pub auto_identify: bool,
    /// A quest item the party may not throw away.
    pub crucial: bool,
    pub merchant_cannot_sell: bool,
    pub identify_difficulty: u8,
    pub two_handed: bool,
    /// May be wielded in the off hand.
    pub off_hand: bool,
    pub container: bool,
    pub weapon_skill: Option<Skill>,
    /// How far a weapon attacks; two weapons in one set share it.
    pub reach: Reach,
    /// The ammunition a launcher shoots.
    pub shoots: Option<Ammunition>,
    /// The ammunition this is.
    pub ammunition: Option<Ammunition>,
    pub damage: Dice,
    /// The ways it strikes; one is chosen per attack.
    pub attack_modes: EnumSet<AttackMode>,
    /// What its blows may do besides damage.
    pub effects: AttackEffects,
    /// The monster kind it slays for an extra damage die.
    pub slays: Option<u8>,
    /// Swings it adds to each attack.
    pub extra_swings: i32,
    /// Its retail kind (its unidentified name), which a few combat rules
    /// single out.
    pub kind: u16,
    /// Thrown, it is not used up.
    pub never_depletes: bool,
    pub damage_bonus: i8,
    pub hit_bonus: i8,
    pub armor_class_bonus: i8,
    /// The spell it casts when used, at its own power.
    pub spell: Option<super::spells::ItemSpell>,
    /// How it is used, which decides the skill its spell is cast with.
    pub use_type: UseType,
    pub must_equip_to_use: bool,
    /// Authored health, stamina and spell-point regeneration rates.
    pub regeneration: [i8; 3],
    /// Resistance modifiers while equipped, by magical realm.
    pub resistance_bonus: EnumMap<Realm, i8>,
    pub quantity: QuantityKind,
    /// Rolled when the item is created.
    pub initial_quantity: Dice,
    /// The largest stack retail merges into.
    pub stack_limit: u8,
    /// Who may use it.
    pub professions: EnumSet<Profession>,
    pub races: EnumSet<Race>,
    pub genders: EnumSet<Gender>,
    /// What its user needs at least; 0 for nothing.
    pub attribute_minimums: EnumMap<Attribute, u8>,
    pub skill_minimums: EnumMap<Skill, u8>,
    pub value: u32,
    /// Tenths of a pound.
    pub weight: u16,
    /// Retail's "binds on equip" byte; the assay screen shows it means cursed.
    pub cursed: bool,
}

impl ItemDefinition {
    pub fn requires_worn(&self) -> bool {
        use EquipClass::*;
        self.must_equip_to_use
            || matches!(
                self.class,
                ShortWeapon
                    | ExtendedWeapon
                    | RangedWeapon
                    | ThrownWeapon
                    | Ammunition
                    | Misc
                    | Head
                    | Torso
                    | Legs
                    | Hands
                    | Feet
                    | Cloak
                    | Shield
            )
    }

    /// Retail preserves an exhausted item only when it still has these properties.
    pub fn keeps_empty(&self) -> bool {
        !self.attack_modes.is_empty()
            || self.armor_class_bonus != 0
            || self.cursed
            || self.resistance_bonus.values().any(|&value| value != 0)
            || self.regeneration.iter().any(|&value| value != 0)
    }
}

/// Every item definition, by id.
/// How an item is used (retail record 0x42, `ubUseType`), and the skill
/// casting its spell takes (retail `g_item_spell_presentation`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UseType {
    #[default]
    None,
    /// Wands, rods and charged worn items.
    Charged,
    Scroll,
    Spellbook,
    Food,
    Potion,
    Instrument,
    /// Bombs and powders.
    Thrown,
    Gadget,
    Flamethrower,
}

impl UseType {
    /// The skill its spell is cast with; none for potions and food, whose
    /// spells never fail.
    pub fn skill(self) -> Option<Skill> {
        match self {
            UseType::Charged | UseType::Scroll => Some(Skill::Artifacts),
            UseType::Instrument => Some(Skill::Music),
            UseType::Thrown => Some(Skill::ThrowingAndSling),
            UseType::Gadget => Some(Skill::Engineering),
            UseType::Flamethrower => Some(Skill::ModernWeapon),
            UseType::None | UseType::Spellbook | UseType::Food | UseType::Potion => None,
        }
    }
}

#[derive(Asset, TypePath, Debug, Default)]
pub struct ItemCatalog(pub Vec<ItemDefinition>);

impl ItemCatalog {
    pub fn get(&self, id: ItemId) -> Option<&ItemDefinition> {
        self.0.get(usize::from(id.0))
    }
}

/// A loot table: weighted items and how many to draw.
#[derive(Clone, Debug)]
pub struct LootTable {
    pub name: String,
    /// Items and weights; weight 0 always drops.
    pub entries: Vec<(ItemId, u8)>,
    /// Draw only items fit for the party's level.
    pub level_scaled: bool,
    pub item_count: Dice,
    pub gold: Dice,
}

#[derive(Asset, TypePath, Debug, Default)]
pub struct LootTables(pub Vec<LootTable>);

impl LootTables {
    /// A table by name, ignoring case as retail does.
    pub fn find(&self, name: &str) -> Option<&LootTable> {
        self.0.iter().find(|table| table.name.eq_ignore_ascii_case(name))
    }
}

/// What an item looks like lying in the world (retail `.itm`).
#[derive(Asset, TypePath, Debug)]
pub struct ItemModel {
    pub parts: Vec<ItemModelPart>,
}

/// One material's faces of an item model.
#[derive(Debug)]
pub struct ItemModelPart {
    pub mesh: Handle<Mesh>,
    pub surface: super::surface::Surface,
}
