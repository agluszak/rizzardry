//! Party members: attributes, skills, and derived statistics.
//! The tables come from the original executable.

pub mod creation;
pub mod progression;
pub mod resistance;

use bevy::prelude::*;
use enum_map::EnumMap;

use crate::content::characters::{Attribute, Realm, Skill, Trait};
pub use crate::content::characters::{Gender, Profession, Race};
use crate::game::actors::{Abilities, Attributes};
use crate::game::rng::GameRng;
use enumset::EnumSet;

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

/// Who a character is to those who see and hear them.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub name: String,
    pub full_name: String,
    pub personality: crate::content::characters::Personality,
    pub voice: crate::content::characters::Voice,
    /// The portrait's file stem (e.g. `hummf` for `data/portraits/small/shummf.sti`).
    pub portrait: String,
}

/// What a character was born as, for life.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Birth {
    pub race: Race,
    pub gender: Gender,
}

/// What a character trained as, and how far: the mark of one who trained,
/// and what brings the rest of a character with it.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(
    MonsterKnowledge,
    Tally,
    Practice,
    Experience,
    progression::SpellPicks,
    crate::game::items::Wears,
    crate::game::items::Pack,
    crate::game::items::Burden,
    crate::game::magic::LearnedSpells,
    crate::game::magic::SpellPoints,
    crate::game::party::Order
)]
pub struct Calling {
    pub profession: Profession,
    pub first: Profession,
    /// Levels gained in each profession; the level is their sum.
    pub levels: EnumMap<Profession, u8>,
}

impl Calling {
    /// One level of a profession, as a character begins.
    pub fn begun(profession: Profession) -> Self {
        let mut levels = EnumMap::default();
        levels[profession] = 1;
        Self { profession, first: profession, levels }
    }

    /// The total level: every level gained in any profession (retail
    /// `uiExpLevel`). Advancement stops at [`MAX_LEVEL`].
    pub fn level(&self) -> u8 {
        self.levels.values().sum()
    }

    /// The level in the profession followed now, which a trait's strength
    /// grows with.
    pub fn mastery(&self) -> u8 {
        self.levels[self.profession]
    }

    /// A trait's value by the level of the current profession (retail
    /// `ScaleValueByProfessionLevel`).
    pub fn scale(&self, value: f32) -> f32 {
        scale_by_level(self.mastery(), value)
    }

    /// Levels weighted by how physical each profession is.
    pub fn physical_levels(&self) -> i32 {
        self.levels
            .iter()
            .map(|(profession, &levels)| i32::from(levels) * profession.row().physical)
            .sum()
    }

    /// The health training gives at this Vitality:
    /// `round(sum of (0.6 + 0.008 x VIT) x factor x levels)`, counting one
    /// extra level for the first profession (retail
    /// `RecalculateCharacterHitPoints`).
    pub fn health(&self, vitality: u32) -> u32 {
        let share = 0.6 + 0.008 * vitality as f32;
        let total: f32 = self
            .levels
            .iter()
            .map(|(profession, &levels)| {
                let mut levels = u32::from(levels);
                if profession == self.first && levels > 0 {
                    levels += 1;
                }
                share * profession.row().hit_points * levels as f32
            })
            .sum();
        total.round() as u32
    }

    /// The passed threshold plus this profession's next increment: doubling
    /// to level 10, then 20% growth. Prior professions keep their actual cost.
    pub fn experience_goal(&self, experience: &Experience) -> u64 {
        experience.last_goal + progression::experience_increment(self.profession, self.level())
    }
}

/// The stamina training gives: `round((STR + PIE + VIT) / 3 x (0.1 x level
/// + 2))` (retail `RecalculateCharacterStamina`).
pub fn trained_stamina(level: u32, a: &Attributes) -> u32 {
    let sum = (a[Attribute::Strength] + a[Attribute::Piety] + a[Attribute::Vitality]) as f32;
    (sum / 3.0 * (0.1 * level as f32 + 2.0)).round() as u32
}

/// Points put into each skill.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Training(pub EnumMap<Skill, u32>);

/// Successful practice towards each skill's next point (retail
/// `practice_count`; eight make a point).
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Practice(pub EnumMap<Skill, u8>);

/// What a character has earned, and the threshold actually passed, which
/// keeps the prices of earlier professions.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Experience {
    pub earned: u64,
    pub last_goal: u64,
}

