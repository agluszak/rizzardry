//! Party members: attributes, skills, and derived statistics.
//! The tables come from the original executable.

pub mod creation;
pub mod progression;
pub mod resistance;

use bevy::prelude::*;
use enum_map::EnumMap;

use crate::content::characters::{Attribute, Realm, Skill};
pub use crate::content::characters::{Gender, Profession, Race};
use crate::content::items::ItemDefinition;

/// Stamina above which an exhausted member comes round.
const COMES_ROUND_ABOVE: u32 = 9;

/// Retail's conditions, mildest first. Death is not one of them here: a
/// member is dead exactly when their health is 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Condition {
    Drained,
    Diseased,
    Irritated,
    Nauseated,
    Slowed,
    Afraid,
    Poisoned,
    Silenced,
    Hexed,
    Infatuated,
    Insane,
    Blind,
    Turncoat,
    Webbed,
    Asleep,
    Paralyzed,
    Unconscious,
}

/// Whether a party member or monster can fight: alive and nothing worse than
/// webbed (retail `highest_condition < ASLEEP`).
pub fn can_fight(hp: u32, conditions: &[(Condition, Lasting)]) -> bool {
    hp != 0 && conditions.iter().all(|&(c, _)| c < Condition::Asleep)
}

/// Inflict a condition on a party member or a monster (retail
/// `InflictCharacterCondition`, `InflictMonsterCondition`): a longer
/// duration replaces a shorter one, never the reverse.
pub fn inflict(conditions: &mut Vec<(Condition, Lasting)>, condition: Condition, lasting: Lasting) {
    match conditions.iter_mut().find(|(c, _)| *c == condition) {
        Some((_, existing)) => {
            *existing = match (*existing, lasting) {
                (Lasting::Rounds(a), Lasting::Rounds(b)) => Lasting::Rounds(a.max(b)),
                _ => Lasting::Indefinite,
            }
        }
        None => conditions.push((condition, lasting)),
    }
}

/// Count timed conditions down by aging ticks, lifting those that run out
/// (retail `TickCharacterCondition`, `TickMonsterCondition`).
pub fn age(conditions: &mut Vec<(Condition, Lasting)>, ticks: u32) {
    conditions.retain_mut(|(_, lasting)| match lasting {
        Lasting::Rounds(left) if u32::from(*left) <= ticks => false,
        Lasting::Rounds(left) => {
            *left -= ticks as u16;
            true
        }
        Lasting::Indefinite => true,
    });
}

/// Whether any condition is timed, so that aging has work to do.
pub fn any_timed(conditions: &[(Condition, Lasting)]) -> bool {
    conditions.iter().any(|(_, lasting)| matches!(lasting, Lasting::Rounds(_)))
}

/// A hit that is not quiet may wake a sleeper: `SEN/2 + 50` percent (retail
/// `damageCharacter`, `MonsterReactsToBeingStruck`). Returns whether it woke.
pub fn wake_on_hit(
    conditions: &mut Vec<(Condition, Lasting)>,
    senses: u32,
    rng: &mut crate::game::rng::GameRng,
) -> bool {
    if !conditions.iter().any(|&(c, _)| c == Condition::Asleep) {
        return false;
    }
    if rng.below(100) < senses / 2 + 50 {
        conditions.retain(|&(c, _)| c != Condition::Asleep);
        return true;
    }
    false
}

/// How long a condition lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lasting {
    /// Combat rounds, which are also the 120-second aging ticks outside it.
    Rounds(u16),
    /// Until something lifts it, like exhaustion until stamina returns.
    Indefinite,
}

// Retail g_profession_skill_availability, ordinary and spellbook skills.
// Bit order is Profession's authored order; realm availability needs learned spells.
const SKILL_AVAILABILITY: [u16; 28] = [
    0x03bf, 0x000f, 0x006f, 0x146f, 0x6bbf, 0x7fff, 0x1f9f, 0x018f, 0x03bf, 0x7fff, 0x03a0, 0x00e0,
    0x0200, 0x02a0, 0x0060, 0x0008, 0x7fff, 0x7fff, 0x03ff, 0x0070, 0x7fff, 0x7fff, 0x7fff, 0x0100,
    0x5010, 0x1406, 0x1828, 0x3040,
];

/// A playable race's base attributes (retail `g_race_base_attributes`);
/// the five monster races have none.
fn race_attributes(race: Race) -> Option<EnumMap<Attribute, u8>> {
    let attributes = match race {
        Race::Human => [45, 45, 45, 45, 45, 45, 45],
        Race::Elf => [35, 50, 50, 35, 50, 45, 40],
        Race::Dwarf => [55, 30, 50, 60, 35, 35, 35],
        Race::Gnome => [35, 50, 40, 50, 50, 35, 45],
        Race::Hobbit => [40, 40, 30, 45, 55, 50, 50],
        Race::Faerie => [25, 55, 35, 30, 50, 60, 45],
        Race::Lizardman => [60, 25, 25, 70, 40, 50, 30],
        Race::Dracon => [55, 35, 30, 60, 50, 40, 30],
        Race::Felpurr => [40, 40, 30, 35, 50, 60, 50],
        Race::Rawulf => [40, 30, 55, 50, 40, 40, 50],
        Race::Mook => [50, 50, 25, 50, 35, 35, 55],
        Race::Trynnie | Race::TRang | Race::Umpani | Race::Rapax | Race::Android => return None,
    };
    Some(EnumMap::from_array(attributes))
}

/// What a profession asks and gives (retail `g_profession_table`).
struct ProfessionRow {
    /// Attribute minimums; 0 for none.
    minimums: EnumMap<Attribute, u8>,
    hp_factor: f32,
    experience: u32,
    bonus_skill: Skill,
    skills: &'static [Skill],
    /// Weight of its levels in a hand's physical score.
    physical: i32,
}

