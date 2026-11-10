//! What characters are made of, as items and monsters refer to it: races,
//! professions, genders, attributes, skills and the body's hit locations.
//! Each is declared in retail's order, so its retail number is its position.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::LazyLock;

use enum_map::{Enum, EnumMap};
use enumset::{EnumSet, EnumSetType};
use num_enum::TryFromPrimitive;
use serde::{Deserialize, Deserializer};

use crate::content::items::ItemId;
use crate::content::spells::Spellbook;

#[derive(Debug, Hash, Enum, EnumSetType, Deserialize)]
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

#[derive(Debug, Hash, Enum, EnumSetType, Deserialize)]
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
#[derive(Debug, EnumSetType, Deserialize)]
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

#[derive(Debug, Hash, Enum, EnumSetType, TryFromPrimitive, Deserialize)]
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
#[derive(Debug, Enum, EnumSetType, TryFromPrimitive, Deserialize)]
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
#[derive(Debug, Hash, Enum, EnumSetType, TryFromPrimitive, Deserialize)]
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

/// What a profession is: one row of `assets/tables/professions.ron`, whose
/// head says what each field means.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfessionRow {
    #[serde(default, deserialize_with = "sparse")]
    pub minimums: EnumMap<Attribute, u32>,
    pub hit_points: f32,
    pub experience: u64,
    pub bonus_skill: Skill,
    pub skills: Vec<Skill>,
    pub physical: i32,
    #[serde(deserialize_with = "set")]
    pub open_skills: EnumSet<Skill>,
    #[serde(default, deserialize_with = "set")]
    pub traits: EnumSet<Trait>,
    #[serde(default, deserialize_with = "set")]
    pub books: EnumSet<Spellbook>,
    #[serde(default)]
    pub caster_offset: Option<i32>,
    #[serde(default)]
    pub picks_from_any_book: bool,
    #[serde(default)]
    pub only_gender: Option<Gender>,
    #[serde(default)]
    pub resistance: i32,
    pub starting_items: Vec<ItemId>,
    #[serde(default)]
    pub extra_item: Option<ExtraItem>,
}

/// The one item a new member of a profession carries beyond its set.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum ExtraItem {
    Always(ItemId),
    /// `then` if the member trains `skill` further than `over`, otherwise
    /// `otherwise`.
    ByTrainedSkill {
        skill: Skill,
        over: Skill,
        then: ItemId,
        otherwise: ItemId,
    },
}

/// What a race is: one row of `assets/tables/races.ron`, whose head says what
/// each field means.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaceRow {
    #[serde(default, deserialize_with = "sparse_all")]
    pub attributes: Option<EnumMap<Attribute, u32>>,
    #[serde(default, deserialize_with = "sparse")]
    pub resistance: EnumMap<Realm, Option<Resist>>,
    #[serde(default, deserialize_with = "set")]
    pub traits: EnumSet<Trait>,
    #[serde(default)]
    pub armour: i32,
    #[serde(default = "RaceRow::usual_carry_factor")]
    pub carry_factor: u32,
    #[serde(default)]
    pub starting_items: Option<Vec<ItemId>>,
}

impl RaceRow {
    fn usual_carry_factor() -> u32 {
        12
    }
}

/// What a race adds to a realm's resistance.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Resist {
    Flat(i32),
    /// A d-th of vitality.
    OfVitality(i32),
}

static PROFESSIONS: LazyLock<EnumMap<Profession, ProfessionRow>> = LazyLock::new(|| {
    rows(include_str!("../../assets/tables/professions.ron"))
        .expect("assets/tables/professions.ron is a table of professions")
});

static RACES: LazyLock<EnumMap<Race, RaceRow>> = LazyLock::new(|| {
    rows(include_str!("../../assets/tables/races.ron"))
        .expect("assets/tables/races.ron is a table of races")
});

impl Profession {
    /// What this profession is, from the table the game ships.
    pub fn row(self) -> &'static ProfessionRow {
        &PROFESSIONS[self]
    }
}

impl Race {
    /// What this race is, from the table the game ships.
    pub fn row(self) -> &'static RaceRow {
        &RACES[self]
    }
}

/// A table with a row for each of `K`, named as `K` names them.
fn rows<K, V>(text: &str) -> Result<EnumMap<K, V>, ron::error::SpannedError>
where
    K: Enum + Hash + Eq + for<'de> Deserialize<'de>,
    V: for<'de> Deserialize<'de>,
{
    let mut rows: HashMap<K, V> = ron::from_str(text)?;
    Ok(EnumMap::from_fn(|key| rows.remove(&key).expect("a row for every key")))
}

/// A map of the keys that have a value, the rest the default.
fn sparse<'de, D, K, V>(deserializer: D) -> Result<EnumMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Enum + Hash + Eq + Deserialize<'de>,
    V: Default + Deserialize<'de>,
{
    let mut given = HashMap::<K, V>::deserialize(deserializer)?;
    Ok(EnumMap::from_fn(|key| given.remove(&key).unwrap_or_default()))
}