/// What a character has done and suffered: the foes they have killed in
/// fights, and the times they have died (retail `kill_count`,
/// `death_count`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub kills: u32,
    pub deaths: u32,
}

/// Recognition history changes independently of anything else.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterKnowledge {
    /// How many of each species the actor has seen, capped at 255.
    pub seen: std::collections::HashMap<u32, u32>,
}

/// A character on paper: what creation and advancement work on, and what
/// one is made with. Spawned, each field is a component of its own.
#[derive(Bundle, Clone, Debug, PartialEq)]
pub struct Character {
    pub identity: Identity,
    pub birth: Birth,
    pub calling: Calling,
    pub abilities: Abilities,
    pub training: Training,
    pub practice: Practice,
    pub experience: Experience,
    pub picks: progression::SpellPicks,
}

impl Character {
    /// A minimal valid level 1 character (retail's NPC recipe): each
    /// attribute at the race's base or the profession's minimum, whichever
    /// is higher; the profession's skills seeded from their base levels.
    /// Only the eleven playable races have bases.
    pub fn new(name: &str, race: Race, gender: Gender, profession: Profession) -> Self {
        let minimums = &profession.row().minimums;
        let base = race.row().attributes.unwrap_or(EnumMap::from_fn(|_| 45));
        let mut character = Character {
            identity: Identity { name: name.to_owned(), full_name: name.to_owned(), ..default() },
            birth: Birth { race, gender },
            calling: Calling::begun(profession),
            abilities: Abilities(EnumMap::from_fn(|a| base[a].max(minimums[a]))),
            training: default(),
            practice: default(),
            experience: default(),
            picks: default(),
        };
        character.seed_starting_skills();
        character
    }

    /// Seed from the final starting attributes, shared by recipes and drafts.
    fn seed_starting_skills(&mut self) {
        let row = self.calling.profession.row();
        self.training = default();
        let share = 60 / (1 + row.skills.len() as u32);
        for &skill in row.skills.iter().chain(std::iter::once(&row.bonus_skill)) {
            self.training.0[skill] = skill_base(&self.abilities, skill) * share / 100;
        }
    }

    /// The most health they have on paper, with nothing worn, laid on them
    /// or ailing them.
    pub fn max_hp(&self) -> u32 {
        self.calling.health(self.abilities.0[Attribute::Vitality])
    }

    /// The most stamina they have on paper.
    pub fn max_stamina(&self) -> u32 {
        trained_stamina(u32::from(self.calling.level()), &Attributes(self.abilities.0))
    }

    /// What they can carry on paper, in tenths of a pound.
    pub fn capacity(&self) -> u32 {
        capacity(&Attributes(self.abilities.0), self.birth.race.row().carry_factor)
    }

    /// What race and the profession followed now let them do.
    pub fn traits(&self) -> EnumSet<Trait> {
        traits_of(self.birth.race, &self.calling)
    }
}

/// A character's components, as the paper they were made from: to take a
/// copy of for a draft, and to write a finished draft back.
#[derive(bevy::ecs::query::QueryData)]
#[query_data(mutable)]
pub struct Paper {
    pub identity: &'static mut Identity,
    pub birth: &'static Birth,
    pub calling: &'static mut Calling,
    pub abilities: &'static mut Abilities,
    pub training: &'static mut Training,
    pub practice: &'static mut Practice,
    pub experience: &'static mut Experience,
    pub picks: &'static mut progression::SpellPicks,
}

impl PaperReadOnlyItem<'_, '_> {
    /// As the formulas of training read them.
    pub fn trained(&self) -> Trained<'_> {
        Trained {
            calling: self.calling,
            abilities: self.abilities,
            training: self.training,
            birth: self.birth,
            traits: traits_of(self.birth.race, self.calling),
        }
    }

    pub fn copy(&self) -> Character {
        Character {
            identity: self.identity.clone(),
            birth: *self.birth,
            calling: self.calling.clone(),
            abilities: *self.abilities,
            training: self.training.clone(),
            practice: self.practice.clone(),
            experience: *self.experience,
            picks: self.picks.clone(),
        }
    }
}

