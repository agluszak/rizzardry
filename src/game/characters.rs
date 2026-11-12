//! Party members: attributes, skills, and derived statistics.
//! The tables come from the original executable.

pub mod creation;
pub mod progression;
pub mod resistance;

use bevy::prelude::*;
use enum_map::EnumMap;

use crate::content::characters::{Attribute, Realm, Skill};
pub use crate::content::characters::{Gender, Profession, Race};
use crate::game::actors::Attributes;
use crate::game::rng::GameRng;

/// Stamina above which an exhausted member comes round.
pub(crate) const COMES_ROUND_ABOVE: u32 = 9;

pub use crate::content::conditions::Condition;

/// A value by a level: full past twenty levels, 60% at none and two points
/// more a level (retail `ScaleValueByProfessionLevel` and
/// `ScaleValueByMonsterLevel`, one formula).
pub fn scale_by_level(level: u8, value: f32) -> f32 {
    if level > 20 { value } else { (f32::from(level) * 2.0 + 60.0) * value * 0.01 }
}

/// What ails a party member or a monster: each condition at most once, with
/// how long it lasts. Its own component, beside the body's [`Vitals`]: what
/// follows from a condition coming or going (the figures it changes, what
/// the body is fit for) is noticed from its change ([`settle`]), never said
/// by whoever changed it.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Conditions {
    list: Vec<(Condition, Lasting)>,
    /// How hard a poison bites while Poisoned (retail `condition_argument`):
    /// damage each aging tick, wearing off as the poison runs its course.
    /// It means nothing otherwise.
    pub poison: u32,
}

impl Conditions {
    /// The rows of `conditions.ron` for what ails the body now.
    pub fn modifiers(&self) -> impl Iterator<Item = &crate::content::conditions::Modifier> {
        crate::content::conditions::MODIFIERS.iter().filter(|row| self.has(row.condition))
    }

    /// What the conditions add to every attack's to-hit.
    pub fn to_hit(&self) -> i32 {
        self.modifiers().map(|row| row.hit).sum()
    }

    /// What the conditions add to armour.
    pub fn armour(&self) -> i32 {
        self.modifiers().map(|row| row.armour).sum()
    }

    /// Whether what ails a body with these traits keeps it out of its
    /// formation place (retail `out_of_formation`).
    pub fn unseats(&self, traits: enumset::EnumSet<crate::content::characters::Trait>) -> bool {
        self.modifiers().any(|row| {
            row.unseats
                && !row.eased_by.as_ref().is_some_and(|eased| traits.contains(eased.ability))
        })
    }

    pub fn has(&self, condition: Condition) -> bool {
        self.list.iter().any(|&(c, _)| c == condition)
    }

    /// How long `condition` lasts, if it is there.
    pub fn lasting(&self, condition: Condition) -> Option<Lasting> {
        self.list.iter().find_map(|&(c, lasting)| (c == condition).then_some(lasting))
    }

    /// How long `condition` lasts, to shorten it in place.
    fn lasting_mut(&mut self, condition: Condition) -> Option<&mut Lasting> {
        self.list.iter_mut().find_map(|(c, lasting)| (*c == condition).then_some(lasting))
    }

    /// Out cold until stamina returns: spent stamina's unconsciousness,
    /// which lasts indefinitely.
    pub fn exhausted(&self) -> bool {
        self.lasting(Condition::Unconscious) == Some(Lasting::Indefinite)
    }

    /// Spent stamina leaves one exhausted (retail `FatigueCharacter`,
    /// `FatigueMonster`).
    pub(crate) fn exhaust(&mut self) {
        self.afflict(Condition::Unconscious, Lasting::Indefinite);
    }

    /// The worst condition, which retail shows and whose rank its rules
    /// compare (`highest_condition`).
    pub fn worst(&self) -> Option<Condition> {
        self.list.iter().map(|&(c, _)| c).max()
    }

    /// Whether nothing is as bad as `condition` (retail
    /// `highest_condition < condition`).
    pub fn below(&self, condition: Condition) -> bool {
        self.worst().is_none_or(|worst| worst < condition)
    }

    /// Inflict a condition (retail `InflictCharacterCondition`,
    /// `InflictMonsterCondition`): a longer duration replaces a shorter one,
    /// never the reverse.
    pub fn afflict(&mut self, condition: Condition, lasting: Lasting) {
        match self.list.iter_mut().find(|(c, _)| *c == condition) {
            Some((_, existing)) => {
                *existing = match (*existing, lasting) {
                    (Lasting::Rounds(a), Lasting::Rounds(b)) => Lasting::Rounds(a.max(b)),
                    _ => Lasting::Indefinite,
                }
            }
            None => self.list.push((condition, lasting)),
        }
    }

    pub fn lift(&mut self, condition: Condition) {
        self.list.retain(|&(c, _)| c != condition);
    }

    /// Lift every condition but infatuation, which only its own cure ends
    /// (retail `Die`, `RemoveAllConditionsFromParty`).
    pub fn lift_all(&mut self) {
        self.list.retain(|&(c, _)| matches!(c, Condition::Infatuated | Condition::Missing));
    }

    /// Count timed conditions down by aging ticks, lifting those that run
    /// out (retail `TickCharacterCondition`, `TickMonsterCondition`), the
    /// poison weakening as it goes.
    pub fn age(&mut self, ticks: u32, rng: &mut GameRng) {
        self.weaken_poison(ticks, rng);
        self.list.retain_mut(|(_, lasting)| match lasting {
            Lasting::Rounds(left) if *left <= ticks => false,
            Lasting::Rounds(left) => {
                *left -= ticks;
                true
            }
            Lasting::Indefinite => true,
        });
    }