fn profession_row(profession: Profession) -> ProfessionRow {
    use Profession::*;
    use Skill::*;
    let (minimums, hp_factor, experience, bonus_skill, skills, physical): (
        [u8; 7],
        f32,
        u32,
        Skill,
        &'static [Skill],
        i32,
    ) = match profession {
        Fighter => (
            [55, 0, 0, 50, 50, 0, 0],
            9.0,
            1000,
            CloseCombat,
            &[RangedCombat, Sword, Axe, Shield],
            4,
        ),
        Lord => {
            ([55, 0, 55, 55, 50, 50, 0], 7.5, 1400, DualWeapons, &[CloseCombat, Sword, Dagger], 4)
        }
        Valkyrie => {
            ([50, 0, 55, 55, 50, 50, 0], 7.5, 1400, Polearm, &[CloseCombat, Mythology, Axe], 4)
        }
        Ranger => {
            ([50, 50, 0, 50, 55, 0, 55], 6.5, 1400, RangedCombat, &[Scouting, Bow, Mythology], 4)
        }
        Samurai => (
            [50, 55, 0, 50, 55, 55, 0],
            7.0,
            1400,
            Sword,
            &[CloseCombat, DualWeapons, CriticalStrike],
            4,
        ),
        Ninja => (
            [50, 50, 0, 50, 55, 55, 50],
            6.5,
            1600,
            CriticalStrike,
            &[CloseCombat, MartialArts, ThrowingAndSling, Stealth],
            4,
        ),
        Monk => (
            [0, 50, 50, 0, 50, 55, 55],
            6.5,
            1400,
            MartialArts,
            &[CloseCombat, CriticalStrike, StaffAndWand, Stealth],
            4,
        ),
        Rogue => (
            [0, 0, 0, 0, 55, 50, 50],
            6.0,
            1000,
            LocksAndTraps,
            &[Dagger, DualWeapons, Pickpocket, Stealth],
            3,
        ),
        Gadgeteer => (
            [45, 55, 0, 0, 60, 0, 55],
            5.5,
            1000,
            ModernWeapon,
            &[RangedCombat, Engineering, LocksAndTraps],
            3,
        ),
        Bard => {
            ([45, 50, 0, 0, 55, 0, 55], 6.0, 1000, Communication, &[Music, Mythology, Artifacts], 3)
        }
        Priest => (
            [0, 0, 60, 55, 0, 0, 0],
            5.0,
            1200,
            Divinity,
            &[MaceAndFlail, StaffAndWand, Communication],
            2,
        ),
        Alchemist => {
            ([0, 55, 0, 0, 60, 0, 0], 4.5, 1200, Alchemy, &[Mythology, ThrowingAndSling], 2)
        }
        Bishop => (
            [0, 55, 55, 0, 55, 0, 55],
            4.0,
            1600,
            Artifacts,
            &[Alchemy, Wizardry, Divinity, Psionics],
            2,
        ),
        Psionic => (
            [0, 55, 0, 0, 0, 0, 60],
            3.5,
            1200,
            Psionics,
            &[Communication, Mythology, MentalMagic],
            2,
        ),
        Mage => (
            [0, 60, 0, 0, 55, 0, 0],
            3.0,
            1200,
            Wizardry,
            &[FireMagic, WaterMagic, AirMagic, MentalMagic],
            2,
        ),
    };
    ProfessionRow {
        minimums: EnumMap::from_array(minimums),
        hp_factor,
        experience,
        bonus_skill,
        skills,
        physical,
    }
}

/// The two attributes a skill's base level averages (one for spellbooks
/// and expert skills).
pub(crate) fn governing(skill: Skill) -> (Attribute, Attribute) {
    use Attribute::*;
    use Skill::*;
    match skill {
        Sword | Axe | Polearm | MaceAndFlail | StaffAndWand | Shield => (Strength, Dexterity),
        Bow | ThrowingAndSling => (Dexterity, Strength),
        Dagger | ModernWeapon | Pickpocket | MartialArts => (Dexterity, Speed),
        LocksAndTraps | Stealth | Music | Alchemy => (Dexterity, Intelligence),
        Engineering => (Intelligence, Dexterity),
        Scouting | CloseCombat | RangedCombat | Mythology | Psionics => (Senses, Intelligence),
        Artifacts | Communication => (Intelligence, Senses),
        DualWeapons => (Dexterity, Senses),
        CriticalStrike => (Senses, Speed),
        Wizardry => (Intelligence, Intelligence),
        Divinity => (Piety, Piety),
        FireMagic | WaterMagic | AirMagic | EarthMagic | MentalMagic | DivineMagic => {
            (Intelligence, Piety)
        }
        PowerStrike => (Strength, Strength),
        PowerCast => (Intelligence, Intelligence),
        IronWill => (Piety, Piety),
        IronSkin => (Vitality, Vitality),
        Reflextion => (Dexterity, Dexterity),
        SnakeSpeed => (Speed, Speed),
        EagleEye => (Senses, Senses),
    }
}

/// How heavily a member is loaded (retail `load_category`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Load {
    #[default]
    None,
    Light,
    Medium,
    Heavy,
    Extreme,
}

impl Load {
    /// Initiative and to-hit lost (retail `CalcInitiative`, `CalcAttacks`).
    pub fn penalty(self) -> i32 {
        match self {
            Load::None => 0,
            Load::Light => 1,
            Load::Medium => 2,
            Load::Heavy => 4,
            Load::Extreme => 8,
        }
    }

    /// Lost from the attack-count score (retail `CalcAttacks`).
    pub fn attack_penalty(self) -> i32 {
        self.penalty() * 15
    }

    /// Armour lost (retail `CalcArmorClasses`).
    pub fn armour_penalty(self) -> i32 {
        match self {
            Load::None | Load::Light => 0,
            Load::Medium => 1,
            Load::Heavy => 2,
            Load::Extreme => 4,
        }
    }

    /// The percent added to load-scaled fatigue (retail `FatigueCharacter`).
    pub fn fatigue_percent(self) -> u32 {
        match self {
            Load::None => 0,
            Load::Light => 25,
            Load::Medium => 50,
            Load::Heavy => 100,
            Load::Extreme => 200,
        }
    }