impl PaperItem<'_, '_> {
    pub fn copy(&self) -> Character {
        Character {
            identity: self.identity.clone(),
            birth: *self.birth,
            calling: self.calling.clone(),
            abilities: *self.abilities,
            training: self.training.clone(),
            practice: self.practice.clone(),
            experience: *self.experience,
            picks: self.picks.clone(),
        }
    }

    /// Each part that differs is written; what is the same is left alone.
    pub fn write(&mut self, next: Character) {
        self.identity.set_if_neq(next.identity);
        self.calling.set_if_neq(next.calling);
        self.abilities.set_if_neq(next.abilities);
        self.training.set_if_neq(next.training);
        self.practice.set_if_neq(next.practice);
        self.experience.set_if_neq(next.experience);
        self.picks.set_if_neq(next.picks);
    }
}

/// One who trained, as the formulas of training read them: borrowed from a
/// character on paper or from an actor's components alike.
#[derive(Clone, Copy, Debug)]
pub struct Trained<'a> {
    pub calling: &'a Calling,
    pub abilities: &'a Abilities,
    pub training: &'a Training,
    pub birth: &'a Birth,
    pub traits: EnumSet<Trait>,
}

/// The components of one who trained, for a query to borrow as [`Trained`].
#[derive(bevy::ecs::query::QueryData)]
pub struct Schooling {
    calling: &'static Calling,
    abilities: &'static Abilities,
    training: &'static Training,
    birth: &'static Birth,
    traits: &'static crate::game::actors::Traits,
}

impl SchoolingItem<'_, '_> {
    pub fn view(&self) -> Trained<'_> {
        Trained {
            calling: self.calling,
            abilities: self.abilities,
            training: self.training,
            birth: self.birth,
            traits: self.traits.0,
        }
    }
}

impl Trained<'_> {
    /// Whether they may use a skill ([`skill_available`]).
    pub fn can_use(&self, skill: Skill) -> bool {
        skill_available(self.calling, self.abilities, skill)
    }
}

impl Character {
    /// As the formulas of training read them.
    pub fn trained(&self) -> Trained<'_> {
        Trained {
            calling: &self.calling,
            abilities: &self.abilities,
            training: &self.training,
            birth: &self.birth,
            traits: self.traits(),
        }
    }
}

/// Abilities conferred by a race and the profession followed now.
pub fn traits_of(race: Race, calling: &Calling) -> EnumSet<Trait> {
    calling.profession.row().traits | race.row().traits
}

/// The average of a skill's governing attributes' bases.
fn skill_base(abilities: &Abilities, skill: Skill) -> u32 {
    let (a, b) = governing(skill);
    (abilities.0[a] + abilities.0[b]) / 2
}

/// Whether the original lets one of this calling and these abilities use
/// a skill. Realm skills need a learned spell in that realm, which is not
/// asked here.
pub fn skill_available(calling: &Calling, abilities: &Abilities, skill: Skill) -> bool {
    let index = skill as usize;
    if index >= Skill::PowerStrike as usize {
        let attribute = Attribute::try_from((index - Skill::PowerStrike as usize) as u8)
            .expect("expert skills follow the attribute order");
        return abilities.0[attribute] >= 100;
    }
    if index >= Skill::FireMagic as usize {
        return false;
    }
    let row = calling.profession.row();
    if !row.open_skills.contains(skill) {
        return false;
    }
    if skill == row.bonus_skill || row.skills.contains(&skill) {
        return true;
    }
    // A hybrid's book skills open with its caster level.
    if index >= Skill::Wizardry as usize && row.caster_offset.is_some_and(|offset| offset < 0) {
        return calling.mastery() > 4;
    }
    true
}

/// Practise a skill `usage` times (retail `PracticeCharacterSkill`): each
/// use counts with chance `(100 - points) x base / 100` percent (halved for
/// the expert skills, at least 1), and every eighth count is a point, up to
/// 100. Returns whether the skill improved.
pub fn practise_skill(
    (calling, abilities): (&Calling, &Abilities),
    (training, practice): (&mut Training, &mut Practice),
    skill: Skill,
    usage: u32,
    rng: &mut GameRng,
) -> bool {
    let index = skill as usize;
    let allowed = if index >= Skill::PowerStrike as usize {
        skill_available(calling, abilities, skill)
    } else if index >= Skill::FireMagic as usize {
        !calling.profession.row().books.is_empty()
    } else {
        calling.profession.row().open_skills.contains(skill)
    };
    if usage == 0 || !allowed {
        return false;
    }
    let mut improved = false;
    for _ in 0..usage {
        let points = training.0[skill];
        if points >= 100 {
            break;
        }
        let mut threshold = (100 - points) * skill_base(abilities, skill) / 100;
        if index >= Skill::PowerStrike as usize {
            threshold /= 2;
        }
        if rng.below(100) < threshold.max(1) {
            practice.0[skill] += 1;
            if practice.0[skill] >= 8 {
                practice.0[skill] = 0;
                training.0[skill] += 1;
                improved = true;
            }
        }
    }
    improved
}