    /// Whether any condition is timed, so that aging has work to do.
    pub fn any_timed(&self) -> bool {
        self.list.iter().any(|(_, lasting)| matches!(lasting, Lasting::Rounds(_)))
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Condition, Lasting)> {
        self.list.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// A timed condition has only `left` to run, as a partial cure leaves it.
    pub fn shorten(&mut self, condition: Condition, left: u32) {
        if let Some(lasting) = self.lasting_mut(condition) {
            *lasting = Lasting::Rounds(left);
        }
    }

    /// A hit that is not quiet may wake a sleeper: `SEN/2 + 50` percent
    /// (retail `damageCharacter`, `MonsterReactsToBeingStruck`). Whether it
    /// woke.
    pub fn wake_on_hit(&mut self, senses: u32, rng: &mut GameRng) -> bool {
        let wakes = self.has(Condition::Asleep) && rng.below(100) < senses / 2 + 50;
        if wakes {
            self.lift(Condition::Asleep);
        }
        wakes
    }

    /// Haste, as a spell to lay for `turns`, first spends itself on a
    /// slowing: the turns left to lay, none against a slowing without end.
    pub fn hasted(&mut self, turns: u32) -> u32 {
        match self.lasting_mut(Condition::Slowed) {
            None => turns,
            Some(Lasting::Indefinite) => 0,
            Some(Lasting::Rounds(left)) => {
                let cancelled = turns.min(*left);
                *left -= cancelled;
                if *left == 0 {
                    self.lift(Condition::Slowed);
                }
                turns - cancelled
            }
        }
    }

    /// Poison the body for `lasting` at `strength` (retail
    /// `SetCharacterCondition` with a poison strength): a fresh poisoning
    /// bites as hard as it is, a further one only ever harder.
    pub fn poison(&mut self, lasting: Lasting, strength: u32) {
        if !self.has(Condition::Poisoned) {
            self.poison = 0;
        }
        self.afflict(Condition::Poisoned, lasting);
        self.poison = self.poison.max(strength);
    }

    /// A poison that `ticks` take off without ending it weakens by their
    /// share of what is left of it, a roll deciding a share under one
    /// (retail `TickCharacterCondition`, for aging and partial cures alike).
    /// The caller counts the poison's duration down.
    pub fn weaken_poison(&mut self, ticks: u32, rng: &mut GameRng) {
        let left = match self.lasting(Condition::Poisoned) {
            Some(Lasting::Rounds(left)) => Some(left),
            _ => None,
        };
        if let Some(left) = left.filter(|&left| left > ticks) {
            let mut lost = self.poison * ticks / left;
            if lost == 0 && rng.below(100) < self.poison * ticks * 100 / left {
                lost = 1;
            }
            self.poison -= lost.min(self.poison);
        }
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
}

impl<const N: usize> From<[(Condition, Lasting); N]> for Conditions {
    fn from(conditions: [(Condition, Lasting); N]) -> Self {
        conditions.into_iter().collect()
    }
}

impl FromIterator<(Condition, Lasting)> for Conditions {
    fn from_iter<I: IntoIterator<Item = (Condition, Lasting)>>(conditions: I) -> Self {
        let mut all = Self::default();
        for (condition, lasting) in conditions {
            all.afflict(condition, lasting);
        }
        all
    }
}

/// How long a condition lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lasting {
    /// Combat rounds, which are also the 120-second aging ticks outside it.
    Rounds(u32),
    /// Until something lifts it, like exhaustion until stamina returns.
    Indefinite,
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
        (amount as i32 + percent * amount as i32 / 100) as u32
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

/// A pool's value once its maximum moves from `old` to `new`: it moves by
/// as much, stopping at 0.
pub fn resized(current: u32, old: u32, new: u32) -> u32 {
    (current + new).max(old) - old
}

/// What disease did to a member ([`Vitals::sicken`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sickness {
    Condition(Condition),
    Weakened(Attribute),
    Health(u32),
    Stamina(u32),
}
/// The highest total level (retail `RecalculateCharacterDerivedStats`
/// clamps the total and each profession's level to it).
pub const MAX_LEVEL: u8 = 50;

/// An adventurer's identity, training and progression.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(
    MonsterKnowledge,
    Tally,
    crate::game::items::Wears,
    crate::game::items::Pack,
    crate::game::items::Burden,
    crate::game::magic::LearnedSpells,
    crate::game::magic::SpellPoints,
    crate::game::party::Order
)]
pub struct Character {
    pub name: String,
    pub full_name: String,
    pub personality: crate::content::characters::Personality,
    pub voice: crate::content::characters::Voice,
    pub race: Race,
    pub gender: Gender,
    pub profession: Profession,
    pub original_profession: Profession,
    /// Levels gained in each profession; the total level is their sum.
    pub profession_levels: EnumMap<Profession, u8>,
    pub experience: u64,
    /// The threshold actually passed, retaining the prices of earlier professions.
    pub experience_previous_goal: u64,
    pub attributes: EnumMap<Attribute, u32>,
    /// Points invested per skill.
    pub skill_points: EnumMap<Skill, u32>,
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

/// What a character has done and suffered: the foes they have killed in
/// fights, and the times they have died (retail `kill_count`,
/// `death_count`). Apart from the sheet, which their figures are made from.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub kills: u32,
    pub deaths: u32,
}

/// Recognition history changes independently of profile, vitals or inventory.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterKnowledge {
    /// How many of each species the actor has seen, capped at 255.
    pub seen: std::collections::HashMap<u32, u32>,
}

impl Character {
    /// The total level: every level gained in any profession (retail
    /// `uiExpLevel`, which every advancement raises with the profession's
    /// own level). Advancement stops at [`MAX_LEVEL`].
    pub fn level(&self) -> u8 {
        self.profession_levels.values().sum()
    }

