//! Item definitions: what every kind of item is, shared by every copy.

use bevy::prelude::*;

/// An item definition's stable number (retail's item number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Reflect)]
pub struct ItemId(pub u16);

/// `count` d `sides` + `base`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct Dice {
    pub count: u8,
    pub sides: u8,
    pub base: i16,
}

/// Where an item can be worn or wielded, and what it is (retail equipment
/// class; labels 1087..1119 in the string table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
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
    Other,
    Dagger,
    LongSword,
    Bipennis,
    BattleAxe,
    Flail,
    Mace,
}

impl EquipClass {
    pub fn from_retail(class: u8) -> Option<Self> {
        use EquipClass::*;
        const ALL: [EquipClass; 32] = [
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
            Other,
            Dagger,
            LongSword,
            Bipennis,
            BattleAxe,
            Flail,
            Mace,
        ];
        ALL.get(usize::from(class)).copied()
    }
}

/// How an item counts: a stack of identical things, or charges, uses or
/// shots left in one thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum QuantityKind {
    None,
    Stack,
    Charges,
    Uses,
    Shots,
    /// Retail value 5, which has no label.
    Unlabelled,
}

/// A stat or skill an item asks a minimum of, by retail index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub struct Requirement {
    pub id: u8,
    pub minimum: u8,
}

#[derive(Clone, Debug, Reflect)]
pub struct ItemDefinition {
    pub id: ItemId,
    pub name: String,
    /// The generic kind (retail's unidentified-name index): what it looks
    /// like before identification, and which ammunition fits which weapon.
    pub kind: u16,
    /// The item's picture, a retail path such as `data/items/dagger.sti`.
    pub image: String,
    pub class: EquipClass,
    /// Identified as soon as the party has it.
    pub auto_identify: bool,
    /// A quest item the party may not throw away.
    pub crucial: bool,
    pub two_handed: bool,
    /// May be wielded in the off hand.
    pub off_hand: bool,
    pub merchants_refuse: bool,
    pub container: bool,
    /// Only works while equipped.
    pub equip_to_use: bool,
    pub never_depletes: bool,
    pub weapon_skill: Option<u8>,
    /// The range band a weapon attacks at; two weapons in one set share it.
    pub range: u8,
    pub damage: Dice,
    pub damage_bonus: i8,
    pub hit_bonus: i8,
    pub armor_class_bonus: i8,
    pub quantity: QuantityKind,
    /// Rolled when the item is created.
    pub initial_quantity: Dice,
    /// The largest stack retail merges into.
    pub maximum_quantity: u8,
    pub profession_mask: u16,
    pub race_mask: u32,
    pub gender_mask: u8,
    pub attribute_requirements: Vec<Requirement>,
    pub skill_requirements: Vec<Requirement>,
    pub value: u32,
    /// Tenths of a pound.
    pub weight: u16,
    /// Retail's "binds on equip" byte; the assay screen shows it means cursed.
    pub cursed: bool,
}

/// Every item definition, by id.
#[derive(Asset, TypePath, Debug, Default)]
pub struct ItemCatalog(pub Vec<ItemDefinition>);

impl ItemCatalog {
    pub fn get(&self, id: ItemId) -> Option<&ItemDefinition> {
        self.0.get(usize::from(id.0))
    }
}

/// A loot table: weighted items and how many to draw.
#[derive(Clone, Debug, Reflect)]
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