/// The same, for a value that is all there or not at all.
fn sparse_all<'de, D, K, V>(deserializer: D) -> Result<Option<EnumMap<K, V>>, D::Error>
where
    D: Deserializer<'de>,
    K: Enum + Hash + Eq + Deserialize<'de>,
    V: Default + Deserialize<'de>,
{
    let given = Option::<HashMap<K, V>>::deserialize(deserializer)?;
    Ok(given.map(|mut given| EnumMap::from_fn(|key| given.remove(&key).unwrap_or_default())))
}

/// A set written as a list.
fn set<'de, D, T>(deserializer: D) -> Result<EnumSet<T>, D::Error>
where
    D: Deserializer<'de>,
    T: EnumSetType + Deserialize<'de>,
{
    Ok(Vec::<T>::deserialize(deserializer)?.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::ItemId;

    /// Every profession and every race has a row: the tables parse whole.
    #[test]
    fn every_profession_and_race_has_a_row() {
        for profession in EnumSet::<Profession>::all() {
            assert!(profession.row().hit_points > 0.0, "{profession:?}");
            assert!(
                profession.row().open_skills.contains(profession.row().bonus_skill)
                    || profession.row().bonus_skill as usize >= Skill::FireMagic as usize,
                "{profession:?}"
            );
        }
        let playable = EnumSet::<Race>::all().iter().filter(|r| r.row().attributes.is_some());
        assert_eq!(playable.count(), 11);
    }

    #[test]
    fn a_fighter_is_a_hardy_axe_or_sword_fighter_without_spells() {
        let fighter = Profession::Fighter.row();
        assert_eq!(fighter.minimums[Attribute::Strength], 55);
        assert_eq!(fighter.minimums[Attribute::Intelligence], 0);
        assert_eq!((fighter.hit_points, fighter.experience, fighter.physical), (9.0, 1000, 4));
        assert_eq!(fighter.bonus_skill, Skill::CloseCombat);
        assert_eq!(fighter.skills, [Skill::RangedCombat, Skill::Sword, Skill::Axe, Skill::Shield]);
        assert_eq!(fighter.traits, Trait::StaminaRegeneration | Trait::Knockout | Trait::Berserk);
        assert!(fighter.books.is_empty() && fighter.caster_offset.is_none());
        assert_eq!(fighter.starting_items, [ItemId(143), ItemId(153), ItemId(203), ItemId(246)]);
        assert!(matches!(
            fighter.extra_item,
            Some(ExtraItem::ByTrainedSkill {
                skill: Skill::Axe,
                over: Skill::Sword,
                then: ItemId(18),
                otherwise: ItemId(7)
            })
        ));
    }

    #[test]
    fn the_casting_professions_differ_in_books_and_offsets() {
        assert_eq!(Profession::Mage.row().books, Spellbook::Wizardry);
        assert_eq!(Profession::Mage.row().caster_offset, Some(0));
        assert_eq!(Profession::Mage.row().resistance, 5);
        assert_eq!(Profession::Lord.row().books, Spellbook::Divinity);
        assert_eq!(Profession::Lord.row().caster_offset, Some(-4));
        assert_eq!(Profession::Bishop.row().books, EnumSet::<Spellbook>::all());
        assert!(Profession::Bishop.row().picks_from_any_book);
        assert!(!Profession::Priest.row().picks_from_any_book);
        assert_eq!(Profession::Valkyrie.row().only_gender, Some(Gender::Female));
        assert_eq!(Profession::Fighter.row().only_gender, None);
    }

    #[test]
    fn a_race_starts_with_its_scores_and_adds_to_what_it_resists() {
        let human = Race::Human.row();
        assert_eq!(human.attributes, Some(EnumMap::from_fn(|_| 45)));
        assert_eq!((human.armour, human.carry_factor), (0, 12));
        let faerie = Race::Faerie.row();
        assert_eq!(faerie.attributes.unwrap()[Attribute::Speed], 60);
        assert_eq!((faerie.armour, faerie.carry_factor), (2, 8));
        assert_eq!(faerie.starting_items.as_deref().unwrap().len(), 6);
        assert!(matches!(faerie.resistance[Realm::Air], Some(Resist::Flat(15))));
        assert!(faerie.resistance[Realm::Fire].is_none());
        assert!(matches!(Race::Dwarf.row().resistance[Realm::Fire], Some(Resist::OfVitality(5))));
        assert!(matches!(Race::Lizardman.row().resistance[Realm::Divine], Some(Resist::Flat(-10))));
        assert!(Race::Android.row().attributes.is_none());
        assert_eq!(
            Race::Android.row().traits,
            Trait::AndroidConditionImmunity | Trait::CannotLearn
        );
    }
}