    /// A minimal valid level 1 character (retail's NPC recipe): each
    /// attribute at the race's base or the profession's minimum, whichever
    /// is higher; the profession's skills seeded from their base levels;
    /// Vitals initialize separately. Only the eleven playable races have bases.
    pub fn new(name: &str, race: Race, gender: Gender, profession: Profession) -> Self {
        let minimums = &profession.row().minimums;
        let base = race.row().attributes.unwrap_or(EnumMap::from_fn(|_| 45));
        let attributes = EnumMap::from_fn(|a| base[a].max(minimums[a]));
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
        let row = self.profession.row();
        self.skill_points = EnumMap::default();
        let share = 60 / (1 + row.skills.len() as u32);
        for &skill in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
            self.skill_points[skill] = self.skill_base(skill) * share / 100;
        }
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
        let row = self.profession.row();
        if !row.open_skills.contains(skill) {
            return false;
        }
        if skill == row.bonus_skill || row.skills.contains(&skill) {
            return true;
        }
        // A hybrid's book skills open with its caster level.
        if index >= Skill::Wizardry as usize && row.caster_offset.is_some_and(|offset| offset < 0) {
            return self.profession_levels[self.profession] > 4;
        }
        true
    }

    /// Practise a skill `usage` times (retail `PracticeCharacterSkill`): each
    /// use counts with chance `(100 − points)·base/100` percent (halved for
    /// the expert skills, at least 1), and every eighth count is a point, up
    /// to 100. Returns whether the skill improved.
    pub fn practise(&mut self, skill: Skill, usage: u32, rng: &mut GameRng) -> bool {
        let index = skill as usize;
        let allowed = if index >= Skill::PowerStrike as usize {
            self.skill_available(skill)
        } else if index >= Skill::FireMagic as usize {
            !self.profession.row().books.is_empty()
        } else {
            self.profession.row().open_skills.contains(skill)
        };
        if usage == 0 || !allowed {
            return false;
        }
        let mut improved = false;
        for _ in 0..usage {
            let points = self.skill_points[skill];
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

    /// Abilities conferred by the current profession and race.
    pub fn traits(&self) -> enumset::EnumSet<crate::content::characters::Trait> {
        self.profession.row().traits | self.race.row().traits
    }

    pub fn has_trait(&self, ability: crate::content::characters::Trait) -> bool {
        self.traits().contains(ability)
    }

    /// A trait's value by the level of the current profession: full past
    /// twenty levels, 60% at none and two points more a level (retail
    /// `ScaleValueByProfessionLevel`).
    pub fn scale_by_profession(&self, value: f32) -> f32 {
        scale_by_level(self.profession_levels[self.profession], value)
    }

    /// The average of a skill's governing attributes' bases.
    fn skill_base(&self, skill: Skill) -> u32 {
        let (a, b) = governing(skill);
        (self.attributes[a] + self.attributes[b]) / 2
    }

    /// Levels weighted by how physical each profession is.
    pub fn physical_levels(&self) -> i32 {
        self.profession_levels
            .iter()
            .map(|(profession, &levels)| i32::from(levels) * profession.row().physical)
            .sum()
    }

    /// Add earned experience to the character.
    pub fn award_experience(&mut self, amount: u32) {
        self.experience += amount as u64;
    }

    /// The passed threshold plus this profession's next increment: doubling
    /// to level 10, then 20% growth. Prior professions keep their actual cost.
    pub fn experience_goal(&self) -> u64 {
        self.experience_previous_goal
            + progression::experience_increment(self.profession, self.level())
    }
}

impl Character {
    /// Percent taken off every blow that lands on them: a Dwarf's toughness,
    /// a Monk's training and Iron Skin (retail `RecalculateCharacterState`).
    /// Body of Stone adds its own where armour is refitted.
    pub fn damage_reduction(&self, a: &Attributes, skills: &crate::game::actors::Skills) -> i32 {
        use crate::content::characters::Trait;
        let mut reduction = 0;
        if self.has_trait(Trait::DwarfDamageResistance) {
            reduction += a[Attribute::Vitality] as i32 / 10;
        }
        if self.has_trait(Trait::MonkDamageResistance) {
            reduction += self.scale_by_profession(15.0) as i32;
        }
        if self.skill_available(Skill::IronSkin) {
            reduction += (skills[Skill::IronSkin] >> 2) as i32 + 5;
        }
        reduction
    }
    /// `round(Σ (0.6 + 0.008·VIT) × factor × levels)`, counting one extra
    /// level for the original profession.
    pub fn max_hp(&self, a: &Attributes) -> u32 {
        let vitality = 0.6 + 0.008 * a[Attribute::Vitality] as f32;
        let total: f32 = self
            .profession_levels
            .iter()
            .map(|(profession, &levels)| {
                let mut levels = u32::from(levels);
                if profession == self.original_profession && levels > 0 {
                    levels += 1;
                }
                vitality * profession.row().hit_points * levels as f32
            })
            .sum();
        (total.round() as u32).max(self.hp_drained + 1) - self.hp_drained
    }
    /// `round((STR + PIE + VIT) / 3 × (0.1·level + 2))`.
    pub fn max_stamina(&self, a: &Attributes) -> u32 {
        let sum = (a[Attribute::Strength] + a[Attribute::Piety] + a[Attribute::Vitality]) as f32;
        let total = (sum / 3.0 * (0.1 * self.level() as f32 + 2.0)).round() as u32;
        total.max(self.stamina_drained) - self.stamina_drained
    }
    /// Tenths of a pound: `(VIT + 2·STR)` times the race's carry factor.
    pub fn capacity(&self, a: &Attributes) -> u32 {
        (a[Attribute::Vitality] + 2 * a[Attribute::Strength]) * self.race.row().carry_factor
    }
    /// The load band of carrying `weight` (own plus pool share).
    pub fn load(&self, a: &Attributes, weight: u32) -> Load {
        load_band(weight, self.capacity(a))
    }
    /// `(level + 1)/2 + SEN/5 + SPD/5 − 10`, plus Snake Speed, less 1, 2, 4
    /// or 8 by load.
    pub fn initiative(
        &self,
        a: &Attributes,
        load: Load,
        skills: &crate::game::actors::Skills,
    ) -> i32 {
        let mut initiative = (i32::from(self.level()) + 1) / 2
            + a[Attribute::Senses] as i32 / 5
            + a[Attribute::Speed] as i32 / 5
            - 10;
        if self.skill_points[Skill::SnakeSpeed] > 0 {
            initiative += skills[Skill::SnakeSpeed] as i32 / 10 + 1;
        }
        initiative - load.penalty()
    }
}

/// Health or stamina: what is left and the most there can be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub current: u32,
    pub max: u32,
}

impl Pool {
    pub fn full(max: u32) -> Self {
        Self { current: max, max }
    }

    /// The share spent, in percent.
    pub fn spent_percent(&self) -> u32 {
        100 - self.current * 100 / self.max.max(1)
    }