    /// Fatigue scaled by load, minus a quarter for Slow or plus a quarter
    /// for Haste (retail `FatigueCharacter`).
    pub fn scale_fatigue(self, amount: u32, slowed: bool, hasted: bool) -> u32 {
        let percent = self.fatigue_percent() as i32
            + if slowed {
                -25
            } else if hasted {
                25
            } else {
                0
            };
        (amount as i32 + percent * amount as i32 / 100).max(0) as u32
    }

    /// How much more walking tires (retail `HandlePartyMovement`).
    pub fn movement_factor(self) -> f32 {
        match self {
            Load::None => 1.0,
            Load::Light => 1.25,
            Load::Medium => 1.5,
            Load::Heavy => 2.0,
            Load::Extreme => 3.0,
        }
    }
}

/// The load band of carrying `weight` with room for `capacity` (both in
/// tenths of a pound): under 50%, 70%, 85%, up to 100%, then beyond.
fn load_band(weight: u32, capacity: u32) -> Load {
    if capacity == 0 {
        return if weight == 0 { Load::None } else { Load::Extreme };
    }
    match weight * 100 / capacity {
        0..50 => Load::None,
        50..70 => Load::Light,
        70..85 => Load::Medium,
        85..=100 => Load::Heavy,
        _ => Load::Extreme,
    }
}

/// What disease did to a member ([`Vitals::sicken`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sickness {
    Condition(Condition),
    Weakened(Attribute),
    Health(u32),
    Stamina(u32),
}

/// An adventurer's identity, training and progression.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(MonsterKnowledge)]
pub struct Character {
    pub name: String,
    pub full_name: String,
    pub personality: crate::content::characters::Personality,
    pub voice: crate::content::characters::Voice,
    pub race: Race,
    pub gender: Gender,
    pub profession: Profession,
    pub original_profession: Profession,
    pub level: u8,
    pub profession_levels: EnumMap<Profession, u8>,
    pub experience: u32,
    /// The threshold actually passed, retaining the prices of earlier professions.
    pub experience_previous_goal: u32,
    pub attributes: EnumMap<Attribute, u8>,
    /// Points invested per skill.
    pub skill_points: EnumMap<Skill, u8>,
    /// Unspent spell picks carried forward to later character advancement.
    pub spell_picks: progression::SpellPicks,
    /// Successful practice towards each skill's next point (retail
    /// `practice_count`; eight make a point).
    pub skill_practice: EnumMap<Skill, u8>,
    /// The portrait's file stem (e.g. `hummf` for `data/portraits/small/shummf.sti`).
    pub portrait: String,
    /// Health the maximum has lost to draining and disease, kept until
    /// Drained is lifted (retail `hp_adjustment`).
    pub hp_drained: u32,
    /// Likewise stamina (retail `fatigue_penalty`).
    pub stamina_drained: u32,
}

/// Recognition history changes independently of profile, vitals or inventory.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterKnowledge {
    /// How many of each species the actor has seen, capped at 255.
    pub seen: std::collections::HashMap<u32, u8>,
}

impl Character {
    /// A minimal valid level 1 character (retail's NPC recipe): each
    /// attribute at the race's base or the profession's minimum, whichever
    /// is higher; the profession's skills seeded from their base levels;
    /// Vitals initialize separately. Only the eleven playable races have bases.
    pub fn new(name: &str, race: Race, gender: Gender, profession: Profession) -> Self {
        let row = profession_row(profession);
        let base = race_attributes(race).unwrap_or(EnumMap::from_fn(|_| 45));
        let attributes = EnumMap::from_fn(|a| base[a].max(row.minimums[a]));
        let mut profession_levels = EnumMap::default();
        profession_levels[profession] = 1;
        let mut character = Character {
            name: name.to_owned(),
            full_name: name.to_owned(),
            personality: default(),
            voice: default(),
            race,
            gender,
            profession,
            original_profession: profession,
            level: 1,
            profession_levels,
            experience: 0,
            experience_previous_goal: 0,
            attributes,
            skill_points: EnumMap::default(),
            spell_picks: default(),
            skill_practice: EnumMap::default(),
            portrait: String::new(),
            hp_drained: 0,
            stamina_drained: 0,
        };
        character.seed_starting_skills();
        character
    }