/// The load band of carrying `weight` when `(VIT + 2 x STR)` times the
/// carry factor can be carried, in tenths of a pound.
pub fn load(a: &Attributes, carry_factor: u32, weight: u32) -> Load {
    load_band(weight, capacity(a, carry_factor))
}

/// Tenths of a pound: `(VIT + 2 x STR)` times the carry factor.
pub fn capacity(a: &Attributes, carry_factor: u32) -> u32 {
    (a[Attribute::Vitality] + 2 * a[Attribute::Strength]) * carry_factor
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

/// The body of a party member or a monster: its health and stamina, and
/// how health last ran out. What ails it is its [`Conditions`], and what it
/// is fit for the markers [`settle`] keeps on it. The most of each is a
/// figure, written from what the body is made of.
///
/// [`settle`]: crate::game::actors::settle
#[derive(Component, Clone, Debug, Default, PartialEq)]
#[require(
    Conditions,
    Recovering,
    // A body is made fit for everything; what ails it takes that away.
    crate::game::actors::Lucid,
    crate::game::actors::CanGuard,
    crate::game::actors::CanAct,
    crate::game::actors::CanFight,
    crate::game::actors::fitness::Present,
    crate::game::world::hostility::Side,
    crate::game::world::hostility::Loyalty,
    // What it is made of.
    crate::game::actors::Abilities,
    crate::game::actors::Traits,
    crate::game::actors::Nature,
    crate::game::actors::Drained,
    crate::game::actors::Modifiers,
    // What rules read of it.
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
    /// The health the body had before whatever ran it out, which a
    /// Valkyrie's later cheats of death in a fight roll under (retail
    /// `CheatDeathRevive` reads it before the blow lands).
    pub fell_from: u32,
    /// Who ran its health out, if someone did (retail `condition_target`),
    /// written by whatever ran it out and read as its death is settled.
    fell_to: Option<Entity>,
}

impl Vitals {
    /// A character as they are on paper, rested and well.
    pub fn new(sheet: &Character) -> Self {
        Self::full(sheet.max_hp(), sheet.max_stamina())
    }

    /// Full health and stamina at these maxima.
    pub fn full(health: u32, stamina: u32) -> Self {
        Self { health: Pool::full(health), stamina: Pool::full(stamina), ..default() }
    }

    pub fn is_dead(&self) -> bool {
        self.health.current == 0
    }