    /// Move to a new maximum, the pool moving by as much and stopping at 0
    /// (retail `RecalculateCharacterHitPoints`,
    /// `RecalculateCharacterStamina`).
    fn follow(&mut self, max: u32) {
        self.current = resized(self.current, self.max, max);
        self.max = max;
    }
}

/// The body of a party member or a monster: its health and stamina. What
/// ails it is its [`Conditions`], and what it is fit for the markers
/// [`settle`] keeps on it. A monster's maxima
/// were rolled when it was made; a member's are written when they are
/// refitted.
#[derive(Component, Clone, Debug, Default, PartialEq)]
#[require(
    Conditions,
    // A body is made fit for everything; what ails it takes that away.
    crate::game::actors::Lucid,
    crate::game::actors::CanGuard,
    crate::game::actors::CanAct,
    crate::game::actors::CanFight,
    crate::game::actors::fitness::Present,
    crate::game::world::hostility::Side,
    crate::game::world::hostility::Loyalty,
    crate::game::actors::Attributes,
    crate::game::actors::Skills,
    crate::game::actors::Level,
    crate::game::actors::Resistances,
    crate::game::actors::Armour,
    crate::game::actors::Attacks,
    crate::game::actors::Mending
)]
pub struct Vitals {
    pub health: Pool,
    pub stamina: Pool,
    /// The fight in which a Valkyrie last cheated death, which only the
    /// first time in a fight does at full strength.
    pub cheated_death_in: Option<Entity>,
    /// The health the body had before whatever ran it out, which a
    /// Valkyrie's later cheats of death in a fight roll under (retail
    /// `CheatDeathRevive` reads it before the blow lands).
    pub fell_from: u32,
    /// Who ran its health out, if someone did (retail `condition_target`),
    /// written by whatever ran it out and read as its death is settled.
    fell_to: Option<Entity>,
    /// What a member has regained towards whole points.
    pub regeneration: Regeneration,
}

impl Vitals {
    /// A member as their profile makes them, rested and well.
    pub fn new(profile: &Character) -> Self {
        let a = Attributes::of_sheet(profile);
        Self::full(profile.max_hp(&a), profile.max_stamina(&a))
    }

    /// Full health and stamina at these maxima.
    pub fn full(health: u32, stamina: u32) -> Self {
        Self { health: Pool::full(health), stamina: Pool::full(stamina), ..default() }
    }

    pub fn is_dead(&self) -> bool {
        self.health.current == 0
    }

    /// A member's maxima are these now, their gear, sheet or spells having
    /// moved them (a refit): what is left stays as it is, held to the new
    /// maximum.
    pub fn refit_maxima(&mut self, health: u32, stamina: u32) {
        for (pool, max) in [(&mut self.health, health), (&mut self.stamina, stamina)] {
            pool.max = max;
            pool.current = pool.current.min(max);
        }
    }

    /// Raised from the dead with this much health and stamina.
    pub fn raise(&mut self, health: u32, stamina: u32) {
        self.health.current = health;
        self.stamina.current = stamina;
    }

    /// Take `amount` health that got past any Guardian Angel: two thirds
    /// of it tire, and health falls by it, to nothing at most (retail
    /// `ApplyDamageToCharacter`, `ApplyDamageToMonster`). The dead take
    /// none. Health run out is death once it is settled
    /// ([`fall`]). Returns the health lost. The caller handles
    /// waking. `by` is whoever does it, who answers for a death it brings:
    /// nobody for a poison, a cloud or a fall.
    pub fn hurt(&mut self, amount: u32, by: Option<Entity>) -> u32 {
        if self.is_dead() {
            return 0;
        }
        self.tire(amount * 2 / 3);
        let lost = amount.min(self.health.current);
        self.health.current -= lost;
        if self.is_dead() {
            self.fell_from = lost;
            self.fell_to = by;
        }
        lost
    }

    /// Die outright, as death effects inflict it (retail
    /// `SetCharacterCondition` with Dead): no damage, absorption or fatigue
    /// comes first. The health lost; death itself is settled as any other.
    pub fn slay(&mut self, by: Option<Entity>) -> u32 {
        let had = self.health.current;
        self.health.current = 0;
        if had > 0 {
            self.fell_from = had;
            self.fell_to = by;
        }
        had
    }

    /// The Valkyrie's trait (retail `CheatDeathRevive`) keeps her from death
    /// unless she is already seven rounds or more out cold: a stint
    /// unconscious, shorter the higher her level, and health back. The first
    /// time in a fight it is half to all of her maximum, scaled; any other
    /// time, a scaled roll of the health she had before the blow (what the
    /// blow took back). Whether she cheated it.
    pub fn cheats_death(
        &mut self,
        conditions: &mut Conditions,
        profile: &Character,
        fight: Option<Entity>,
        rng: &mut GameRng,
    ) -> bool {
        use crate::content::characters::Trait;
        if !profile.has_trait(Trait::CheatDeath) {
            return false;
        }
        let rounds = match conditions.lasting(Condition::Unconscious) {
            None => 0,
            Some(Lasting::Rounds(rounds)) => rounds,
            Some(Lasting::Indefinite) => return false,
        };
        if rounds >= 7 {
            return false;
        }
        let shortened = profile.scale_by_profession(5.0) as u32;
        let stint = (rounds + 7).max(shortened) - shortened;
        if stint > 0 {
            conditions.afflict(Condition::Unconscious, Lasting::Rounds(stint));
        }
        let maximum = self.health.max;
        self.health.current = match fight {
            Some(fight) if self.cheated_death_in != Some(fight) => {
                self.cheated_death_in = Some(fight);
                let share = rng.below(profile.scale_by_profession(maximum as f32 * 0.5) as u32);
                (share + 50) * maximum / 100
            }
            _ => rng.below(profile.scale_by_profession(self.fell_from as f32) as u32) + 1,
        };
        true
    }