    /// Seed from the final starting attributes, shared by recipes and drafts.
    fn seed_starting_skills(&mut self) {
        let row = profession_row(self.profession);
        self.skill_points = EnumMap::default();
        let share = 60 / (1 + row.skills.len() as u32);
        for &skill in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
            self.skill_points[skill] = (self.skill_base(skill) * share / 100) as u8;
        }
    }

    /// Effective attribute: no modifiers are modelled yet, so the base,
    /// within retail's 1..=125.
    pub fn attribute(&self, attribute: Attribute) -> u32 {
        u32::from(self.attributes[attribute]).clamp(1, 125)
    }

    /// Whether retail currently permits this character to use a skill.
    /// Realm skills require a learned spell in that realm.
    pub fn skill_available(&self, skill: Skill) -> bool {
        let index = skill as usize;
        if index >= Skill::PowerStrike as usize {
            let attribute = Attribute::try_from((index - Skill::PowerStrike as usize) as u8)
                .expect("expert skills follow the attribute order");
            return self.attributes[attribute] >= 100;
        }
        if index >= Skill::FireMagic as usize {
            return false;
        }
        if SKILL_AVAILABILITY[index] & (1 << self.profession as usize) == 0 {
            return false;
        }
        let row = profession_row(self.profession);
        if skill == row.bonus_skill || row.skills.contains(&skill) {
            return true;
        }
        if index >= Skill::Wizardry as usize {
            let delayed = matches!(
                self.profession,
                Profession::Lord
                    | Profession::Valkyrie
                    | Profession::Ranger
                    | Profession::Samurai
                    | Profession::Ninja
                    | Profession::Monk
            );
            if delayed {
                return self.profession_levels[self.profession] > 4;
            }
        }
        true
    }

    /// Practise a skill `usage` times (retail `PracticeCharacterSkill`): each
    /// use counts with chance `(100 − points)·base/100` percent (halved for
    /// the expert skills, at least 1), and every eighth count is a point, up
    /// to 100. Returns whether the skill improved.
    pub fn practise(
        &mut self,
        skill: Skill,
        usage: u32,
        rng: &mut crate::game::rng::GameRng,
    ) -> bool {
        let index = skill as usize;
        let allowed = if index >= Skill::PowerStrike as usize {
            self.skill_available(skill)
        } else if index >= Skill::FireMagic as usize {
            !crate::game::magic::profession_books(self.profession).is_empty()
        } else {
            SKILL_AVAILABILITY[index] & (1 << self.profession as usize) != 0
        };
        if usage == 0 || !allowed {
            return false;
        }
        let mut improved = false;
        for _ in 0..usage {
            let points = u32::from(self.skill_points[skill]);
            if points >= 100 {
                break;
            }
            let mut threshold = (100 - points) * self.skill_base(skill) / 100;
            if index >= Skill::PowerStrike as usize {
                threshold /= 2;
            }
            if rng.below(100) < threshold.max(1) {
                self.skill_practice[skill] += 1;
                if self.skill_practice[skill] >= 8 {
                    self.skill_practice[skill] = 0;
                    self.skill_points[skill] += 1;
                    improved = true;
                }
            }
        }
        improved
    }

    pub fn bonus_skill(&self) -> Skill {
        profession_row(self.profession).bonus_skill
    }

    pub fn profession_skills(&self) -> &'static [Skill] {
        profession_row(self.profession).skills
    }

    /// Abilities conferred by the current profession and race.
    pub fn traits(&self) -> enumset::EnumSet<crate::content::characters::Trait> {
        self.profession_traits() | self.racial_traits()
    }

    pub fn profession_traits(&self) -> enumset::EnumSet<crate::content::characters::Trait> {
        use crate::content::characters::Trait::*;
        use Profession::*;
        let profession: &[crate::content::characters::Trait] = match self.profession {
            Fighter => &[StaminaRegeneration, Knockout, Berserk],
            Lord => &[HealthRegeneration],
            Valkyrie => &[CheatDeath],
            Ranger => &[RangedCriticals, Search],
            Samurai => &[Fearless, LightningStrike],
            Ninja => &[ThrownCriticals, ThrownAutoPenetrate],
            Monk => &[MonkDamageResistance, EffectiveWhileBlind],
            Rogue => &[Backstab],
            Gadgeteer => &[MergeGadgets],
            Bard => &[CampRecovery],
            Priest => &[Pray, TurnUndead],
            Alchemist => &[MakePotions],
            Bishop => &[RemoveCursedItems, TurnUndead],
            Psionic => &[Fearless, MentalConditionImmunity],
            Mage => &[MagicResistance],
        };
        profession.iter().copied().collect()
    }

    pub fn racial_traits(&self) -> enumset::EnumSet<crate::content::characters::Trait> {
        use crate::content::characters::Trait::*;
        let racial: &[crate::content::characters::Trait] = match self.race {
            Race::Dwarf => &[DwarfDamageResistance],
            Race::Faerie => &[
                FaerieArmour,
                UnusualStartingEquipment,
                FaerieCarryCapacity,
                EquippableItemWeightLimit,
                FastMagicRecovery,
            ],
            Race::Lizardman => &[LizardmanMagicRecovery],
            Race::Dracon => &[Breathe],
            Race::Android => &[AndroidConditionImmunity, CannotLearn],
            _ => &[],
        };
        racial.iter().copied().collect()
    }

    pub fn has_trait(&self, ability: crate::content::characters::Trait) -> bool {
        self.traits().contains(ability)
    }

    /// A trait's value by the level of the current profession: full past
    /// twenty levels, 60% at none and two points more a level (retail
    /// `ScaleValueByProfessionLevel`).
    pub fn scale_by_profession(&self, value: f32) -> f32 {
        let level = self.profession_levels[self.profession];
        if level > 20 { value } else { (f32::from(level) * 2.0 + 60.0) * value * 0.01 }
    }

    /// Percent taken off every blow that lands on them, without spells.
    pub fn damage_reduction(&self) -> i32 {
        self.base_stats().damage_reduction()
    }

    /// The average of a skill's governing attributes' bases.
    fn skill_base(&self, skill: Skill) -> u32 {
        let (a, b) = governing(skill);
        (u32::from(self.attributes[a]) + u32::from(self.attributes[b])) / 2
    }

    /// Effective skill level: the points, plus a quarter (at least one) for
    /// the profession's bonus skill.
    pub fn skill(&self, skill: Skill) -> u32 {
        let points = u32::from(self.skill_points[skill]);
        let bonus = if profession_row(self.profession).bonus_skill == skill {
            (points * 25 / 100).max(1)
        } else {
            0
        };
        (points + bonus).min(125)
    }

    /// `round(Σ (0.6 + 0.008·VIT) × factor × levels)`, counting one extra
    /// level for the original profession.
    pub fn max_hp(&self) -> u32 {
        self.base_stats().max_hp()
    }

    /// `round((STR + PIE + VIT) / 3 × (0.1·level + 2))`.
    pub fn max_stamina(&self) -> u32 {
        self.base_stats().max_stamina()
    }

    /// Levels weighted by how physical each profession is.
    pub fn physical_levels(&self) -> i32 {
        self.profession_levels
            .iter()
            .map(|(profession, &levels)| i32::from(levels) * profession_row(profession).physical)
            .sum()
    }

    /// Tenths of a pound: `(VIT + 2·STR) × 12`, × 8 for a Faerie.
    pub fn capacity(&self) -> u32 {
        self.base_stats().capacity()
    }

    /// The load band of carrying `weight` (own plus pool share).
    pub fn load(&self, weight: u32) -> Load {
        self.base_stats().load(weight)
    }

    /// `(level + 1)/2 + SEN/5 + SPD/5 − 10`, plus Snake Speed, less 1, 2, 4
    /// or 8 by load.
    pub fn initiative(&self, load: Load) -> i32 {
        self.base_stats().initiative(load)
    }

    /// The passed threshold plus this profession's next increment: doubling
    /// to level 10, then 20% growth. Prior professions keep their actual cost.
    pub fn experience_goal(&self) -> u32 {
        self.experience_previous_goal
            .saturating_add(progression::experience_increment(self.profession, self.level))
    }

    /// Whether the character may use an item (retail `CanCharacterUseItem`):
    /// profession, race and gender, and every attribute and skill minimum.
    /// Spellbooks and casting items' extra tests need spells, not modelled.
    pub fn can_use(&self, item: &ItemDefinition) -> bool {
        item.professions.contains(self.profession)
            && item.races.contains(self.race)
            && item.genders.contains(self.gender)
            && item.attribute_minimums.iter().all(|(a, &m)| self.attribute(a) >= u32::from(m))
            && item.skill_minimums.iter().all(|(s, &m)| self.skill(s) >= u32::from(m))
    }
}