    /// The most the body can have is this now. Whatever moved it, what is
    /// left moves by as much, not below nothing (retail
    /// `RecalculateCharacterHitPoints`, `RecalculateCharacterStamina`); the
    /// dead keep nothing. Health brought to nothing is a fall.
    pub fn set_most(&mut self, health: u32, stamina: u32) {
        if self.is_dead() {
            (self.health.max, self.stamina.max) = (health, stamina);
            return;
        }
        let had = self.health.current;
        self.health.follow(health);
        self.stamina.follow(stamina);
        if self.is_dead() {
            (self.fell_from, self.fell_to) = (had, None);
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
    /// none. Health run out is death once it is settled ([`fall`]).
    /// Returns the health lost. The caller handles waking. `by` is whoever
    /// does it, who answers for a death it brings: nobody for a poison, a
    /// cloud or a fall.
    pub fn hurt(&mut self, amount: u32, by: Option<Entity>) -> u32 {
        if self.is_dead() {
            return 0;
        }
        self.tire(amount * 2 / 3);
        let lost = amount.min(self.health.current);
        self.health.current -= lost;
        if self.is_dead() {
            (self.fell_from, self.fell_to) = (lost, by);
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
            (self.fell_from, self.fell_to) = (had, by);
        }
        had
    }

    /// One with the trait (retail `CheatDeathRevive`) is kept from death
    /// unless already seven rounds or more out cold: a stint unconscious,
    /// shorter the higher their `mastery`, and health back. The `first`
    /// time in a fight it is half to all of their maximum, scaled; any
    /// other time, a scaled roll of the health they had before the blow.
    /// Whether they cheated it.
    pub fn cheats_death(
        &mut self,
        conditions: &mut Conditions,
        (traits, mastery): (&crate::game::actors::Traits, u8),
        first: bool,
        rng: &mut GameRng,
    ) -> bool {
        if !traits.has(Trait::CheatDeath) {
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
        let shortened = scale_by_level(mastery, 5.0) as u32;
        let stint = (rounds + 7).max(shortened) - shortened;
        if stint > 0 {
            conditions.afflict(Condition::Unconscious, Lasting::Rounds(stint));
        }
        let maximum = self.health.max;
        self.health.current = if first {
            let share = rng.below(scale_by_level(mastery, maximum as f32 * 0.5) as u32);
            (share + 50) * maximum / 100
        } else {
            rng.below(scale_by_level(mastery, self.fell_from as f32) as u32) + 1
        };
        true
    }

    /// Drain health for good (retail `DamageCharacter`, which draining
    /// blows, breaths and Vampiric Vortex use): two thirds of it tires, the
    /// maximum loses it until Drained is lifted, and what is left falls
    /// with the maximum; none left is death. Guardian Angel does not stop
    /// it. Returns the health lost.
    pub fn drain_health(
        &mut self,
        conditions: &mut Conditions,
        drained: &mut crate::game::actors::Drained,
        amount: u32,
        by: Option<Entity>,
    ) -> u32 {
        if self.is_dead() || amount == 0 {
            return 0;
        }
        self.tire(amount * 2 / 3);
        let had = self.health.current;
        drained.health += amount;
        self.health.follow(self.health.max.max(amount + 1) - amount);
        if self.is_dead() {
            (self.fell_from, self.fell_to) = (had, by);
        }
        if !conditions.has(Condition::Drained) {
            conditions.afflict(Condition::Drained, Lasting::Indefinite);
        }
        had - self.health.current
    }

    /// What disease does to a body as `minutes` pass (retail
    /// `GameTurnsPassedChar`): a roll under the minutes out of ten times
    /// Vitality strikes one of four ways. Retail draws its condition from
    /// five but names a sixth, Paralyzed, that the draw never reaches.
    pub fn sicken(
        &mut self,
        conditions: &mut Conditions,
        (abilities, drained): (
            &mut crate::game::actors::Abilities,
            &mut crate::game::actors::Drained,
        ),
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
            // A point is lost for good; the maxima follow as figures do.
            1 => {
                use enum_map::Enum;
                let attribute = Attribute::from_usize(rng.below(7) as usize);
                if abilities.0[attribute] <= 1 {
                    return None;
                }
                abilities.0[attribute] -= 1;
                Some(Sickness::Weakened(attribute))
            }
            // The most health or stamina wastes, and what is left with it,
            // though never the last of it.
            2 => {
                if self.health.max < 5 {
                    return None;
                }
                let loss = rng.below(4) + 1;
                drained.health += loss;
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
                drained.stamina += loss;
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
    /// cold, which settling sees to.
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

    /// Passive stamina recovery over `ticks`, at `(0.9 x most + 20) / 240`
    /// a tick times `scale` (retail `RegenCharacterStamina`).
    pub fn recover_stamina(&mut self, parts: &mut Recovering, ticks: u32, scale: f32) {
        let (dead, pool) = (self.is_dead(), self.stamina);
        if let Some(whole) = regenerate(dead, pool, &mut parts.stamina, ticks, scale, 0.9) {
            self.rest(whole);
            if self.stamina.current >= self.stamina.max {
                parts.stamina = 0.0;
            }
        }
    }

    /// Passive healing over `ticks`, at `(0.4 x most + 20) / 240` a tick
    /// times `scale` (retail aging).
    pub fn recover_health(&mut self, parts: &mut Recovering, ticks: u32, scale: f32) {
        let (dead, pool) = (self.is_dead(), self.health);
        if let Some(whole) = regenerate(dead, pool, &mut parts.health, ticks, scale, 0.4) {
            self.heal(whole);
            if self.health.current >= self.health.max {
                parts.health = 0.0;
            }
        }
    }
}

/// What a body has regained of health and stamina in parts not yet whole
/// points, kept between ticks as the original keeps them.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Recovering {
    pub health: f32,
    pub stamina: f32,
}

/// Regeneration over `ticks` at `(factor x most + 20) / 240` a tick times
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

/// A body whose health has run out, a party member's or a monster's. What
/// death brings about (a monster's loot, a member giving up their seat, an
/// NPC member leaving, a fight's kill) follows its addition, wherever the
/// blow came from; and it answers to whoever ran the health out, if someone
/// did.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dead {
    pub by: Option<Entity>,
}

/// One who has cheated death in a fight, which only the first time in a
/// fight is done at full strength. Gone with the fight.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = CheatedBy)]
pub struct CheatedDeathIn(pub Entity);

/// Those who cheated death in a fight, maintained by Bevy from
/// [`CheatedDeathIn`].
#[derive(Component, Debug)]
#[relationship_target(relationship = CheatedDeathIn)]
pub struct CheatedBy(Vec<Entity>);

/// One who trained used a skill. Use may raise it a point
/// ([`practise_skill`]).
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

/// A skill is practised; one that rose is announced. Anyone who never
/// trained practises nothing.
pub(crate) fn practise(
    event: On<Practise>,
    mut trained: Query<(&Calling, &Abilities, &mut Training, &mut Practice)>,
    mut rng: ResMut<GameRng>,
    mut improved: MessageWriter<Improved>,
) {
    let Ok((calling, abilities, mut training, mut practice)) = trained.get_mut(event.member) else {
        return;
    };
    // Counted on a copy first, so that practice which changes nothing does
    // not mark the training as changed.
    let (mut points, mut counts) = (training.clone(), practice.clone());
    let rose = practise_skill(
        (calling, abilities),
        (&mut points, &mut counts),
        event.skill,
        event.usage,
        &mut rng,
    );
    practice.set_if_neq(counts);
    training.set_if_neq(points);
    if rose {
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

/// One whose health ran out dies: one with the trait cheats it
/// ([`Vitals::cheats_death`]) in whatever fight is on; anyone else loses
/// what lasted on them, keeps the side they died on, and is [`Dead`].
#[allow(clippy::type_complexity)]
pub fn fall(
    fell: On<Fell>,
    mut commands: Commands,
    mut bodies: Query<
        (
            &mut Vitals,
            &mut Conditions,
            &mut crate::game::world::hostility::Side,
            (&crate::game::actors::Traits, &crate::game::actors::Level, Option<&Calling>),
            Option<&mut Tally>,
            Option<&CheatedDeathIn>,
        ),
        Without<Dead>,
    >,
    fight: Option<Single<Entity, With<crate::game::combat::Battle>>>,
    mut rng: ResMut<GameRng>,
) {
    let Ok((mut vitals, mut conditions, mut side, (traits, level, calling), tally, cheated)) =
        bodies.get_mut(fell.body)
    else {
        return;
    };
    let fight = fight.map(|fight| *fight);
    let first = fight.is_some_and(|fight| cheated != Some(&CheatedDeathIn(fight)));
    let mastery = calling.map_or(level.0 as u8, Calling::mastery);
    let cheats = vitals.is_dead()
        && vitals.cheats_death(&mut conditions, (traits, mastery), first, &mut rng);
    if cheats {
        if let (true, Some(fight)) = (first, fight) {
            commands.entity(fell.body).insert(CheatedDeathIn(fight));
        }
    } else if vitals.is_dead() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Attribute::*;
    use crate::content::items::EquipClass;
    use crate::game::actors::{Drained, Skills, Traits};

    fn fighter() -> Character {
        Character::new("Ragnar", Race::Human, Gender::Male, Profession::Fighter)
    }

    /// Practice counts towards a point every eighth success, only in skills
    /// the profession may train, and never past 100 (retail
    /// `PracticeCharacterSkill`).
    #[test]
    fn practice_raises_a_point_every_eight_successes() {
        let mut rng = crate::game::rng::GameRng::seeded(4);
        let practise = |sheet: &mut Character, rng: &mut GameRng| {
            practise_skill(
                (&sheet.calling, &sheet.abilities),
                (&mut sheet.training, &mut sheet.practice),
                Skill::FireMagic,
                400,
                rng,
            )
        };
        let mut mage = Character::new("Mage", Race::Human, Gender::Male, Profession::Mage);
        let before = mage.training.0[Skill::FireMagic];
        assert!(practise(&mut mage, &mut rng));
        assert!(mage.training.0[Skill::FireMagic] > before);
        assert!(!practise(&mut fighter(), &mut rng), "no books, no realm practice");
        mage.training.0[Skill::FireMagic] = 100;
        assert!(!practise(&mut mage, &mut rng));
        assert_eq!(mage.training.0[Skill::FireMagic], 100);
    }

    #[test]
    fn a_human_fighter_has_the_worked_example_numbers() {
        let fighter = fighter();
        assert_eq!(fighter.abilities.0, EnumMap::from_array([55, 45, 45, 50, 50, 45, 45]));
        assert_eq!((fighter.max_hp(), fighter.max_stamina(), fighter.capacity()), (18, 105, 1920));
        let a = Attributes::of_sheet(&fighter);
        let initiative = |load| {
            crate::game::actors::figures::trained_initiative(
                (1, &fighter.training),
                &a,
                &Skills::of_sheet(&fighter),
                load,
            )
        };
        assert_eq!((initiative(Load::None), initiative(Load::Heavy)), (9, 5));
        assert_eq!(load(&a, 12, 959), Load::None);
        assert_eq!(load(&a, 12, 960), Load::Light);
        assert_eq!(load(&a, 12, 1939), Load::Heavy, "integer percent: still 100");
        assert_eq!(load(&a, 12, 1940), Load::Extreme);
        assert_eq!(fighter.calling.experience_goal(&fighter.experience), 1000);
        // Profession and bonus skills are seeded; others are not.
        let close_combat = fighter.training.0[Skill::CloseCombat];
        assert!(Skills::of_sheet(&fighter)[Skill::CloseCombat] > close_combat);
        assert!(fighter.training.0[Skill::Sword] > 0);
        assert_eq!(fighter.training.0[Skill::LocksAndTraps], 0);
    }

    /// Hybrids cast from their fifth level, so their book skills open then;
    /// expert skills open with an attribute at a hundred.
    #[test]
    fn skill_availability_follows_profession_levels_and_attribute_mastery() {
        let mut fighter = fighter();
        assert!(fighter.trained().can_use(Skill::Sword));
        assert!(!fighter.trained().can_use(Skill::Music));
        assert!(!fighter.trained().can_use(Skill::PowerStrike));
        fighter.abilities.0[Strength] = 100;
        assert!(fighter.trained().can_use(Skill::PowerStrike));
        let mut samurai = Character::new("S", Race::Human, Gender::Male, Profession::Samurai);
        assert!(!samurai.trained().can_use(Skill::Wizardry));
        samurai.calling.levels[Profession::Samurai] = 5;
        assert!(samurai.trained().can_use(Skill::Wizardry));
        assert!(!samurai.trained().can_use(Skill::FireMagic));
    }

    #[test]
    fn experience_goals_double_then_grow_by_a_fifth() {
        let mut mage = Character::new("Mira", Race::Elf, Gender::Female, Profession::Mage);
        mage.calling.levels[Profession::Mage] = 3;
        mage.experience.last_goal = 2400;
        assert_eq!(mage.calling.experience_goal(&mage.experience), 1200 + 1200 + 2400);
        mage.calling.levels[Profession::Mage] = 11;
        mage.experience.last_goal = 1200 * (1 + 511);
        // Level 1 is one weight; levels 2 to 10 add 1, 2, ..., 256 weights.
        let at_ten = 1200 * (1 + 511);
        assert_eq!(mage.calling.experience_goal(&mage.experience), at_ten + 256 * 1200 * 12 / 10);
    }

    #[test]
    fn items_check_profession_race_gender_and_minimums() {
        let fighter = fighter();
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
                (&fighter.calling, &fighter.birth),
                &Attributes::of_sheet(&fighter),
                &Skills::of_sheet(&fighter),
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

    /// One with the trait cheats death (retail `CheatDeathRevive`): out
    /// cold for a few rounds instead, at half to all of their health the
    /// first time in a fight and a roll of what they had any other time;
    /// seven rounds or more out cold, or out cold for good, they die.
    #[test]
    fn a_valkyrie_cheats_death_unless_long_out_cold() {
        let valkyrie =
            Character::new("Brunhild", Race::Human, Gender::Female, Profession::Valkyrie);
        let (traits, maximum) = (Traits(valkyrie.traits()), valkyrie.max_hp());
        let mut rng = crate::game::rng::GameRng::seeded(9);
        // A blow just past their health, and death settled: cheated, or not.
        let mut struck = |vitals: &mut Vitals, conditions: &mut Conditions, first| {
            let hp = vitals.health.current;
            assert_eq!(vitals.hurt(hp + 1, None), hp, "the blow takes what they had");
            vitals.cheats_death(conditions, (&traits, 1), first, &mut rng)
        };
        // Not the first time in a fight: a roll of the health they had, at
        // least one, and a stint out cold (level one scales five to three).
        let (mut member, mut ailing) = (Vitals::new(&valkyrie), Conditions::default());
        member.health.current = 4;
        assert!(struck(&mut member, &mut ailing, false));
        assert!((1..=4).contains(&member.health.current), "{}", member.health.current);
        assert_eq!(ailing.lasting(Condition::Unconscious), Some(Lasting::Rounds(4)));
        // The first time in a fight, half their maximum or more.
        let (mut member, mut ailing) = (Vitals::new(&valkyrie), Conditions::default());
        assert!(struck(&mut member, &mut ailing, true));
        let hp = member.health.current;
        assert!(hp >= maximum / 2 && hp <= maximum, "{hp}");
        // Long out cold, or out cold for good, they die; so does anyone
        // without the trait.
        for lasting in [Lasting::Rounds(7), Lasting::Indefinite] {
            let mut member = Vitals::new(&valkyrie);
            let mut ailing = Conditions::from([(Condition::Unconscious, lasting)]);
            assert!(!struck(&mut member, &mut ailing, true));
            assert!(member.is_dead());
        }
        let (mut member, mut ailing) = (Vitals::new(&fighter()), Conditions::default());
        member.hurt(1000, None);
        assert!(!member.cheats_death(&mut ailing, (&Traits::default(), 1), true, &mut rng));
    }

    #[test]
    fn damage_tires_and_kills_and_the_dead_take_and_gain_nothing() {
        let mut fighter = Vitals::new(&fighter());
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
        let (mut fighter, mut parts) = (Vitals::new(&fighter()), Recovering::default());
        fighter.health.current = 1;
        fighter.stamina.current = 0;
        // (0.9 x 105 + 20) / 240 x 0.5 is about 0.24 a tick: 10 after 42.
        fighter.recover_stamina(&mut parts, 41, 0.5);
        assert_eq!(fighter.stamina.current, 9);
        fighter.recover_stamina(&mut parts, 1, 0.5);
        assert_eq!(fighter.stamina.current, 10);
        // (0.4 x 18 + 20) / 240 x 0.5 is about 0.057 a tick.
        fighter.recover_health(&mut parts, 17, 0.5);
        assert_eq!(fighter.health.current, 1);
        fighter.recover_health(&mut parts, 1, 0.5);
        assert_eq!(fighter.health.current, 2);
    }

    /// Whenever the most a body can have moves, what is left moves by as
    /// much, and health brought to nothing is death (retail
    /// `RecalculateCharacterHitPoints`, `RecalculateCharacterStamina`).
    /// Draining moves it at once, until the drain is lifted (retail
    /// `DamageCharacter`).
    #[test]
    fn what_is_left_moves_with_the_most() {
        let mut member = Vitals::full(18, 105);
        member.health.current = 10;
        member.set_most(24, 105);
        assert_eq!((member.health.current, member.health.max), (16, 24));
        member.set_most(18, 100);
        assert_eq!((member.health.current, member.stamina.current), (10, 100));
        let (mut ailing, mut drained) = (Conditions::default(), Drained::default());
        assert_eq!(member.drain_health(&mut ailing, &mut drained, 3, None), 3);
        assert_eq!((member.health.current, member.health.max, drained.health), (7, 15, 3));
        assert_eq!(member.stamina.current, 98, "two thirds of it tires");
        assert!(ailing.has(Condition::Drained));
        assert!(drained.lift(&mut ailing) && drained == Drained::default());
        assert!(!drained.lift(&mut ailing), "nothing left to lift");
        member.set_most(3, 100);
        assert!(member.is_dead(), "a maximum that falls past what is left kills");
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
