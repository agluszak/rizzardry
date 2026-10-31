//! What characters are made of, as items and monsters refer to it: races,
//! professions, genders, attributes, skills and the body's hit locations.
//! Each is declared in retail's order, so its retail number is its position.

use enum_map::Enum;
use enumset::EnumSetType;
use num_enum::TryFromPrimitive;
use serde::Deserialize;

#[derive(Debug, Enum, EnumSetType, Deserialize)]
#[enumset(repr = "u32")]
pub enum Race {
    Human,
    Elf,
    Dwarf,
    Gnome,
    Hobbit,
    Faerie,
    Lizardman,
    Dracon,
    Felpurr,
    Rawulf,
    Mook,
    Trynnie,
    TRang,
    Umpani,
    Rapax,
    Android,
}

#[derive(Debug, Enum, EnumSetType, Deserialize)]
#[enumset(repr = "u16")]
pub enum Profession {
    Fighter,
    Lord,
    Valkyrie,
    Ranger,
    Samurai,
    Ninja,
    Monk,
    Rogue,
    Gadgeteer,
    Bard,
    Priest,
    Alchemist,
    Bishop,
    Psionic,
    Mage,
}

#[derive(Debug, Enum, EnumSetType, Deserialize)]
#[enumset(repr = "u8")]
pub enum Gender {
    Male,
    Female,
}

/// Retail personality and the two voice variations chosen at creation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Personality {
    #[default]
    Aggressive,
    Intellectual,
    Burly,
    Chaotic,
    Cunning,
    Eccentric,
    Kindly,
    Laidback,
    Loner,
}

impl Personality {
    pub const ALL: [Self; 9] = [
        Self::Aggressive,
        Self::Intellectual,
        Self::Burly,
        Self::Chaotic,
        Self::Cunning,
        Self::Eccentric,
        Self::Kindly,
        Self::Laidback,
        Self::Loner,
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Voice {
    #[default]
    First,
    Second,
}

/// Profession and racial abilities, in retail's description order.
#[derive(Debug, EnumSetType)]
#[enumset(repr = "u32")]
#[repr(u8)]
pub enum Trait {
    StaminaRegeneration = 0,
    HealthRegeneration = 1,
    CheatDeath = 2,
    Fearless = 3,
    RangedCriticals = 4,
    ThrownCriticals = 5,
    MonkDamageResistance = 6,
    EffectiveWhileBlind = 7,
    MergeGadgets = 8,
    Backstab = 9,
    CampRecovery = 10,
    Pray = 11,
    Search = 12,
    RemoveCursedItems = 13,
    MentalConditionImmunity = 14,
    MagicResistance = 15,
    LightningStrike = 16,
    TurnUndead = 17,
    MakePotions = 18,
    Knockout = 19,
    Berserk = 20,
    ThrownAutoPenetrate = 21,
    FaerieArmour = 22,
    UnusualStartingEquipment = 23,
    FaerieCarryCapacity = 24,
    EquippableItemWeightLimit = 25,
    /// Retail trait 0x1a multiplies spell recovery in GameplayTime.cpp.
    FastMagicRecovery = 26,
    LizardmanMagicRecovery = 27,
    Breathe = 28,
    DwarfDamageResistance = 29,
    /// Immune to fear, disease, irritation, nausea, poison and sleep.
    AndroidConditionImmunity = 30,
    CannotLearn = 31,
}

#[derive(Debug, Enum, EnumSetType, TryFromPrimitive)]
#[repr(u8)]
pub enum Attribute {
    Strength,
    Intelligence,
    Piety,
    Vitality,
    Dexterity,
    Speed,
    Senses,
}

/// Retail's 41 skills.
#[derive(Debug, Enum, EnumSetType, TryFromPrimitive)]
#[repr(u8)]
pub enum Skill {
    Sword,
    Axe,
    Polearm,
    MaceAndFlail,
    Dagger,
    StaffAndWand,
    Shield,
    ModernWeapon,
    Bow,
    ThrowingAndSling,
    LocksAndTraps,
    Stealth,
    Music,
    Pickpocket,
    MartialArts,
    Scouting,
    CloseCombat,
    RangedCombat,
    DualWeapons,
    CriticalStrike,
    Artifacts,
    Mythology,
    Communication,
    Engineering,
    Wizardry,
    Divinity,
    Alchemy,
    Psionics,
    FireMagic,
    WaterMagic,
    AirMagic,
    EarthMagic,
    MentalMagic,
    DivineMagic,
    PowerStrike,
    PowerCast,
    IronWill,
    IronSkin,
    Reflextion,
    SnakeSpeed,
    EagleEye,
}

/// The six magical realms, in retail's resistance and spell order.
#[derive(Debug, Enum, EnumSetType, TryFromPrimitive)]
#[repr(u8)]
pub enum Realm {
    Fire,
    Water,
    Air,
    Earth,
    Mental,
    Divine,
}

impl Realm {
    /// The skill casting in this realm trains.
    pub fn skill(self) -> Skill {
        match self {
            Realm::Fire => Skill::FireMagic,
            Realm::Water => Skill::WaterMagic,
            Realm::Air => Skill::AirMagic,
            Realm::Earth => Skill::EarthMagic,
            Realm::Mental => Skill::MentalMagic,
            Realm::Divine => Skill::DivineMagic,
        }
    }
}

impl Skill {
    /// The realm this skill casts in, if it is a realm skill.
    pub fn realm(self) -> Option<Realm> {
        enumset::EnumSet::<Realm>::all().iter().find(|realm| realm.skill() == self)
    }
}

/// Where a blow lands on a character (strings 0x429..0x431).
#[derive(Debug, Enum, EnumSetType)]
pub enum BodyPart {
    Head,
    Body,
    Leg,
    Hand,
    Foot,
}