/// A borrowed view of current attributes. No copy of the profile or temporary
/// modifiers is stored; creation uses base stats, gameplay supplies live effects.
#[derive(Clone, Copy)]
pub struct CharacterStats<'a> {
    profile: &'a Character,
    enchantments: Option<&'a crate::game::magic::enchantments::Enchantments>,
}
impl std::ops::Deref for CharacterStats<'_> {
    type Target = Character;
    fn deref(&self) -> &Character {
        self.profile
    }
}
impl Character {
    pub fn stats<'a>(
        &'a self,
        effects: &'a crate::game::magic::enchantments::Enchantments,
    ) -> CharacterStats<'a> {
        CharacterStats { profile: self, enchantments: Some(effects) }
    }
    pub fn base_stats(&self) -> CharacterStats<'_> {
        CharacterStats { profile: self, enchantments: None }
    }
}
impl CharacterStats<'_> {
    /// Percent taken off every blow that lands on them: a Dwarf's toughness,
    /// a Monk's training, Iron Skin and Body of Stone (retail
    /// `RecalculateCharacterState`).
    pub fn damage_reduction(&self) -> i32 {
        use crate::content::characters::Trait;
        let mut reduction = 0;
        if self.has_trait(Trait::DwarfDamageResistance) {
            reduction += self.attribute(Attribute::Vitality) as i32 / 10;
        }
        if self.has_trait(Trait::MonkDamageResistance) {
            reduction += self.scale_by_profession(15.0) as i32;
        }
        if self.skill_available(Skill::IronSkin) {
            reduction += (self.skill(Skill::IronSkin) >> 2) as i32 + 5;
        }
        reduction += self.enchantments.map_or(0, |e| e.damage_reduction() as i32);
        reduction.min(100)
    }
    pub fn attribute(&self, attribute: Attribute) -> u32 {
        let bonus = self.enchantments.map_or(0, |e| e.attribute_bonus(attribute));
        (self.profile.attribute(attribute) + bonus).min(125)
    }
    pub fn max_hp(&self) -> u32 {
        let vitality = 0.6 + 0.008 * self.attribute(Attribute::Vitality) as f32;
        let total: f32 = self
            .profession_levels
            .iter()
            .map(|(profession, &levels)| {
                let mut levels = u32::from(levels);
                if profession == self.original_profession && levels > 0 {
                    levels += 1;
                }
                vitality * profession_row(profession).hp_factor * levels as f32
            })
            .sum();
        (total.round() as u32).saturating_sub(self.hp_drained).max(1)
    }
    pub fn max_stamina(&self) -> u32 {
        let sum = [Attribute::Strength, Attribute::Piety, Attribute::Vitality]
            .map(|a| self.attribute(a))
            .iter()
            .sum::<u32>() as f32;
        ((sum / 3.0 * (0.1 * f32::from(self.level) + 2.0)).round() as u32)
            .saturating_sub(self.stamina_drained)
    }
    pub fn capacity(&self) -> u32 {
        let factor = if self.race == Race::Faerie { 8 } else { 12 };
        (self.attribute(Attribute::Vitality) + 2 * self.attribute(Attribute::Strength)) * factor
    }
    pub fn load(&self, weight: u32) -> Load {
        load_band(weight, self.capacity())
    }
    pub fn initiative(&self, load: Load) -> i32 {
        let mut initiative = (i32::from(self.level) + 1) / 2
            + self.attribute(Attribute::Senses) as i32 / 5
            + self.attribute(Attribute::Speed) as i32 / 5
            - 10;
        if self.skill_points[Skill::SnakeSpeed] > 0 {
            initiative += self.skill(Skill::SnakeSpeed) as i32 / 10 + 1;
        }
        initiative - load.penalty()
    }
}

/// Health, fatigue and conditions change independently of identity, training
/// and recognition. Recruitment initializes these from the actual profile.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Vitals {
    pub enchantments: crate::game::magic::enchantments::Enchantments,
    pub hp: u32,
    pub stamina: u32,
    pub conditions: Vec<(Condition, Lasting)>,
    /// Recovery not yet whole points; retail keeps these between ticks.
    pub hp_fraction: f32,
    pub stamina_fraction: f32,
    /// How hard a poison bites while the member is Poisoned (retail
    /// `condition_argument`): damage each aging tick, wearing off as the
    /// poison runs its course. It means nothing otherwise.
    pub poison: u32,
}

impl Vitals {
    pub fn new(profile: &Character) -> Self {
        Self { hp: profile.max_hp(), stamina: profile.max_stamina(), ..default() }
    }

    /// The share of stamina spent, in percent.
    pub fn stamina_spent(&self, maximum: u32) -> u32 {
        100 - self.stamina * 100 / maximum.max(1)
    }

    pub fn is_dead(&self) -> bool {
        self.hp == 0
    }

    pub fn has(&self, condition: Condition) -> bool {
        self.conditions.iter().any(|&(c, _)| c == condition)
    }

    /// The worst condition, which retail shows and which decides whether
    /// the member can act.
    pub fn worst_condition(&self) -> Option<Condition> {
        self.conditions.iter().map(|&(c, _)| c).max()
    }

    /// Alive and nothing worse than blind: able to act outside a combat
    /// round, such as to search, trade or rest (retail
    /// `highest_condition < TURNCOAT`).
    pub fn can_act(&self) -> bool {
        !self.is_dead() && self.worst_condition().is_none_or(|c| c < Condition::Turncoat)
    }