    /// Drain a member's health for good (retail `DamageCharacter`, which
    /// draining blows, breaths and Vampiric Vortex use): two thirds of it
    /// tires, the maximum loses it until Drained is lifted, and present
    /// health falls with the maximum; none left is death. Guardian Angel
    /// does not stop it. Returns the health lost.
    pub fn drain_health(
        &mut self,
        conditions: &mut Conditions,
        profile: &mut Character,
        amount: u32,
        by: Option<Entity>,
    ) -> u32 {
        if self.is_dead() || amount == 0 {
            return 0;
        }
        self.tire(amount * 2 / 3);
        let had = self.health.current;
        profile.hp_drained += amount;
        self.health.follow(self.health.max.max(amount + 1) - amount);
        if self.is_dead() {
            self.fell_from = had;
            self.fell_to = by;
        }
        if !conditions.has(Condition::Drained) {
            conditions.afflict(Condition::Drained, Lasting::Indefinite);
        }
        had - self.health.current
    }

    /// Lift Drained from a member (retail `RemoveCharacterCondition`): the
    /// maxima get back what draining and disease took, and what is left
    /// rises with them. Whether it was there to lift.
    pub fn lift_drain(&mut self, conditions: &mut Conditions, profile: &mut Character) -> bool {
        if !conditions.has(Condition::Drained) {
            return false;
        }
        conditions.lift(Condition::Drained);
        let (health, stamina) = (profile.hp_drained, profile.stamina_drained);
        (profile.hp_drained, profile.stamina_drained) = (0, 0);
        if self.is_dead() {
            self.health.max += health;
            self.stamina.max += stamina;
        } else {
            self.health.follow(self.health.max + health);
            self.stamina.follow(self.stamina.max + stamina);
        }
        true
    }

    /// An attribute lost for good moves a member's maxima to these, and
    /// what is left falls by as much (retail `ApplyAttributeChange`,
    /// `RecalculateCharacterHitPoints`, `RecalculateCharacterStamina`): one
    /// with too little health left dies of it. Whether they did.
    pub fn weaken(&mut self, health: u32, stamina: u32) -> bool {
        if self.is_dead() {
            return false;
        }
        let had = self.health.current;
        self.health.follow(health);
        self.stamina.follow(stamina);
        if self.is_dead() {
            self.fell_from = had;
            self.fell_to = None;
        }
        self.is_dead()
    }

    /// What disease does to a member as `minutes` pass (retail
    /// `GameTurnsPassedChar`): a roll under the minutes out of ten times
    /// Vitality strikes one of four ways. Retail draws its condition from
    /// five but names a sixth, Paralyzed, that the draw never reaches.
    pub fn sicken(
        &mut self,
        conditions: &mut Conditions,
        profile: &mut Character,
        vitality: u32,
        minutes: u32,
        rng: &mut GameRng,
    ) -> Option<Sickness> {
        if !conditions.has(Condition::Diseased) || rng.below(vitality * 10) >= minutes {
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
                conditions.afflict(condition, Lasting::Indefinite);
                Some(Sickness::Condition(condition))
            }
            // The sheet loses a point; the member is refitted for it, and
            // their maxima follow.
            1 => {
                use enum_map::Enum;
                let attribute = Attribute::from_usize(rng.below(7) as usize);
                if profile.attributes[attribute] <= 1 {
                    return None;
                }
                profile.attributes[attribute] -= 1;
                Some(Sickness::Weakened(attribute))
            }
            2 => {
                if self.health.max < 5 {
                    return None;
                }
                let loss = rng.below(4) + 1;
                profile.hp_drained += loss;
                self.health.max -= loss;
                if self.health.current > 1 {
                    self.health.current -= loss.min(self.health.current - 1);
                }
                Some(Sickness::Health(loss))
            }
            _ => {
                if self.stamina.max < 5 {
                    return None;
                }
                let loss = rng.below(4) + 1;
                profile.stamina_drained += loss;
                self.stamina.max -= loss;
                if self.stamina.current > 1 {
                    self.stamina.current -= loss.min(self.stamina.current - 1);
                }
                Some(Sickness::Stamina(loss))
            }
        }
    }

    /// Spend stamina (retail `FatigueCharacter`, `FatigueMonster`). The
    /// dead spend none. What it took. A body whose stamina is spent is out
    /// cold, which [`settle`] sees to.
    pub fn tire(&mut self, amount: u32) -> u32 {
        if self.is_dead() {
            return 0;
        }
        let taken = amount.min(self.stamina.current);
        self.stamina.current -= taken;
        taken
    }

    /// Give stamina back up to its maximum. The dead get none. What it gave.
    pub fn rest(&mut self, amount: u32) -> u32 {
        if self.is_dead() {
            return 0;
        }
        let before = self.stamina.current;
        self.stamina.current = (before + amount).min(self.stamina.max);
        self.stamina.current - before
    }

    /// Heal up to the maximum. Healing does not raise the dead. What it gave.
    pub fn heal(&mut self, amount: u32) -> u32 {
        if self.is_dead() {
            return 0;
        }
        let before = self.health.current;
        self.health.current = (before + amount).min(self.health.max);
        self.health.current - before
    }
}

/// A body whose health has run out, a party member's or a monster's. What
/// death brings about (a monster's loot, a member giving up their seat, an
/// NPC member leaving, a fight's kill) follows its addition, wherever the
/// blow came from; and it answers to whoever ran the health out, if someone
/// did.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dead {
    pub by: Option<Entity>,
}

/// A member used a skill. Use may raise it a point
/// ([`Character::practise`]), which refits them.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Practise {
    #[event_target]
    pub member: Entity,
    pub skill: Skill,
    pub usage: u32,
}

/// A member's skill went up a point.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Improved {
    pub member: Entity,
    pub skill: Skill,
}

/// A member's skill is practised; one that rose is announced. Anyone without a sheet practises nothing.
pub(crate) fn practise(
    event: On<Practise>,
    mut sheets: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
    mut improved: MessageWriter<Improved>,
) {
    let Ok(mut sheet) = sheets.get_mut(event.member) else { return };
    if sheet.practise(event.skill, event.usage, &mut rng) {
        improved.write(Improved { member: event.member, skill: event.skill });
    }
}

/// An actor's health has run out. Said of that actor by whatever hurt it,
/// as it does ([`Damaged`] tells the screen what was lost).
///
/// [`Damaged`]: crate::game::hits::Damaged
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Fell {
    #[event_target]
    pub body: Entity,
}

