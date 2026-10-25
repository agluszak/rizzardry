//! What characters are made of, as items and monsters refer to it: races,
//! professions, genders, attributes, skills and the body's hit locations.
//! Each is declared in retail's order, so its retail number is its position.

use enum_map::Enum;
use enumset::EnumSetType;
use num_enum::TryFromPrimitive;

#[derive(Debug, Enum, EnumSetType)]
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

#[derive(Debug, Enum, EnumSetType)]
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

#[derive(Debug, Enum, EnumSetType)]
#[enumset(repr = "u8")]
pub enum Gender {
    Male,
    Female,
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

/// Retail's 41 skills (`docs/research/characters.md` §2.5).
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

/// Where a blow lands on a character (strings 0x429..0x431).
#[derive(Debug, Enum, EnumSetType)]
pub enum BodyPart {
    Head,
    Body,
    Leg,
    Hand,
    Foot,
}