    /// Alive and nothing worse than webbed: able to take or be the target of
    /// a combat action (retail `highest_condition < ASLEEP`).
    pub fn can_fight(&self) -> bool {
        can_fight(self.hp, &self.conditions)
    }

    /// Inflict a condition, keeping the longer duration if already afflicted.
    pub fn inflict(&mut self, condition: Condition, lasting: Lasting) {
        let lasting = if condition == Condition::Slowed {
            if let Lasting::Rounds(turns) = lasting {
                let remaining = self.enchantments.counter_slow(u32::from(turns));
                if remaining == 0 {
                    return;
                }
                Lasting::Rounds(remaining as u16)
            } else {
                self.enchantments.0.retain(|e| e.spell != crate::content::spells::SpellId::HASTE);
                lasting
            }
        } else {
            lasting
        };
        inflict(&mut self.conditions, condition, lasting);
    }

    /// Poison the member for `lasting` at `strength` (retail
    /// `SetCharacterCondition` with a poison strength): a fresh poisoning
    /// bites as hard as it is, a further one only ever harder. No strength,
    /// no poisoning. Whether it took.
    pub fn poison(&mut self, lasting: Lasting, strength: u32) -> bool {
        if strength == 0 {
            return false;
        }
        if !self.has(Condition::Poisoned) {
            self.poison = 0;
        }
        self.inflict(Condition::Poisoned, lasting);
        self.poison = self.poison.max(strength);
        true
    }

    pub fn lift(&mut self, condition: Condition) {
        self.conditions.retain(|&(c, _)| c != condition);
    }

    /// Count timed conditions down, lifting those that run out (retail
    /// `TickCharacterCondition`). A poison still running weakens by its
    /// share of what is left of it, a roll deciding a share under one.
    pub fn age_conditions(&mut self, ticks: u32, rng: &mut crate::game::rng::GameRng) {
        let left = self.conditions.iter().find_map(|&(c, lasting)| match lasting {
            Lasting::Rounds(left) if c == Condition::Poisoned => Some(u32::from(left)),
            _ => None,
        });
        if let Some(left) = left.filter(|&left| left > ticks) {
            let mut lost = self.poison * ticks / left;
            if lost == 0 && rng.below(100) < self.poison * ticks * 100 / left {
                lost = 1;
            }
            self.poison -= lost.min(self.poison);
        }
        age(&mut self.conditions, ticks);
        self.enchantments.age(ticks);
    }

    /// The damage poison does over `ticks` aging ticks (retail
    /// `GameTurnsPassedChar`'s damage per minute): its strength each tick,
    /// half again unless the party stands still (retail `wait_state` 3,
    /// outside a fight).
    pub fn poison_damage(&self, ticks: u32, still: bool) -> u32 {
        if !self.has(Condition::Poisoned) {
            return 0;
        }
        let damage = self.poison * ticks;
        if still { damage } else { damage + damage / 2 }
    }

    /// Take damage (retail `damageCharacter`): two thirds of it also drain
    /// stamina, and health reaching 0 is death. The dead take none. Returns
    /// the health lost. Guardian Angel absorbs before fatigue. The caller
    /// handles waking; exceptional survival traits remain unported.
    pub fn damage(&mut self, amount: u32) -> u32 {
        if self.is_dead() {
            return 0;
        }
        let amount = self.enchantments.absorb(amount);
        self.spend_stamina(amount * 2 / 3);
        let lost = amount.min(self.hp);
        self.hp -= lost;
        if self.is_dead() {
            self.die();
        }
        lost
    }

    fn die(&mut self) {
        self.enchantments.0.clear();
        // Death clears every condition but infatuation (retail `Die`).
        self.conditions.retain(|&(c, _)| c == Condition::Infatuated);
        self.stamina = 0;
    }

    /// Drain health for good (retail `DamageCharacter`, which draining blows,
    /// breaths and Vampiric Vortex use): two thirds of it tires, the maximum
    /// loses it until Drained is lifted, and present health falls with the
    /// maximum; none left is death. Guardian Angel does not stop it. Returns
    /// the health lost.
    pub fn drain_health(&mut self, profile: &mut Character, amount: u32) -> u32 {
        if self.is_dead() || amount == 0 {
            return 0;
        }
        self.spend_stamina(amount * 2 / 3);
        let before = profile.stats(&self.enchantments).max_hp();
        profile.hp_drained = profile.hp_drained.saturating_add(amount);
        let after = profile.stats(&self.enchantments).max_hp();
        let lost = (before - after).min(self.hp);
        self.hp -= lost;
        if self.is_dead() {
            self.die();
        }
        if !self.has(Condition::Drained) {
            self.inflict(Condition::Drained, Lasting::Indefinite);
        }
        lost
    }

    /// Lift Drained (retail `RemoveCharacterCondition`): the maxima get back
    /// what draining and disease took, and the pools rise with them. Whether
    /// it was there to lift.
    pub fn lift_drain(&mut self, profile: &mut Character) -> bool {
        let Some(index) = self.conditions.iter().position(|&(c, _)| c == Condition::Drained) else {
            return false;
        };
        self.conditions.remove(index);
        let before = profile.stats(&self.enchantments);
        let (hp, stamina) = (before.max_hp(), before.max_stamina());
        profile.hp_drained = 0;
        profile.stamina_drained = 0;
        let after = profile.stats(&self.enchantments);
        if !self.is_dead() {
            self.hp += after.max_hp() - hp;
            self.stamina += after.max_stamina() - stamina;
        }
        true
    }