/// A death is settled: said once one who fell is [`Dead`], or has cheated
/// it, for whatever follows (the party's defeat).
#[derive(Event, Clone, Copy, Debug)]
pub struct Settled;

/// One whose health ran out dies: a Valkyrie cheats it
/// ([`Vitals::cheats_death`]) in whatever fight is on; anyone else loses
/// what lasted on them, keeps the side they died on, and is [`Dead`].
pub fn fall(
    fell: On<Fell>,
    mut commands: Commands,
    mut bodies: Query<
        (
            &mut Vitals,
            &mut Conditions,
            &mut crate::game::world::hostility::Side,
            Option<&Character>,
            Option<&mut Tally>,
        ),
        Without<Dead>,
    >,
    fight: Option<Single<Entity, With<crate::game::combat::Battle>>>,
    mut rng: ResMut<GameRng>,
) {
    let Ok((mut vitals, mut conditions, mut side, profile, tally)) = bodies.get_mut(fell.body)
    else {
        return;
    };
    let fight = fight.map(|fight| *fight);
    let cheated = vitals.is_dead()
        && profile
            .is_some_and(|profile| vitals.cheats_death(&mut conditions, profile, fight, &mut rng));
    if vitals.is_dead() && !cheated {
        if let Some(mut tally) = tally {
            tally.deaths += 1;
        }
        side.die(&conditions);
        conditions.lift_all();
        vitals.stamina.current = 0;
        commands.entity(fell.body).insert(Dead { by: vitals.fell_to });
    }
    commands.trigger(Settled);
}

/// What a member regains of health and stamina in parts not yet whole
/// points; retail keeps these between ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Regeneration {
    pub health: f32,
    pub stamina: f32,
}

impl Vitals {
    /// Passive stamina recovery over `ticks` 1.4-second stamina ticks, at
    /// `(0.9·max + 20)/240` per tick times `scale` (retail
    /// `RegenCharacterStamina`).
    pub fn recover_stamina(&mut self, ticks: u32, scale: f32) {
        let (dead, pool) = (self.is_dead(), self.stamina);
        let fraction = &mut self.regeneration.stamina;
        if let Some(whole) = regenerate(dead, pool, fraction, ticks, scale, 0.9) {
            self.rest(whole);
            if self.stamina.current >= self.stamina.max {
                self.regeneration.stamina = 0.0;
            }
        }
    }

    /// Passive healing over `ticks` 120-second aging ticks, at
    /// `(0.4·max + 20)/240` per tick times `scale` (retail aging).
    pub fn recover_health(&mut self, ticks: u32, scale: f32) {
        let (dead, pool) = (self.is_dead(), self.health);
        let fraction = &mut self.regeneration.health;
        if let Some(whole) = regenerate(dead, pool, fraction, ticks, scale, 0.4) {
            self.heal(whole);
            if self.health.current >= self.health.max {
                self.regeneration.health = 0.0;
            }
        }
    }
}

