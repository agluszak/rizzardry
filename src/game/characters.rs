//! Party members: attributes, skills, and derived statistics.
//! The tables come from the original executable.

pub mod creation;
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

/// How heavily a member is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Load {
    None,
    Light,
    Medium,
    Heavy,
    Extreme,
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
    pub attributes: EnumMap<Attribute, u8>,
    /// Points invested per skill.
    pub skill_points: EnumMap<Skill, u8>,
    /// Unspent spell picks carried forward to later character advancement.
    pub unspent_spell_picks: u8,
    /// Successful practice towards each skill's next point (retail
    /// `practice_count`; eight make a point).
    pub skill_practice: EnumMap<Skill, u8>,
    /// The portrait's file stem (e.g. `hummf` for `data/portraits/small/shummf.sti`).
    pub portrait: String,
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
            attributes,
            skill_points: EnumMap::default(),
            unspent_spell_picks: 0,
            skill_practice: EnumMap::default(),
            portrait: String::new(),
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
                FaerieAbility23,
                FaerieCarryCapacity,
                FaerieAbility25,
                FaerieAbility26,
            ],
            Race::Lizardman => &[LizardmanMagicRecovery],
            Race::Dracon => &[Breathe],
            Race::Android => &[AndroidAbility30, CannotLearn],
            _ => &[],
        };
        racial.iter().copied().collect()
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
        (total.round() as u32).max(1)
    }

    /// `round((STR + PIE + VIT) / 3 × (0.1·level + 2))`.
    pub fn max_stamina(&self) -> u32 {
        let sum = [Attribute::Strength, Attribute::Piety, Attribute::Vitality]
            .map(|a| self.attribute(a))
            .iter()
            .sum::<u32>() as f32;
        (sum / 3.0 * (0.1 * f32::from(self.level) + 2.0)).round() as u32
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
        let factor = if self.race == Race::Faerie { 8 } else { 12 };
        (self.attribute(Attribute::Vitality) + 2 * self.attribute(Attribute::Strength)) * factor
    }

    /// The load band of carrying `weight` (own plus pool share).
    pub fn load(&self, weight: u32) -> Load {
        load_band(weight, self.capacity())
    }

    /// `(level + 1)/2 + SEN/5 + SPD/5 − 10`, plus Snake Speed, less 1, 2, 4
    /// or 8 by load.
    pub fn initiative(&self, load: Load) -> i32 {
        let mut initiative = (i32::from(self.level) + 1) / 2
            + self.attribute(Attribute::Senses) as i32 / 5
            + self.attribute(Attribute::Speed) as i32 / 5
            - 10;
        if self.skill_points[Skill::SnakeSpeed] > 0 {
            initiative += self.skill(Skill::SnakeSpeed) as i32 / 10 + 1;
        }
        initiative
            - match load {
                Load::None => 0,
                Load::Light => 1,
                Load::Medium => 2,
                Load::Heavy => 4,
                Load::Extreme => 8,
            }
    }

    /// Experience needed for the next level (retail goals): the weight at
    /// level 1, doubling increments to level 10, then 20% more each level.
    pub fn experience_goal(&self) -> u32 {
        let weight = profession_row(self.profession).experience;
        let mut goal = weight;
        let mut step = weight;
        for level in 2..=u32::from(self.level) {
            step = if level <= 10 { weight << (level - 2) } else { step * 12 / 10 };
            goal += step;
        }
        goal
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

/// Health, fatigue and conditions change independently of identity, training
/// and recognition. Recruitment initializes these from the actual profile.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Vitals {
    pub hp: u32,
    pub stamina: u32,
    pub conditions: Vec<(Condition, Lasting)>,
    /// Recovery not yet whole points; retail keeps these between ticks.
    pub hp_fraction: f32,
    pub stamina_fraction: f32,
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

    /// Inflict a condition, keeping the longer duration if already afflicted.
    pub fn inflict(&mut self, condition: Condition, lasting: Lasting) {
        inflict(&mut self.conditions, condition, lasting);
    }

    pub fn lift(&mut self, condition: Condition) {
        self.conditions.retain(|&(c, _)| c != condition);
    }

    /// Count timed conditions down, lifting those that run out (retail
    /// `AgeCondition`). Poison's weakening is not modelled yet.
    pub fn age_conditions(&mut self, ticks: u32) {
        age(&mut self.conditions, ticks);
    }

    /// Take damage (retail `damageCharacter`): two thirds of it also drain
    /// stamina, and health reaching 0 is death. The dead take none. Returns
    /// the health lost. Guardian Angel, cheating death and waking the asleep
    /// are not modelled yet.
    pub fn damage(&mut self, amount: u32) -> u32 {
        if self.is_dead() {
            return 0;
        }
        self.spend_stamina(amount * 2 / 3);
        let lost = amount.min(self.hp);
        self.hp -= lost;
        if self.is_dead() {
            // Death clears every condition but infatuation (retail `Die`).
            self.conditions.retain(|&(c, _)| c == Condition::Infatuated);
            self.stamina = 0;
        }
        lost
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
        let max = maximum;
        if self.is_dead() || scale <= 0.0 || self.stamina >= max {
            self.stamina_fraction = 0.0;
            return;
        }
        self.stamina_fraction += ticks as f32 * (0.9 * max as f32 + 20.0) / 240.0 * scale;
        let whole = self.stamina_fraction.trunc();
        self.stamina_fraction -= whole;
        self.restore_stamina(whole as u32, maximum);
        if self.stamina >= max {
            self.stamina_fraction = 0.0;
        }
    }

    /// Passive healing over `ticks` 120-second aging ticks, at
    /// `(0.4·max + 20)/240` per tick times `scale` (retail aging).
    pub fn recover_health(&mut self, ticks: u32, scale: f32, maximum: u32) {
        let max = maximum;
        if self.is_dead() || scale <= 0.0 || self.hp >= max {
            self.hp_fraction = 0.0;
            return;
        }
        self.hp_fraction += ticks as f32 * (0.4 * max as f32 + 20.0) / 240.0 * scale;
        let whole = self.hp_fraction.trunc();
        self.hp_fraction -= whole;
        self.heal(whole as u32, maximum);
        if self.hp >= max {
            self.hp_fraction = 0.0;
        }
    }
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
        assert_eq!(mage.experience_goal(), 1200 + 1200 + 2400);
        mage.level = 11;
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
        mage.age_conditions(2);
        assert_eq!(
            mage.conditions,
            vec![
                (Condition::Asleep, Lasting::Rounds(1)),
                (Condition::Unconscious, Lasting::Indefinite)
            ]
        );
        mage.age_conditions(1);
        assert_eq!(mage.conditions, vec![(Condition::Unconscious, Lasting::Indefinite)]);
    }
}