    /// What disease does as `minutes` pass (retail `GameTurnsPassedChar`):
    /// a roll under the minutes out of ten times Vitality strikes one of four
    /// ways. Retail draws its condition from five but names a sixth,
    /// Paralyzed, that the draw never reaches.
    pub fn sicken(
        &mut self,
        profile: &mut Character,
        minutes: u32,
        rng: &mut crate::game::rng::GameRng,
    ) -> Option<Sickness> {
        if !self.has(Condition::Diseased) {
            return None;
        }
        let vitality = profile.stats(&self.enchantments).attribute(Attribute::Vitality);
        if rng.below(vitality * 10) >= minutes {
            return None;
        }
        match rng.below(4) {
            0 => {
                let condition = [
                    Condition::Irritated,
                    Condition::Nauseated,
                    Condition::Slowed,
                    Condition::Blind,
                    Condition::Insane,
                ][rng.below(5) as usize];
                self.inflict(condition, Lasting::Indefinite);
                Some(Sickness::Condition(condition))
            }
            1 => {
                use enum_map::Enum;
                let attribute = Attribute::from_usize(rng.below(7) as usize);
                (profile.attributes[attribute] > 1).then(|| {
                    profile.attributes[attribute] -= 1;
                    Sickness::Weakened(attribute)
                })
            }
            2 => {
                if profile.stats(&self.enchantments).max_hp() < 5 {
                    return None;
                }
                let loss = rng.below(4) + 1;
                profile.hp_drained += loss;
                if self.hp > 1 {
                    self.hp -= loss.min(self.hp - 1);
                }
                Some(Sickness::Health(loss))
            }
            _ => {
                if profile.stats(&self.enchantments).max_stamina() < 5 {
                    return None;
                }
                let loss = rng.below(4) + 1;
                profile.stamina_drained += loss;
                if self.stamina > 1 {
                    self.stamina -= loss.min(self.stamina - 1);
                }
                Some(Sickness::Stamina(loss))
            }
        }
    }

    /// Spend stamina; running out knocks the member unconscious until it
    /// comes back.
    pub fn spend_stamina(&mut self, amount: u32) {
        if self.stamina == 0 || self.is_dead() || amount == 0 {
            return;
        }
        self.stamina -= amount.min(self.stamina);
        if self.stamina == 0 {
            self.inflict(Condition::Unconscious, Lasting::Indefinite);
        }
    }

    /// Restore stamina up to its maximum; an exhausted member comes round
    /// once it is above 9.
    pub fn restore_stamina(&mut self, amount: u32, maximum: u32) {
        if self.is_dead() {
            return;
        }
        self.stamina = (self.stamina + amount).min(maximum);
        let exhausted = self.conditions.contains(&(Condition::Unconscious, Lasting::Indefinite));
        if exhausted && self.stamina > COMES_ROUND_ABOVE {
            self.lift(Condition::Unconscious);
        }
    }

    /// Heal up to the maximum. Healing does not raise the dead.
    pub fn heal(&mut self, amount: u32, maximum: u32) {
        if !self.is_dead() {
            self.hp = (self.hp + amount).min(maximum);
        }
    }

    /// Passive stamina recovery over `ticks` 1.4-second stamina ticks, at
    /// `(0.9·max + 20)/240` per tick times `scale` (retail
    /// `RegenCharacterStamina`).
    pub fn recover_stamina(&mut self, ticks: u32, scale: f32, maximum: u32) {
        let dead = self.is_dead();
        if let Some(whole) =
            regenerate(dead, self.stamina, &mut self.stamina_fraction, ticks, scale, maximum, 0.9)
        {
            self.restore_stamina(whole, maximum);
            if self.stamina >= maximum {
                self.stamina_fraction = 0.0;
            }
        }
    }

    /// Passive healing over `ticks` 120-second aging ticks, at
    /// `(0.4·max + 20)/240` per tick times `scale` (retail aging).
    pub fn recover_health(&mut self, ticks: u32, scale: f32, maximum: u32) {
        let dead = self.is_dead();
        if let Some(whole) =
            regenerate(dead, self.hp, &mut self.hp_fraction, ticks, scale, maximum, 0.4)
        {
            self.heal(whole, maximum);
            if self.hp >= maximum {
                self.hp_fraction = 0.0;
            }
        }
    }
}