/// Regeneration over `ticks` at `(factor·max + 20)/240` a tick times
/// `scale`: the whole points now due, carrying the rest in `fraction`; none,
/// with the fraction dropped, for the dead, at no rate, or when full.
fn regenerate(
    dead: bool,
    pool: Pool,
    fraction: &mut f32,
    ticks: u32,
    scale: f32,
    factor: f32,
) -> Option<u32> {
    if dead || scale <= 0.0 || pool.current >= pool.max {
        *fraction = 0.0;
        return None;
    }
    *fraction += ticks as f32 * (factor * pool.max as f32 + 20.0) / 240.0 * scale;
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

    /// Use announces itself: a skill that rose says so.
    #[test]
    fn practice_announces_a_skill_that_rose() {
        let mut app = App::new();
        app.insert_resource(crate::game::rng::GameRng::seeded(4))
            .add_message::<Improved>()
            .add_observer(practise);
        let mage = app
            .world_mut()
            .spawn(Character::new("Mage", Race::Human, Gender::Male, Profession::Mage))
            .id();
        app.world_mut().trigger(Practise { member: mage, skill: Skill::FireMagic, usage: 400 });
        app.world_mut().flush();
        let told: Vec<_> = app.world_mut().resource_mut::<Messages<Improved>>().drain().collect();
        assert!(told.contains(&Improved { member: mage, skill: Skill::FireMagic }));
    }

    #[test]
    fn a_human_fighter_has_the_worked_example_numbers() {
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        assert_eq!(fighter.attributes, EnumMap::from_array([55, 45, 45, 50, 50, 45, 45]));
        assert_eq!(fighter.max_hp(&Attributes::of_sheet(&fighter)), 18);
        assert_eq!(fighter.max_stamina(&Attributes::of_sheet(&fighter)), 105);
        assert_eq!(fighter.capacity(&Attributes::of_sheet(&fighter)), 1920);
        assert_eq!(
            fighter.initiative(
                &Attributes::of_sheet(&fighter),
                Load::None,
                &crate::game::actors::Skills::of_sheet(&fighter)
            ),
            9
        );
        assert_eq!(
            fighter.initiative(
                &Attributes::of_sheet(&fighter),
                Load::Heavy,
                &crate::game::actors::Skills::of_sheet(&fighter)
            ),
            5
        );
        assert_eq!(fighter.load(&Attributes::of_sheet(&fighter), 959), Load::None);
        assert_eq!(fighter.load(&Attributes::of_sheet(&fighter), 960), Load::Light);
        assert_eq!(
            fighter.load(&Attributes::of_sheet(&fighter), 1939),
            Load::Heavy,
            "integer percent: still 100"
        );
        assert_eq!(fighter.load(&Attributes::of_sheet(&fighter), 1940), Load::Extreme);
        assert_eq!(fighter.experience_goal(), 1000);
        // Profession and bonus skills are seeded; others are not.
        let close_combat = fighter.skill_points[Skill::CloseCombat];
        assert!(crate::game::actors::Skills::of_sheet(&fighter)[Skill::CloseCombat] > close_combat);
        assert!(fighter.skill_points[Skill::Sword] > 0);
        assert_eq!(fighter.skill_points[Skill::LocksAndTraps], 0);
    }

    #[test]
    fn skill_availability_follows_profession_levels_and_attribute_mastery() {
        let mut fighter = Character::new("F", Race::Human, Gender::Male, Profession::Fighter);
        assert!(fighter.skill_available(Skill::Sword));
        assert!(!fighter.skill_available(Skill::Music));
        fighter.skill_points[Skill::CloseCombat] = 0;
        assert_eq!(crate::game::actors::Skills::of_sheet(&fighter)[Skill::CloseCombat], 1);
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
        mage.profession_levels[Profession::Mage] = 3;
        mage.experience_previous_goal = 2400;
        assert_eq!(mage.experience_goal(), 1200 + 1200 + 2400);
        mage.profession_levels[Profession::Mage] = 11;
        mage.experience_previous_goal = 1200 * (1 + 511);
        // Level 1 is one weight; levels 2 to 10 add 1, 2, ..., 256 weights.
        let at_ten = 1200 * (1 + 511);
        assert_eq!(mage.experience_goal(), at_ten + 256 * 1200 * 12 / 10);
    }

    /// Hybrids cast from their fifth level, so their book skills open then;
    /// pure casters have theirs at once and the others never.
    #[test]
    fn a_hybrids_book_skills_open_with_its_caster_level() {
        let mut lord = Character::new("L", Race::Human, Gender::Male, Profession::Lord);
        assert!(!lord.skill_available(Skill::Divinity));
        lord.profession_levels[Profession::Lord] = 5;
        assert!(lord.skill_available(Skill::Divinity));
        assert!(!lord.skill_available(Skill::Wizardry));
        let priest = Character::new("P", Race::Human, Gender::Male, Profession::Priest);
        assert!(priest.skill_available(Skill::Divinity));
        let fighter = Character::new("F", Race::Human, Gender::Male, Profession::Fighter);
        assert!(!fighter.skill_available(Skill::Divinity));
    }

    #[test]
    fn items_check_profession_race_gender_and_minimums() {
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut bow = crate::content::items::ItemDefinition {
            name: "Strong Bow".into(),
            class: EquipClass::RangedWeapon,
            weapon_skill: Some(Skill::Bow),
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..default()
        };
        bow.attribute_minimums[Strength] = 85;

        let can_use = |bow: &crate::content::items::ItemDefinition| {
            crate::game::items::users(bow).admit(
                &fighter,
                &Attributes::of_sheet(&fighter),
                &crate::game::actors::Skills::of_sheet(&fighter),
            )
        };
        assert!(!can_use(&bow), "too weak");
        bow.attribute_minimums[Strength] = 0;
        assert!(can_use(&bow));
        bow.genders = EnumSet::only(Gender::Female);
        assert!(!can_use(&bow), "female only");
        bow.genders = EnumSet::all();
        bow.professions = EnumSet::only(Profession::Mage);
        assert!(!can_use(&bow));
    }

    /// A Valkyrie cheats death (retail `CheatDeathRevive`): she falls
    /// unconscious for a few rounds instead, at half to all of her health
    /// the first time in a fight and a roll of what she had any other time;
    /// seven rounds or more out cold, or out cold for good, she dies.
    #[test]
    fn a_valkyrie_cheats_death_unless_long_out_cold() {
        let valkyrie =
            Character::new("Brunhild", Race::Human, Gender::Female, Profession::Valkyrie);
        let maximum = valkyrie.max_hp(&Attributes::of_sheet(&valkyrie));
        let mut rng = crate::game::rng::GameRng::seeded(9);
        let fight = Entity::from_bits(77);
        let other = Entity::from_bits(78);
        // A blow just past her health, and death settled as it is settled:
        // cheated, or died.
        let mut struck = |vitals: &mut Vitals, conditions: &mut Conditions, fight| {
            let hp = vitals.health.current;
            let lost = vitals.hurt(hp + 1, None);
            assert_eq!(lost, hp, "the blow takes what she had");
            vitals.cheats_death(conditions, &valkyrie, fight, &mut rng);
        };
        // Outside a fight: a roll of the health she had, at least one, and
        // a stint out cold (level one scales five down to three).
        let (mut member, mut ailing) = (Vitals::new(&valkyrie), Conditions::default());
        member.health.current = 4;
        struck(&mut member, &mut ailing, None);
        assert!((1..=4).contains(&member.health.current), "{}", member.health.current);
        assert_eq!(ailing.lasting(Condition::Unconscious), Some(Lasting::Rounds(4)));
        // The first time in a fight, half her maximum or more; the second
        // time in the same fight only the small roll; a new fight, half again.
        let (mut member, mut ailing) = (Vitals::new(&valkyrie), Conditions::default());
        struck(&mut member, &mut ailing, Some(fight));
        let hp = member.health.current;
        assert!(hp >= maximum / 2 && hp <= maximum, "{hp}");
        ailing.lift(Condition::Unconscious);
        struck(&mut member, &mut ailing, Some(fight));
        let now = member.health.current;
        assert!(now <= hp, "only a roll of what she had: {now} of {hp}");
        ailing.lift(Condition::Unconscious);
        struck(&mut member, &mut ailing, Some(other));
        assert!(member.health.current >= maximum / 2, "{}", member.health.current);
        // Long out cold, or out cold for good, she dies; so does anyone
        // without the trait.
        for lasting in [Lasting::Rounds(7), Lasting::Indefinite] {
            let mut member = Vitals::new(&valkyrie);
            let mut ailing = Conditions::from([(Condition::Unconscious, lasting)]);
            struck(&mut member, &mut ailing, Some(fight));
            assert!(member.is_dead());
        }
        let fighter = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let (mut member, mut ailing) = (Vitals::new(&fighter), Conditions::default());
        member.hurt(1000, None);
        assert!(!member.cheats_death(&mut ailing, &fighter, Some(fight), &mut rng));
        // Draining her to nothing is cheated too.
        let mut drained = valkyrie.clone();
        let (mut member, mut ailing) = (Vitals::new(&drained), Conditions::default());
        member.health.current = 2;
        member.drain_health(&mut ailing, &mut drained, 5, None);
        assert!(member.is_dead() && member.fell_from == 2);
        assert!(member.cheats_death(&mut ailing, &drained, None, &mut rng));
        assert!(!member.is_dead() && ailing.has(Condition::Drained));
    }

    #[test]
    fn damage_tires_and_kills_and_the_dead_take_and_gain_nothing() {
        let profile = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut fighter = Vitals::new(&profile);
        assert_eq!(fighter.hurt(6, None), 6);
        assert_eq!((fighter.health.current, fighter.stamina.current), (12, 101));
        assert_eq!(fighter.hurt(100, None), 12);
        assert!(fighter.is_dead());
        assert_eq!(fighter.hurt(5, None), 0);
        fighter.heal(10);
        assert!(fighter.is_dead(), "healing does not raise the dead");
    }

    #[test]
    fn rest_recovers_stamina_quickly_and_health_slowly() {
        let profile = Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter);
        let mut fighter = Vitals::new(&profile);
        fighter.health.current = 1;
        fighter.stamina.current = 0;
        // (0.9 × 105 + 20) / 240 × 0.5 ≈ 0.24 a tick: 10 after 42.
        fighter.recover_stamina(41, 0.5);
        assert_eq!(fighter.stamina.current, 9);
        fighter.recover_stamina(1, 0.5);
        assert_eq!(fighter.stamina.current, 10);
        // (0.4 × 18 + 20) / 240 × 0.5 ≈ 0.057 a tick.
        fighter.recover_health(17, 0.5);
        assert_eq!(fighter.health.current, 1);
        fighter.recover_health(1, 0.5);
        assert_eq!(fighter.health.current, 2);
    }

    /// A poison bites at its strength each aging tick, half again on the
    /// move, and weakens by its share of what is left of it (retail
    /// `GameTurnsPassedChar`, `TickCharacterCondition`).
    #[test]
    fn poison_bites_harder_on_the_move_and_wears_off() {
        let mut rng = GameRng::default();
        let mut poisoned = Conditions::default();
        poisoned.poison(Lasting::Rounds(10), 4);
        assert_eq!(poisoned.poison_damage(1, false), 6, "four, half again on the move");
        assert_eq!(poisoned.poison_damage(1, true), 4);
        // A tenth of the course gone takes a tenth of the strength, which
        // rounds to nothing here but a roll's chance of one.
        poisoned.age(1, &mut rng);
        assert!(poisoned.poison == 4 || poisoned.poison == 3);
        assert_eq!(poisoned.lasting(Condition::Poisoned), Some(Lasting::Rounds(9)));
        // Half of what is left: half the strength.
        let mut halved = Conditions::default();
        halved.poison(Lasting::Rounds(10), 8);
        halved.age(5, &mut rng);
        assert_eq!(halved.poison, 4);
        // A stronger poison raises the bite; a weaker one does not lower it,
        // and a fresh poisoning after one runs out starts afresh.
        halved.poison(Lasting::Rounds(2), 6);
        assert_eq!(halved.poison, 6);
        halved.poison(Lasting::Rounds(2), 1);
        assert_eq!(halved.poison, 6);
        halved.age(20, &mut rng);
        assert!(!halved.has(Condition::Poisoned));
        assert_eq!(halved.poison_damage(5, false), 0);
        halved.poison(Lasting::Rounds(3), 2);
        assert_eq!(halved.poison, 2);
    }

    /// Draining takes health from the maximum until Drained is lifted, which
    /// gives back what draining and disease took (retail `DamageCharacter`,
    /// `RemoveCharacterCondition`).
    #[test]
    fn drained_health_stays_lost_until_the_drain_is_lifted() {
        let mut profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut member = Vitals::new(&profile);
        let (max_hp, max_stamina) = (
            profile.max_hp(&Attributes::of_sheet(&profile)),
            profile.max_stamina(&Attributes::of_sheet(&profile)),
        );
        let mut ailing = Conditions::default();
        assert_eq!(member.drain_health(&mut ailing, &mut profile, 3, None), 3);
        assert_eq!(
            (profile.max_hp(&Attributes::of_sheet(&profile)), member.health.current),
            (max_hp - 3, max_hp - 3)
        );
        assert_eq!(member.stamina.current, max_stamina - 2, "two thirds of it tires");
        assert!(ailing.has(Condition::Drained));
        profile.stamina_drained = 4;
        member.stamina.current =
            member.stamina.current.min(profile.max_stamina(&Attributes::of_sheet(&profile)));
        // Recovery heals only up to the lowered maximum.
        member.recover_health(60, 1.0);
        assert_eq!(member.health.current, max_hp - 3);
        let stamina = member.stamina.current;
        assert!(member.lift_drain(&mut ailing, &mut profile));
        assert_eq!(
            (profile.max_hp(&Attributes::of_sheet(&profile)), member.health.current),
            (max_hp, max_hp)
        );
        assert_eq!(member.stamina.current, stamina + 4);
        assert!(!member.lift_drain(&mut ailing, &mut profile), "nothing left to lift");
        // Draining the last of it kills, and the dead stay Drained.
        let mut doomed = Vitals::new(&profile);
        doomed.health.current = 2;
        let mut ailing = Conditions::default();
        assert_eq!(doomed.drain_health(&mut ailing, &mut profile, 5, None), 2);
        assert!(doomed.is_dead() && ailing.has(Condition::Drained));
    }

    #[test]
    fn timed_conditions_run_out() {
        let mut rng = crate::game::rng::GameRng::default();
        let mut ailing = Conditions::from([
            (Condition::Asleep, Lasting::Rounds(3)),
            (Condition::Unconscious, Lasting::Indefinite),
        ]);
        ailing.age(2, &mut rng);
        assert_eq!(ailing.lasting(Condition::Asleep), Some(Lasting::Rounds(1)));
        ailing.age(1, &mut rng);
        ailing.afflict(Condition::Asleep, Lasting::Rounds(70_000));
        ailing.age(69_999, &mut rng);
        assert_eq!(ailing.lasting(Condition::Asleep), Some(Lasting::Rounds(1)));
        ailing.age(1, &mut rng);
        assert_eq!(ailing, Conditions::from([(Condition::Unconscious, Lasting::Indefinite)]));
    }
}