/// Regeneration over `ticks` at `(factor·max + 20)/240` a tick times
/// `scale`: the whole points now due, carrying the rest in `fraction`; none,
/// with the fraction dropped, for the dead, at no rate, or when full.
fn regenerate(
    dead: bool,
    value: u32,
    fraction: &mut f32,
    ticks: u32,
    scale: f32,
    max: u32,
    factor: f32,
) -> Option<u32> {
    if dead || scale <= 0.0 || value >= max {
        *fraction = 0.0;
        return None;
    }
    *fraction += ticks as f32 * (factor * max as f32 + 20.0) / 240.0 * scale;
    let whole = fraction.trunc();
    *fraction -= whole;
    Some(whole as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Attribute::*;
    use crate::content::items::EquipClass;
    use enumset::EnumSet;

    /// Practice counts towards a point every eighth success, only in skills
    /// the profession may train, and never past 100 (retail
    /// `PracticeCharacterSkill`).
    #[test]
    fn practice_raises_a_point_every_eight_successes() {
        let mut rng = crate::game::rng::GameRng::seeded(4);
        let mut mage = Character::new("Mage", Race::Human, Gender::Male, Profession::Mage);
        let before = mage.skill_points[Skill::FireMagic];
        assert!(mage.practise(Skill::FireMagic, 400, &mut rng));
        assert!(mage.skill_points[Skill::FireMagic] > before);
        let mut fighter = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        assert!(!fighter.practise(Skill::FireMagic, 400, &mut rng), "no books, no realm practice");
        mage.skill_points[Skill::FireMagic] = 100;
        assert!(!mage.practise(Skill::FireMagic, 400, &mut rng));
        assert_eq!(mage.skill_points[Skill::FireMagic], 100);
    }

    #[test]
    fn a_human_fighter_has_the_worked_example_numbers() {
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        assert_eq!(fighter.attributes, EnumMap::from_array([55, 45, 45, 50, 50, 45, 45]));
        assert_eq!(fighter.max_hp(), 18);
        assert_eq!(fighter.max_stamina(), 105);
        assert_eq!(fighter.capacity(), 1920);
        assert_eq!(fighter.initiative(Load::None), 9);
        assert_eq!(fighter.initiative(Load::Heavy), 5);
        assert_eq!(fighter.load(959), Load::None);
        assert_eq!(fighter.load(960), Load::Light);
        assert_eq!(fighter.load(1939), Load::Heavy, "integer percent: still 100");
        assert_eq!(fighter.load(1940), Load::Extreme);
        assert_eq!(fighter.experience_goal(), 1000);
        // Profession and bonus skills are seeded; others are not.
        let close_combat = fighter.skill_points[Skill::CloseCombat];
        assert!(fighter.skill(Skill::CloseCombat) > close_combat.into());
        assert!(fighter.skill_points[Skill::Sword] > 0);
        assert_eq!(fighter.skill_points[Skill::LocksAndTraps], 0);
    }

    #[test]
    fn skill_availability_follows_profession_levels_and_attribute_mastery() {
        let mut fighter = Character::new("F", Race::Human, Gender::Male, Profession::Fighter);
        assert!(fighter.skill_available(Skill::Sword));
        assert!(!fighter.skill_available(Skill::Music));
        fighter.skill_points[Skill::CloseCombat] = 0;
        assert_eq!(fighter.skill(Skill::CloseCombat), 1);
        assert!(!fighter.skill_available(Skill::PowerStrike));
        fighter.attributes[Strength] = 100;
        assert!(fighter.skill_available(Skill::PowerStrike));
        let mut samurai = Character::new("S", Race::Human, Gender::Male, Profession::Samurai);
        assert!(!samurai.skill_available(Skill::Wizardry));
        samurai.profession_levels[Profession::Samurai] = 5;
        assert!(samurai.skill_available(Skill::Wizardry));
        assert!(!samurai.skill_available(Skill::FireMagic));
    }

    #[test]
    fn experience_goals_double_then_grow_by_a_fifth() {
        let mut mage = Character::new("Mira", Race::Elf, Gender::Female, Profession::Mage);
        mage.level = 3;
        mage.experience_previous_goal = 2400;
        assert_eq!(mage.experience_goal(), 1200 + 1200 + 2400);
        mage.level = 11;
        mage.experience_previous_goal = 1200 * (1 + 511);
        // Level 1 is one weight; levels 2 to 10 add 1, 2, ..., 256 weights.
        let at_ten = 1200 * (1 + 511);
        assert_eq!(mage.experience_goal(), at_ten + 256 * 1200 * 12 / 10);
    }

    #[test]
    fn items_check_profession_race_gender_and_minimums() {
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut bow = ItemDefinition {
            name: "Strong Bow".into(),
            class: EquipClass::RangedWeapon,
            weapon_skill: Some(Skill::Bow),
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..default()
        };
        bow.attribute_minimums[Strength] = 85;

        assert!(!fighter.can_use(&bow), "too weak");
        bow.attribute_minimums[Strength] = 0;
        assert!(fighter.can_use(&bow));
        bow.genders = EnumSet::only(Gender::Female);
        assert!(!fighter.can_use(&bow), "female only");
        bow.genders = EnumSet::all();
        bow.professions = EnumSet::only(Profession::Mage);
        assert!(!fighter.can_use(&bow));
    }

    #[test]
    fn damage_tires_knocks_out_and_kills() {
        let profile = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut fighter = Vitals::new(&profile);
        assert_eq!(fighter.damage(6), 6);
        assert_eq!((fighter.hp, fighter.stamina), (12, 101));
        fighter.spend_stamina(200);
        assert!(fighter.has(Condition::Unconscious));
        fighter.restore_stamina(9, profile.max_stamina());
        assert!(fighter.has(Condition::Unconscious), "not above 9 yet");
        fighter.restore_stamina(1, profile.max_stamina());
        assert!(!fighter.has(Condition::Unconscious));
        assert_eq!(fighter.damage(100), 12);
        assert!(fighter.is_dead());
        assert!(fighter.conditions.is_empty() && fighter.stamina == 0);
        assert_eq!(fighter.damage(5), 0);
        fighter.heal(10, profile.max_hp());
        assert!(fighter.is_dead(), "healing does not raise the dead");
    }

    #[test]
    fn rest_recovers_stamina_quickly_and_health_slowly() {
        let profile = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut fighter = Vitals::new(&profile);
        fighter.hp = 1;
        fighter.stamina = 0;
        fighter.inflict(Condition::Unconscious, Lasting::Indefinite);
        // (0.9 × 105 + 20) / 240 × 0.5 ≈ 0.24 a tick: 10 above 9 after 42.
        fighter.recover_stamina(41, 0.5, profile.max_stamina());
        assert!(fighter.has(Condition::Unconscious));
        fighter.recover_stamina(1, 0.5, profile.max_stamina());
        assert_eq!(fighter.stamina, 10);
        assert!(!fighter.has(Condition::Unconscious));
        // (0.4 × 18 + 20) / 240 × 0.5 ≈ 0.057 a tick.
        fighter.recover_health(17, 0.5, profile.max_hp());
        assert_eq!(fighter.hp, 1);
        fighter.recover_health(1, 0.5, profile.max_hp());
        assert_eq!(fighter.hp, 2);
        fighter.recover_health(1, 0.0, profile.max_hp());
        assert_eq!(fighter.hp_fraction, 0.0, "no recovery clears the fraction");
    }

    #[test]
    fn timed_conditions_run_out() {
        let profile = Character::new("Mira", Race::Elf, Gender::Female, Profession::Mage);
        let mut mage = Vitals::new(&profile);
        mage.inflict(Condition::Asleep, Lasting::Rounds(3));
        mage.inflict(Condition::Unconscious, Lasting::Indefinite);
        mage.age_conditions(2, &mut crate::game::rng::GameRng::default());
        assert_eq!(
            mage.conditions,
            vec![
                (Condition::Asleep, Lasting::Rounds(1)),
                (Condition::Unconscious, Lasting::Indefinite)
            ]
        );
        mage.age_conditions(1, &mut crate::game::rng::GameRng::default());
        assert_eq!(mage.conditions, vec![(Condition::Unconscious, Lasting::Indefinite)]);
    }
}
