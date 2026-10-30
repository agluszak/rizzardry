//! Spellcasting rules: who can cast what, spell points per realm, whether a
//! cast comes off, and what the modelled spells do to their victims (retail
//! `Magic.cpp`, `Magic Effects.cpp`, `Health Stamina Mana.cpp`).
//!
//! A learned spell and an item's spell share effect resolution but keep
//! their own validation and payment: casting spends spell points, using an
//! item spends a use (see `game::magic::casting`).

pub mod casting;
pub(crate) mod dialogue;
pub mod effects;
pub mod enchantments;
pub(crate) mod impacts;
pub(crate) mod pointcasts;
pub mod portals;
pub(crate) mod sources;
pub(crate) mod targeting;
pub(crate) mod traps;
pub(crate) mod utilities;

pub use effects::{Effect, Field, Victim};

use crate::game::party::Members;

use std::collections::BTreeSet;

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::spells::{
    MAX_POWER, SpellCatalog, SpellDefinition, SpellId, Spellbook, Targeting, min_caster_level,
};
use crate::game::characters::{Character, Profession};
use crate::game::inventory::PartyInventory;
use crate::game::party::DamageApplied;
use crate::game::party::formation::Formation;
use crate::game::rng::GameRng;
use crate::game::world::damage::MonsterDamage;

/// The spells a character has learned. Creation picks the first ones.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct LearnedSpells(pub BTreeSet<SpellId>);

/// Skill eligibility combines profile rules with the actual learned spell set.
pub fn skill_available(
    character: &Character,
    learned: Option<&LearnedSpells>,
    catalog: &SpellCatalog,
    skill: Skill,
) -> bool {
    if skill as usize >= Skill::Wizardry as usize
        && (skill as usize) < Skill::PowerStrike as usize
        && character.traits().contains(crate::content::characters::Trait::CannotLearn)
    {
        return false;
    }
    if let Some(realm) = skill.realm() {
        if !profession_books(character.profession)
            .iter()
            .any(|book| character.skill_available(book_skill(book)))
        {
            return false;
        }
        return learned.is_some_and(|learned| {
            learned
                .0
                .iter()
                .any(|&id| catalog.get(id).is_some_and(|spell| spell.realm == Some(realm)))
        });
    }
    character.skill_available(skill)
}

/// Mana already spent and fractional recovery are history. The current pool is
/// the live maximum minus this deficit: retail's RebuildRealmSpellPointCeilings
/// adds each maximum change to current mana, preserving precisely this value.
/// A loss of capacity can leave signed current mana below zero; it is not a
/// fresh empty pool that a later capacity increase should refill for free.
/// Default means no spending: full mana at the actor's live capacity.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct SpellPoints {
    spent: EnumMap<Realm, i64>,
    pub fraction: EnumMap<Realm, f32>,
}

impl SpellPoints {
    pub fn current(&self, realm: Realm, maximum: &EnumMap<Realm, u32>) -> i64 {
        i64::from(maximum[realm]) - self.spent[realm]
    }

    pub fn available(&self, realm: Realm, maximum: &EnumMap<Realm, u32>) -> u32 {
        self.current(realm, maximum).clamp(0, i64::from(u32::MAX)) as u32
    }

    pub fn drain_realm(&mut self, realm: Realm, amount: u32, maximum: &EnumMap<Realm, u32>) -> u32 {
        let taken = amount.min(self.available(realm, maximum));
        self.spend(realm, taken);
        taken
    }

    /// Record spending after the caller has checked what is available.
    pub fn spend(&mut self, realm: Realm, amount: u32) {
        self.spent[realm] += i64::from(amount);
    }

    /// Drain randomly chosen realms, at most fifty draws (retail
    /// `DrainCharacterSpellPoints`). Fractional recovery is not spent.
    pub fn drain(&mut self, amount: u32, maximum: &EnumMap<Realm, u32>, rng: &mut GameRng) -> u32 {
        let mut remaining = amount;
        for _ in 0..50 {
            if remaining == 0 {
                break;
            }
            let realm = Realm::try_from(rng.below(6) as u8).unwrap();
            let taken = self.drain_realm(realm, remaining, maximum);
            remaining -= taken;
        }
        amount - remaining
    }

    /// Restore the largest realm deficits first (retail
    /// `RestoreCharacterSpellPointsEvenly`). Ties keep the initial deficit order
    /// (initial ties use realm order). Excess is wasted; recovery fractions stay.
    pub fn restore_evenly(&mut self, amount: u32) -> u32 {
        let mut deficits: Vec<_> =
            self.spent.iter().map(|(realm, &spent)| (realm, spent.max(0))).collect();
        deficits.sort_by_key(|&(_, missing)| std::cmp::Reverse(missing));
        let mut restored = 0;
        for _ in 0..amount {
            let largest = deficits.iter().map(|&(_, missing)| missing).max().unwrap_or(0);
            if largest == 0 {
                break;
            }
            let (realm, missing) =
                deficits.iter_mut().find(|(_, missing)| *missing == largest).unwrap();
            self.spent[*realm] -= 1;
            *missing -= 1;
            restored += 1;
        }
        restored
    }

    /// The highest power the realm's points pay for, 0 if none.
    pub fn affordable_power(&self, spell: &SpellDefinition, maximum: &EnumMap<Realm, u32>) -> u8 {
        let Some(realm) = spell.realm else { return 0 };
        if spell.cost == 0 {
            return MAX_POWER;
        }
        (self.available(realm, maximum) / spell.cost).min(u32::from(MAX_POWER)) as u8
    }

    /// Whether any realm is short of its maximum.
    pub fn short(&self) -> bool {
        self.spent.values().any(|&spent| spent > 0)
    }

    /// Full pools need no write unless a previous operation left a fraction
    /// for the aging boundary to clear.
    pub(crate) fn needs_recovery(&self) -> bool {
        self.short() || self.fraction.values().any(|&fraction| fraction != 0.0)
    }

    /// Recover over `ticks` aging ticks at `(0.65·max + 20)/240` per tick
    /// times `scale` (retail aging, `GameplayTime.cpp`).
    pub fn recover(&mut self, ticks: u32, scale: f32, maximum: &EnumMap<Realm, u32>) {
        for (realm, &max) in maximum {
            if self.spent[realm] <= 0 {
                self.fraction[realm] = 0.0;
                continue;
            }
            self.fraction[realm] += ticks as f32 * (0.65 * max as f32 + 20.0) / 240.0 * scale;
            let whole = self.fraction[realm].trunc();
            self.fraction[realm] -= whole;
            self.spent[realm] = (self.spent[realm] - whole as i64).max(0);
            if self.spent[realm] == 0 {
                self.fraction[realm] = 0.0;
            }
        }
    }
}

/// Select an available target by angle to the heading, then distance.
pub fn target_ahead(
    eye: Vec3,
    forward: Vec3,
    reach: f32,
    candidates: impl IntoIterator<Item = (Entity, Vec3, f32)>,
) -> Option<Entity> {
    candidates
        .into_iter()
        .filter(|(_, _, distance)| *distance <= reach)
        .map(|(entity, at, distance)| {
            (entity, (at - eye).with_y(0.0).angle_between(forward.with_y(0.0)), distance)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(entity, ..)| entity)
}

/// The share of spell points recovered while health recovers at
/// `health_scale`: never under half (retail `spellScale`).
pub fn recovery_scale(health_scale: f32) -> f32 {
    health_scale.max(0.5)
}

/// The books a profession casts from (retail `g_profession_spellbooks`).
pub fn profession_books(profession: Profession) -> EnumSet<Spellbook> {
    use Spellbook::*;
    match profession {
        Profession::Lord | Profession::Valkyrie | Profession::Priest => Divinity.into(),
        Profession::Ranger | Profession::Ninja | Profession::Alchemist => Alchemy.into(),
        Profession::Samurai | Profession::Mage => Wizardry.into(),
        Profession::Monk | Profession::Psionic => Psionics.into(),
        Profession::Bishop => EnumSet::all(),
        Profession::Fighter | Profession::Rogue | Profession::Gadgeteer | Profession::Bard => {
            EnumSet::empty()
        }
    }
}

/// A profession's caster level offset: 0 for the pure casters, -4 for the
/// hybrids, none for those who cannot cast (retail
/// `g_profession_magic_level_offsets`).
fn caster_offset(profession: Profession) -> Option<i32> {
    match profession {
        Profession::Fighter | Profession::Rogue | Profession::Gadgeteer | Profession::Bard => None,
        Profession::Priest
        | Profession::Alchemist
        | Profession::Bishop
        | Profession::Psionic
        | Profession::Mage => Some(0),
        _ => Some(-4),
    }
}

/// Retail `GetProfessionCasterLevel`; -1 for a profession that cannot cast.
pub fn caster_level(character: &Character, profession: Profession) -> i32 {
    profession_caster_level(profession, character.profession_levels[profession])
}

/// A profession's caster level after `levels` of it; -1 if it cannot cast.
fn profession_caster_level(profession: Profession, levels: u8) -> i32 {
    caster_offset(profession).map_or(-1, |offset| i32::from(levels) + offset)
}

/// Retail `GetTotalCasterLevel` over `books`: the current profession's caster
/// level plus every other held profession sharing a book.
pub fn total_caster_level(character: &Character, books: EnumSet<Spellbook>) -> i32 {
    let mut total = caster_level(character, character.profession);
    for (profession, &levels) in &character.profession_levels {
        if levels != 0
            && profession != character.profession
            && !(profession_books(profession) & books).is_empty()
        {
            total += caster_level(character, profession).max(0);
        }
    }
    total
}

pub(crate) fn book_skill(book: Spellbook) -> Skill {
    match book {
        Spellbook::Wizardry => Skill::Wizardry,
        Spellbook::Divinity => Skill::Divinity,
        Spellbook::Alchemy => Skill::Alchemy,
        Spellbook::Psionics => Skill::Psionics,
    }
}

/// The books the character has ever cast from.
fn character_books(character: &Character) -> EnumSet<Spellbook> {
    character
        .profession_levels
        .iter()
        .filter(|&(_, &levels)| levels > 0)
        .map(|(profession, _)| profession_books(profession))
        .fold(profession_books(character.profession), |all, books| all | books)
}

/// The book skill a spell is cast by: the best of the character's (retail
/// `GetBestSpellbookSkillForSpell`).
pub(crate) fn best_book(character: &Character, spell: &SpellDefinition) -> Skill {
    let books = spell.books & character_books(character);
    let books = if books.is_empty() { spell.books } else { books };
    books.iter().map(book_skill).max_by_key(|&s| character.skill(s)).unwrap_or(Skill::Wizardry)
}

/// Retail `GetSpellCastingSkillLevel`: the book counts once, the realm four
/// times.
fn casting_skill(character: &Character, spell: &SpellDefinition) -> u32 {
    let realm = spell.realm.map_or(0, |r| character.skill(r.skill()));
    (character.skill(best_book(character, spell)) + realm * 4) / 5
}

/// Whether the character may learn a spell now (retail `CanLearnSpell`): it is
/// in one of their books and its level is within both their caster level and
/// their realm and book training.
pub fn can_learn(character: &Character, spell: &SpellDefinition) -> bool {
    let books = profession_books(character.profession);
    if spell.id.0 == 0 || spell.id.0 >= SpellId::LEARNABLE || (spell.books & books).is_empty() {
        return false;
    }
    if character.traits().contains(crate::content::characters::Trait::CannotLearn)
        || caster_level(character, character.profession) <= 0
        || spell.realm.is_none()
    {
        return false;
    }
    let caster = total_caster_level(character, spell.books);
    let by_level = (1..=7u8).filter(|&l| min_caster_level(l) <= caster).max().unwrap_or(0);
    let realm_points = spell.realm.map_or(0, |r| u32::from(character.skill_points[r.skill()]));
    let book = [Spellbook::Wizardry, Spellbook::Divinity, Spellbook::Alchemy, Spellbook::Psionics]
        .into_iter()
        .filter(|book| spell.books.contains(*book))
        .max_by_key(|book| character.skill_points[book_skill(*book)]);
    let best = book.map_or(0, |book| character.skill(book_skill(book)));
    let by_skill = (realm_points / 10 + best) / 15 + 1;
    u32::from(spell.level) <= u32::from(by_level).min(by_skill)
}

/// How many first spells a new character picks: two for the pure casters,
/// none for the others (retail `CharGeneration.cpp`).
pub fn starting_picks(profession: Profession) -> usize {
    if profession_caster_level(profession, 1) > 0 { 2 } else { 0 }
}

impl crate::game::characters::CharacterStats<'_> {
    /// Spell points per realm (retail `RecalculateRealmSpellPoints`): none in
    /// a realm without a learned spell, and never under its dearest learned
    /// spell.
    pub fn max_points(
        &self,
        learned: &LearnedSpells,
        spells: &SpellCatalog,
    ) -> EnumMap<Realm, u32> {
        let character = self;
        let mut books: Vec<u32> =
            [Skill::Wizardry, Skill::Divinity, Skill::Alchemy, Skill::Psionics]
                .map(|s| character.skill(s))
                .to_vec();
        books.sort_unstable_by(|a, b| b.cmp(a));
        let weight = (books[0] as f32
            + (books[1] / 2) as f32
            + (books[2] / 4) as f32
            + (books[3] / 8) as f32)
            .min(125.0);
        let piety = character.attribute(Attribute::Piety) as f32;
        EnumMap::from_fn(|realm| {
            let known: Vec<_> = learned
                .0
                .iter()
                .filter_map(|&id| spells.get(id))
                .filter(|s| s.realm == Some(realm))
                .collect();
            if known.is_empty() {
                return 0;
            }
            let skill = character.skill(realm.skill()) as f32;
            let levels = (known.len() + usize::from(character.level()) + 1) as f32;
            let points = ((weight + 3.0 * skill + piety) * 0.02 * levels + 0.5) as u32;
            points.max(known.iter().map(|s| s.cost).max().unwrap_or(0))
        })
    }
}

/// The skill threshold a spell's difficulty band needs at power 7 (retail
/// `g_combat_effect_slot_spells_and_cast_success`).
const CAST_THRESHOLDS: [u32; 17] =
    [30, 40, 50, 58, 64, 70, 76, 81, 86, 90, 94, 97, 100, 102, 105, 107, 110];

/// Retail `GetSpellFailureChance`: the skill a spell's difficulty band
/// needs at `power`, and up to 70% failure for falling short of it.
pub(crate) fn spell_failure(skill: u32, spell: &SpellDefinition, power: u8) -> u32 {
    let band = (spell.cost / 2 + u32::from(spell.level)).min(16) as usize;
    let need = CAST_THRESHOLDS[band] * u32::from(power) / 7;
    if skill < need { ((need - skill) * 70 / need.max(1)).min(100) } else { 0 }
}

/// The percent chance a learned cast fails (retail `ExecuteCharacterSpellCast`),
/// before combat pace: the casting skill's shortfall, and the caster levels the
/// spell's level and power call for.
pub fn fail_chance(character: &Character, spell: &SpellDefinition, power: u8) -> u32 {
    let mut chance = spell_failure(casting_skill(character, spell), spell, power) as i32;
    let caster = total_caster_level(character, spell.books);
    let short = min_caster_level(spell.level) - caster - 1 + i32::from(power);
    if short > 0 {
        chance += i32::from(spell.level) * short;
    }
    chance.clamp(0, 100) as u32
}

/// Retail's Power Cast bonus: `(level >> 2) + 1` while the skill is available.
fn power_cast(character: &Character) -> u32 {
    if character.skill_available(Skill::PowerCast) {
        (character.skill(Skill::PowerCast) >> 2) + 1
    } else {
        0
    }
}

/// The strength a cast's victims save against, and the percent its rolls are
/// raised by (retail `CastSpellFromSource`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strength {
    pub power: u8,
    pub against: u32,
    pub boost: u32,
}

impl Strength {
    /// A character's cast: by their total caster level, raised by Power Cast.
    pub fn of(character: &Character, spell: &SpellDefinition, power: u8) -> Self {
        let caster = total_caster_level(character, spell.books).max(0) as u32;
        Strength::at(caster, spell, power, power_cast(character))
    }

    /// Retail `GetSpellDifficulty`: `figure/2 + power + (cost/2 + level)/2`,
    /// three less against all enemies, less the death spells' floors, then
    /// raised by half the boost.
    pub fn at(figure: u32, spell: &SpellDefinition, power: u8, boost: u32) -> Self {
        let mut against =
            figure / 2 + u32::from(power) + (spell.cost / 2 + u32::from(spell.level)) / 2;
        if spell.targeting == Targeting::AllEnemies {
            against = against.saturating_sub(3);
        }
        let floor = match spell.id {
            SpellId::INSTANT_DEATH => 2,
            SpellId::QUICKSAND => 4,
            SpellId::ASPHYXIATION | SpellId::DEATH_CLOUD | SpellId::DEATH_WISH => 6,
            _ => 0,
        };
        against = against.saturating_sub(floor);
        against += against * (boost / 2) / 100;
        Strength { power, against, boost }
    }
}

/// Whether a cast came off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cast,
    Fizzled,
    /// It turned on the caster.
    Backfired,
}

/// Roll a cast against its failure chance (retail `CastSpellFromSource`):
/// the same roll decides failure and backfire. Ordinary failure chances below
/// 6% cannot backfire; quiet (thrown) uses backfire whenever they fail and can.
/// Underwater Fire first overrides failure to 100%, then quiet halves it;
/// either case forbids backfire and keeps the ordinary payment transaction.
pub fn roll(
    rng: &mut GameRng,
    chance: u32,
    spell: &SpellDefinition,
    quiet: bool,
    underwater: bool,
) -> Outcome {
    let suppressed = underwater && spell.realm == Some(Realm::Fire);
    let chance = if suppressed { if quiet { 50 } else { 100 } } else { chance };
    if chance == 0 {
        return Outcome::Cast;
    }
    let rolled = rng.below(100);
    if rolled >= chance {
        return Outcome::Cast;
    }
    let backfire = if suppressed || !spell.can_backfire() || (!quiet && chance < 6) {
        0
    } else if quiet {
        chance
    } else {
        chance / 3
    };
    if rolled < backfire { Outcome::Backfired } else { Outcome::Fizzled }
}

/// Retail fatigue for a learned cast: the spell's level plus its power.
fn cast_fatigue(spell: &SpellDefinition, power: u8) -> u32 {
    u32::from(spell.level) + u32::from(power)
}

pub struct MagicPlugin;

impl Plugin for MagicPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MagicRequest>()
            .add_message::<casting::MagicNews>()
            .add_message::<crate::game::world::hostility::Provocation>()
            .init_resource::<casting::FlightOrder>()
            .add_systems(
                Update,
                (accept_magic, pointcasts::update_lure)
                    .chain()
                    .after(crate::game::GameplaySet::Stats)
                    .before(crate::game::GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            );
        // Missiles fly on the fixed step, ahead of the fight's step there
        // (see `CombatPlugin`).
    }
}

/// A member asks to cast or use an item. In a fight it becomes their order
/// for the next round; otherwise it happens at once.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct MagicRequest {
    pub caster: Entity,
    pub choice: casting::MagicChoice,
}

/// What resolving magic needs beside the members and monsters it changes.
#[derive(bevy::ecs::system::SystemParam)]
pub struct MagicContext<'w, 's> {
    pub roster: Single<'w, 's, &'static crate::game::party::roster::PartyRoster>,
    pub content: If<Res<'w, crate::game::data::Catalogs>>,
    difficulty: Res<'w, crate::game::difficulty::Difficulty>,
    pub formation: Single<'w, 's, &'static mut Formation>,
    pub party_damage: MessageWriter<'w, DamageApplied>,
    pub enchantments: Single<'w, 's, &'static mut enchantments::PartyEffects>,
    pub inventory: Single<'w, 's, &'static mut PartyInventory>,
    pub news: MessageWriter<'w, casting::MagicNews>,
    level: Single<'w, 's, Entity, With<crate::game::world::level::Level>>,
    level_rules: Query<'w, 's, &'static crate::game::world::triggers::LevelState>,
    visibility: crate::game::world::obstacles::WorldObstacles<'w, 's>,
    /// Missiles leave in launch order, so equal arrivals land in it too.
    pub flight_order: ResMut<'w, casting::FlightOrder>,
    provocations: MessageWriter<'w, crate::game::world::hostility::Provocation>,
    fight: Query<'w, 's, Entity, With<crate::game::combat::Battle>>,
}

impl MagicContext<'_, '_> {
    /// The level being played, which spell missiles belong to.
    pub fn level(&self) -> Entity {
        *self.level
    }

    /// Resolve magic at game time `now`: `cast` gets the field of members,
    /// monsters and party belongings it changes, and the spells.
    pub fn resolve<R>(
        &mut self,
        now: u64,
        members: &mut Members,
        monsters: &mut MonsterDamage,
        cast: impl FnOnce(&mut Field, &SpellCatalog) -> R,
    ) -> R {
        let party: Vec<Entity> = self.roster.iter().collect();
        let party_level = crate::game::party::average_level(
            members.iter_many(self.roster.iter()).matched().map(|(c, ..)| c.level()),
        );
        let items = &self.content.items;
        let mut field = Field {
            visibility: &self.visibility,
            underwater: self.level_rules.get(*self.level).is_ok_and(|state| state.camera_sway),
            members,
            monsters,
            enchantments: &mut self.enchantments.0,
            inventory: &mut self.inventory,
            formation: &mut self.formation,
            party_damage: &mut self.party_damage,
            items,
            species: &self.content.species,
            spells: &self.content.spells,
            party_level,
            now,
            difficulty: *self.difficulty,
            party: &party,
            improved: Vec::new(),
            provocations: &mut self.provocations,
            fight: self.fight.single().ok(),
        };
        cast(&mut field, &self.content.spells)
    }

    /// The field of members, monsters and party belongings that blows
    /// change.
    pub fn with_field<R>(
        &mut self,
        now: u64,
        members: &mut Members,
        monsters: &mut MonsterDamage,
        act: impl FnOnce(&mut Field) -> R,
    ) -> R {
        self.resolve(now, members, monsters, |field, _| act(field))
    }
}

/// Carry out magic that needs the whole world, announcing a refusal: a
/// utility spell (portals, dialogue, point casts, Identify) or a cast at a
/// device. The one way such magic executes, in or out of a fight.
pub(crate) fn execute_in_world(
    world: &mut World,
    caster: Entity,
    choice: casting::MagicChoice,
) -> Result<(), casting::Refusal> {
    let utility = utilities::source_spell(world, caster, choice.source).is_ok_and(utilities::spell);
    let result = match choice.aim {
        _ if utility => utilities::cast(world, caster, choice),
        casting::Aim::Device(attempt) => {
            crate::game::world::interaction::cast_device(world, caster, attempt, choice.source)
        }
        _ => Err(casting::Refusal::Unsupported),
    };
    if let Err(refusal) = result {
        world.write_message(casting::MagicNews::Refused { caster, refusal });
    }
    result
}

/// Accept magic requests (retail `ChooseAction`): in combat only record the
/// choice; outside it, execute it now.
pub fn accept_magic(
    mut requests: MessageReader<MagicRequest>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    interaction: Query<(), With<crate::game::world::interaction::DeviceInteraction>>,
    mut orders: Query<&mut crate::game::party::Order>,
    (mut members, mut monsters): (Members, MonsterDamage),
    mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
    mut context: MagicContext,
    mut rng: ResMut<GameRng>,
    party: Query<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>,
    mut commands: Commands,
    clock: Res<crate::game::time::GameClock>,
) {
    for &MagicRequest { caster, choice } in requests.read() {
        if !context.roster.contains(&caster) {
            continue;
        }
        let spell = choice.source.spell_id(
            members.get(caster).ok().map(|(_, _, carried, ..)| carried),
            &context.inventory,
            &context.content.items,
        );
        // A point cast in a fight waits for its turn, as any other order.
        let in_world = (spell.is_some_and(utilities::spell)
            && (battle.is_none() || !spell.is_some_and(pointcasts::spell)))
            || matches!(choice.aim, casting::Aim::Device(_));
        if in_world {
            commands.queue(move |world: &mut World| {
                let _ = execute_in_world(world, caster, choice);
            });
            continue;
        }
        if !interaction.is_empty() {
            context.news.write(casting::MagicNews::Refused {
                caster,
                refusal: casting::Refusal::WrongTime,
            });
            continue;
        }
        if battle.is_some() {
            if let Ok(mut order) = orders.get_mut(caster) {
                order.set_if_neq(crate::game::party::Order::Magic(choice));
            }
            continue;
        }
        let Ok((party_at, facing)) = party
            .single()
            .map(|(at, body)| (crate::game::party::body::PartyBody::eye(at), body.forward()))
        else {
            context.news.write(casting::MagicNews::Refused {
                caster,
                refusal: casting::Refusal::WrongTime,
            });
            continue;
        };
        let roster: Vec<Entity> = context.roster.iter().collect();
        // Outside a fight no monster is a target: the hostile spells are
        // combat spells.
        let situation = casting::Situation {
            context: if camp.is_some() {
                casting::UseContext::Camp
            } else {
                casting::UseContext::World
            },
            party: &roster,
            party_at,
            facing,
            monsters: &[],
            group_of: &|m| vec![m],
        };
        let level = context.level();
        let performed = context.resolve(clock.ms, &mut members, &mut monsters, |field, spells| {
            casting::perform(caster, choice, &situation, spells, &mut points, &mut rng, field)
        });
        context.news.write(match performed {
            Ok((report, flights)) => {
                context.flight_order.launch(&mut commands, flights, level);
                casting::MagicNews::Done(report)
            }
            Err(refusal) => casting::MagicNews::Refused { caster, refusal },
        });
    }
}

/// Fly spell missiles and resolve them on arrival (retail
/// `ResolveSpellMissileHit`): on the victims captured at the cast; one that
/// died meanwhile is missed. Kills in a fight count for it.
pub fn deliver_spells(
    mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
    party: Query<&avian3d::prelude::Position, With<crate::game::party::body::PartyBody>>,
    time: Res<Time<Fixed>>,
    mut flights: Query<(Entity, &mut casting::SpellInFlight)>,
    (mut members, mut monsters): (Members, MonsterDamage),
    mut context: MagicContext,
    mut rng: ResMut<GameRng>,
    mut battle: Option<Single<&mut crate::game::combat::Battle>>,
    mut combat_news: MessageWriter<crate::game::combat::CombatNews>,
    mut commands: Commands,
    clock: Res<crate::game::time::GameClock>,
) {
    if flights.is_empty() {
        return;
    }
    let party_at = party.single().ok().map(crate::game::party::body::PartyBody::eye);
    let spells = &context.content.spells;
    if flights.iter().any(|(_, flight)| {
        spells.get(flight.spell).is_none()
            || !impacts::ready(flight.spell, spells)
            || flight.spell == SpellId::BOILING_BLOOD && party_at.is_none()
    }) {
        return;
    }
    // Every impact due in this update, in the order they arrive.
    let mut due = Vec::new();
    for (entity, mut flight) in &mut flights {
        let left = flight.arrives.remaining();
        if flight.arrives.tick(time.delta()).is_finished() {
            due.push((left, flight.launched, entity));
        }
    }
    let mut due: Vec<_> = due
        .into_iter()
        .map(|(left, launched, entity)| {
            (left, launched, Some(entity), flights.get(entity).unwrap().1.clone())
        })
        .collect();
    while let Some((index, _)) =
        due.iter().enumerate().min_by_key(|(_, (left, launched, ..))| (*left, *launched))
    {
        let (at, _, entity, flight) = due.swap_remove(index);
        if let Some(entity) = entity {
            commands.entity(entity).despawn();
        }
        let mut secondary = Vec::new();
        let landed = context.resolve(clock.ms, &mut members, &mut monsters, |field, spells| {
            let spell = spells.get(flight.spell)?;
            let effects = impacts::resolve(
                flight.caster,
                flight.origin,
                false,
                spell,
                flight.strength,
                &flight.victims,
                flight.finish.as_ref(),
                party_at.unwrap_or_default(),
                spells,
                &mut rng,
                field,
                &mut points,
                &mut secondary,
            );
            Some((effects, std::mem::take(&mut field.improved)))
        });
        let Some((effects, improved)) = landed else { continue };
        if let Some(battle) = battle.as_mut() {
            battle.credit_kills(&effects, &mut combat_news);
        }
        context.news.write(casting::MagicNews::Landed {
            caster: flight.caster,
            spell: flight.spell,
            effects,
            improved,
        });
        for mut reflection in secondary {
            context.flight_order.stamp(&mut reflection);
            let arrival = at + reflection.arrives.remaining();
            if arrival <= time.delta() {
                due.push((arrival, reflection.launched, None, reflection));
            } else {
                reflection.arrives.tick(time.delta().saturating_sub(at));
                let level = context.level();
                context.flight_order.launch(&mut commands, vec![reflection], level);
            }
        }
    }
}

#[cfg(test)]
fn test_maximum(world: &World, member: Entity) -> EnumMap<Realm, u32> {
    let profile = world.get::<Character>(member).unwrap();
    let vitals = world.get::<crate::game::characters::Vitals>(member).unwrap();
    let learned = world.get::<LearnedSpells>(member).unwrap();
    let spells = &world.resource::<crate::game::data::Catalogs>().spells;
    profile.stats(&vitals.enchantments).max_points(learned, spells)
}

#[cfg(test)]
pub(crate) fn test_set_points(world: &mut World, member: Entity, realm: Realm, current: i64) {
    let maximum = test_maximum(world, member);
    world.get_mut::<SpellPoints>(member).unwrap().spent[realm] =
        i64::from(maximum[realm]) - current;
}

#[cfg(test)]
pub(crate) fn test_points(world: &World, member: Entity, realm: Realm) -> i64 {
    world.get::<SpellPoints>(member).unwrap().current(realm, &test_maximum(world, member))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::Dice;
    use crate::content::items::ItemCatalog;
    use crate::content::species::MonsterCatalog;
    use crate::content::spells::{PowerChoice, Reach, Usable};
    use crate::game::characters::{Gender, Race};

    pub(crate) fn heal_wounds() -> SpellDefinition {
        SpellDefinition {
            id: SpellId::HEAL_WOUNDS,
            name: "Heal Wounds".into(),
            realm: Some(Realm::Divine),
            level: 1,
            cost: 3,
            books: Spellbook::Divinity | Spellbook::Alchemy | Spellbook::Psionics,
            power: PowerChoice::ChosenOrRepeat,
            reach: Reach::Long,
            targeting: Targeting::Ally,
            usable: Usable::Always,
            dice: Some(Dice { base: 0, count: 4, sides: 4 }),
            ..default()
        }
    }

    /// The out-of-combat use path with real requests: a potion stack and a
    /// charged wand, each casting its own spell at its own power.
    #[test]
    fn underwater_fire_cannot_backfire_even_for_a_quiet_hostile_spell() {
        let spell = SpellDefinition {
            id: SpellId::ENERGY_BLAST,
            targeting: Targeting::Enemy,
            realm: Some(Realm::Fire),
            ..default()
        };
        assert!(spell.can_backfire());
        let mut rng = GameRng::seeded(1);
        for _ in 0..20 {
            assert_eq!(roll(&mut rng, 0, &spell, false, true), Outcome::Fizzled);
            assert_ne!(roll(&mut rng, 100, &spell, true, true), Outcome::Backfired);
        }
    }

    #[test]
    fn using_items_spends_a_dose_or_a_charge_and_the_last_dose_still_heals() {
        use crate::content::items::{
            EquipClass, ItemDefinition, ItemId, LootTables, QuantityKind, UseType,
        };
        use crate::content::spells::ItemSpell;
        use crate::game::characters::Vitals;
        use crate::game::inventory::{Carried, EquipSlot, Equipped, ItemIds, ItemStack};
        use casting::{Aim, MagicChoice, MagicNews, Refusal, Source};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<crate::game::factions::Factions>()
            .init_resource::<crate::game::time::GameClock>()
            .add_message::<crate::game::factions::FactionNews>()
            .init_resource::<ItemIds>()
            .init_resource::<crate::game::difficulty::Difficulty>()
            .init_resource::<avian3d::collider_tree::ColliderTrees>()
            .init_resource::<casting::FlightOrder>()
            .insert_resource(GameRng::seeded(3))
            .add_message::<DamageApplied>()
            .add_message::<MagicRequest>()
            .add_message::<casting::MagicNews>()
            .add_message::<crate::game::world::hostility::Provocation>()
            .add_systems(Update, accept_magic);
        let potion = ItemDefinition {
            id: ItemId(0),
            name: "Potion of Light Heal".into(),
            class: EquipClass::Potion,
            use_type: UseType::Potion,
            quantity: QuantityKind::Stack,
            stack_limit: 5,
            spell: Some(ItemSpell { spell: SpellId::HEAL_WOUNDS, power: 1 }),
            ..default()
        };
        let wand = ItemDefinition {
            id: ItemId(1),
            name: "Healing Wand".into(),
            armor_class_bonus: 1,
            class: EquipClass::MiscMagic,
            use_type: UseType::Charged,
            quantity: QuantityKind::Charges,
            professions: enumset::EnumSet::all(),
            races: enumset::EnumSet::all(),
            genders: enumset::EnumSet::all(),
            spell: Some(ItemSpell { spell: SpellId::HEAL_WOUNDS, power: 1 }),
            ..default()
        };
        let mut ids = ItemIds::default();
        let potions = ItemStack::new(&potion, &mut ids, 2, true);
        let charged = ItemStack::new(&wand, &mut ids, 1, true);
        let (potion_id, wand_id) = (potions.id, charged.id);
        let world = app.world_mut();
        let items = ItemCatalog(vec![potion, wand]);
        let species = MonsterCatalog::default();
        let loot = LootTables::default();
        let spells =
            SpellCatalog((0..=6).map(|i| if i == 6 { heal_wounds() } else { default() }).collect());
        world.insert_resource(crate::game::data::Catalogs {
            items,
            loot,
            species,
            spells,
            ..default()
        });
        let profile = Character::new("Drinker", Race::Human, Gender::Male, Profession::Fighter);
        let mut vitals = Vitals::new(&profile);
        vitals.hp = 1;
        let mut carried = Carried::default();
        carried.backpack[0] = Some(potions);
        carried.equipment[EquipSlot::Misc1] = Some(Equipped::new(charged));
        let member = world.spawn((profile, vitals, carried)).id();
        crate::game::party::roster::test_party(world, &[member]);
        world.spawn(crate::game::world::level::Level::test(default()));
        world.spawn((
            crate::game::party::body::PartyBody::default(),
            avian3d::prelude::Position::default(),
        ));
        let mut news = Vec::new();
        let mut request = |app: &mut App, source| {
            let choice = MagicChoice { source, aim: Aim::Member(member) };
            app.world_mut().write_message(MagicRequest { caster: member, choice });
            app.update();
            news.extend(app.world_mut().resource_mut::<Messages<MagicNews>>().drain());
            news.last().cloned()
        };
        let hp = |app: &App| app.world().get::<Vitals>(member).unwrap().hp;
        let backpack = |app: &App| app.world().get::<Carried>(member).unwrap().backpack[0].clone();
        assert!(matches!(request(&mut app, Source::Item(potion_id)), Some(MagicNews::Done(_))));
        assert_eq!(backpack(&app).unwrap().count(), 1);
        let before = hp(&app);
        assert!(before > 1, "4d4 heals at power 1");
        // Hurt again, and drink the last one: it is gone, and it heals.
        app.world_mut().get_mut::<Vitals>(member).unwrap().hp = 1;
        assert!(matches!(request(&mut app, Source::Item(potion_id)), Some(MagicNews::Done(_))));
        assert!(backpack(&app).is_none(), "the last dose is used up");
        assert!(hp(&app) > 1, "the effect outlives the potion");
        assert!(matches!(
            request(&mut app, Source::Item(potion_id)),
            Some(MagicNews::Refused { refusal: Refusal::NoItem, .. })
        ));
        // A charged item keeps its body when emptied, and refuses then.
        assert!(matches!(request(&mut app, Source::Item(wand_id)), Some(MagicNews::Done(_))));
        let carried = app.world().get::<Carried>(member).unwrap();
        assert_eq!(carried.worn(EquipSlot::Misc1).unwrap().remaining_uses(), Some(0));
        assert!(matches!(
            request(&mut app, Source::Item(wand_id)),
            Some(MagicNews::Refused { refusal: Refusal::NoCharges, .. })
        ));
    }

    /// Spell points pay for a learned cast whatever its roll; a refusal
    /// spends nothing.
    #[test]
    fn a_learned_cast_pays_even_when_it_fizzles_and_a_refusal_pays_nothing() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let heal = heal_wounds();
        let mut rng = GameRng::seeded(9);
        // An untrained priest at power 7: a high failure chance.
        let chance = fail_chance(&priest, &heal, 7);
        assert!(chance > 50, "{chance}");
        let outcomes: Vec<_> =
            (0..50).map(|_| roll(&mut rng, chance, &heal, false, false)).collect();
        assert!(outcomes.contains(&Outcome::Fizzled) && outcomes.contains(&Outcome::Cast));
    }

    #[test]
    fn a_priest_picks_two_first_level_divine_spells_and_gains_their_points() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let fighter = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let heal = heal_wounds();
        assert_eq!((starting_picks(priest.profession), starting_picks(fighter.profession)), (2, 0));
        assert!(can_learn(&priest, &heal) && !can_learn(&fighter, &heal));
        let higher = SpellDefinition { level: 2, ..heal_wounds() };
        assert!(!can_learn(&priest, &higher), "level 1 casters learn level 1 spells only");
        let catalog =
            SpellCatalog((0..=6).map(|i| if i == 6 { heal_wounds() } else { default() }).collect());
        let learned = LearnedSpells([SpellId::HEAL_WOUNDS].into());
        let points = priest.base_stats().max_points(&learned, &catalog);
        assert!(points[Realm::Divine] >= 3, "never under the dearest learned spell");
        assert_eq!(points[Realm::Fire], 0, "no spell, no points");
    }

    #[test]
    fn mana_restoration_levels_deficits_and_wastes_excess_without_resetting_recovery() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Fire] = 10;
        maximum[Realm::Water] = 5;
        maximum[Realm::Air] = 3;
        let mut points = SpellPoints { spent: maximum.map(|_, max| i64::from(max)), ..default() };
        points.fraction[Realm::Fire] = 0.75;
        assert_eq!(points.restore_evenly(8), 8);
        // Five go to Fire; then the equal deficits share, in realm order.
        assert_eq!(
            (points.current(Realm::Fire, &maximum), points.current(Realm::Water, &maximum)),
            (7, 1)
        );
        assert_eq!(points.current(Realm::Air, &maximum), 0);
        assert_eq!(points.restore_evenly(100), 10);
        assert!(!points.short());
        assert_eq!(points.fraction[Realm::Fire], 0.75);
        assert_eq!(points.restore_evenly(5), 0);
    }

    #[test]
    fn restorative_and_enchantment_items_pay_once_and_keep_their_target() {
        use crate::content::items::{
            Dice, EquipClass, ItemDefinition, ItemId, LootTables, QuantityKind, UseType,
        };
        use crate::content::spells::ItemSpell;
        use crate::game::characters::{Condition, Lasting, Vitals};
        use crate::game::inventory::{Carried, ItemIds, ItemStack};
        use casting::{Aim, MagicChoice, MagicNews, Source};

        for id in [
            SpellId::GUARDIAN_ANGEL,
            SpellId::ARMORPLATE,
            SpellId::HASTE,
            SpellId::SUPERMAN,
            SpellId::BODY_OF_STONE,
            SpellId::RESTORE_HEALTH,
            SpellId::RESTORE_MAGIC,
            SpellId::SMELLING_SALTS,
            SpellId::CURE_POISON,
        ] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default()))
                .init_resource::<crate::game::factions::Factions>()
                .init_resource::<crate::game::time::GameClock>()
                .add_message::<crate::game::factions::FactionNews>()
                .init_resource::<ItemIds>()
                .init_resource::<crate::game::difficulty::Difficulty>()
                .init_resource::<avian3d::collider_tree::ColliderTrees>()
                .init_resource::<casting::FlightOrder>()
                .insert_resource(GameRng::seeded(3))
                .add_message::<DamageApplied>()
                .add_message::<MagicRequest>()
                .add_message::<MagicNews>()
                .add_message::<crate::game::world::hostility::Provocation>()
                .add_systems(Update, accept_magic);
            let definition = ItemDefinition {
                id: ItemId(0),
                class: EquipClass::Potion,
                use_type: if id == SpellId::SMELLING_SALTS {
                    UseType::Thrown
                } else {
                    UseType::Potion
                },
                quantity: QuantityKind::Stack,
                stack_limit: 5,
                professions: EnumSet::all(),
                races: EnumSet::all(),
                genders: EnumSet::all(),
                spell: Some(ItemSpell { spell: id, power: 4 }),
                ..default()
            };
            let stack = ItemStack::new(&definition, &mut ItemIds::default(), 2, true);
            let stack_id = stack.id;
            let world = app.world_mut();
            let items = ItemCatalog(vec![definition]);
            let species = MonsterCatalog::default();
            let loot = LootTables::default();
            let mut definitions = vec![SpellDefinition::default(); 117];
            definitions[usize::from(SpellId::HEAL_WOUNDS.0)] = heal_wounds();
            definitions[usize::from(id.0)] = SpellDefinition {
                id,
                realm: Some(Realm::Divine),
                targeting: Targeting::Ally,
                dice: Some(Dice { base: 0, count: 1, sides: 1 }),
                ..default()
            };
            let spells = SpellCatalog(definitions);
            world.insert_resource(crate::game::data::Catalogs {
                items,
                loot,
                species,
                spells,
                ..default()
            });
            let mut user = Character::new("User", Race::Human, Gender::Male, Profession::Priest);
            user.skill_points[Skill::ThrowingAndSling] = 100;
            let mut carried = Carried::default();
            carried.backpack[0] = Some(stack);
            let user = world.spawn((Vitals::new(&user), user, carried)).id();
            let ally = Character::new("Ally", Race::Human, Gender::Male, Profession::Priest);
            let mut vitals = Vitals::new(&ally);
            vitals.hp = 1;
            vitals.stamina = 1;
            vitals.inflict(Condition::Asleep, Lasting::Rounds(8));
            vitals.inflict(Condition::Unconscious, Lasting::Rounds(8));
            vitals.poison(Lasting::Rounds(20), 10);
            let ally = world
                .spawn((
                    ally,
                    vitals,
                    Carried::default(),
                    LearnedSpells([SpellId::HEAL_WOUNDS].into()),
                    SpellPoints::default(),
                ))
                .id();
            test_set_points(world, ally, Realm::Divine, 0);
            crate::game::party::roster::test_party(world, &[user, ally]);
            world.spawn(crate::game::world::level::Level::test(default()));
            world.spawn((
                crate::game::party::body::PartyBody::default(),
                avian3d::prelude::Position::default(),
            ));
            let user_before = world.get::<Vitals>(user).unwrap().clone();
            world.write_message(MagicRequest {
                caster: user,
                choice: MagicChoice { source: Source::Item(stack_id), aim: Aim::Member(ally) },
            });
            app.update();
            let world = app.world_mut();
            let news: Vec<_> = world.resource_mut::<Messages<MagicNews>>().drain().collect();
            assert!(matches!(news.as_slice(), [MagicNews::Done(_)]), "{id:?}: {news:?}");
            assert_eq!(
                world.get::<Carried>(user).unwrap().backpack[0].as_ref().unwrap().count(),
                1
            );
            let user_after = world.get::<Vitals>(user).unwrap();
            assert_eq!(user_after.hp, user_before.hp);
            assert_eq!(user_after.conditions, user_before.conditions);
            let fatigue = user_before.stamina - user_after.stamina;
            if id == SpellId::SMELLING_SALTS {
                assert_eq!(fatigue, 10);
            } else {
                assert!((1..=2).contains(&fatigue), "using an item still pays its normal fatigue");
            }
            let vitals = world.get::<Vitals>(ally).unwrap();
            match id {
                SpellId::GUARDIAN_ANGEL => {
                    assert!(world.get::<Vitals>(user).unwrap().enchantments.0.is_empty());
                    assert_eq!(
                        vitals
                            .enchantments
                            .get(crate::content::spells::SpellId::GUARDIAN_ANGEL)
                            .unwrap()
                            .absorption,
                        4
                    );
                    let profile = world.get::<Character>(ally).unwrap().clone();
                    let mut vitals = world.get_mut::<Vitals>(ally).unwrap();
                    let stamina = vitals.stamina;
                    let mut rng = GameRng::default();
                    let mut fate = crate::game::characters::Fate { fight: None, rng: &mut rng };
                    assert_eq!(vitals.damage(&profile, 4, &mut fate), 0);
                    assert_eq!(vitals.stamina, stamina, "absorbed damage does not fatigue");
                    assert_eq!(vitals.damage(&profile, 2, &mut fate), 1);
                    assert!(vitals.is_dead());
                    assert!(vitals.enchantments.0.is_empty());
                }
                SpellId::ARMORPLATE => {
                    assert!(vitals.enchantments.0.is_empty());
                    let party = crate::game::party::roster::the_party(world).unwrap();
                    assert_eq!(
                        world.get::<enchantments::PartyEffects>(party).unwrap().0.armour(),
                        2
                    );
                }
                SpellId::HASTE | SpellId::SUPERMAN | SpellId::BODY_OF_STONE => {
                    let turns = vitals.enchantments.get(id).unwrap().turns;
                    let mut vitals = world.get_mut::<Vitals>(ally).unwrap();
                    vitals.age_conditions(turns, &mut GameRng::default());
                    assert!(vitals.enchantments.get(id).is_none());
                }
                SpellId::RESTORE_HEALTH => {
                    assert_eq!(vitals.hp, 5, "power four heals 4d1");
                    assert_eq!(vitals.stamina, 9, "stamina rolls twice the dice, independently");
                }
                SpellId::RESTORE_MAGIC => {
                    assert!(test_points(world, ally, Realm::Divine) > 0);
                    assert_eq!((vitals.hp, vitals.stamina), (1, 1));
                }
                SpellId::SMELLING_SALTS => {
                    assert!(
                        !vitals.conditions.has(Condition::Asleep)
                            && !vitals.conditions.has(Condition::Unconscious)
                    )
                }
                // Power four takes sixteen rounds off, and the poison weakens
                // by the same share, as aging would (retail
                // `TickCharacterCondition`).
                SpellId::CURE_POISON => {
                    assert_eq!(
                        vitals.conditions.lasting(Condition::Poisoned),
                        Some(Lasting::Rounds(4))
                    );
                    assert_eq!(vitals.poison, 2);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn lowered_capacity_keeps_debt_until_the_capacity_returns() {
        let mut high = EnumMap::default();
        high[Realm::Divine] = 20;
        let mut low = high;
        low[Realm::Divine] = 10;
        let mut points = SpellPoints::default();
        points.spend(Realm::Divine, 19);
        points.fraction[Realm::Divine] = 0.5;
        assert_eq!(points.current(Realm::Divine, &low), -9);
        assert_eq!(points.available(Realm::Divine, &low), 0);
        assert_eq!(points.drain_realm(Realm::Divine, 5, &low), 0);
        let spell = SpellDefinition { realm: Some(Realm::Divine), cost: 1, ..default() };
        assert_eq!(points.affordable_power(&spell, &low), 0);
        assert_eq!(points.current(Realm::Divine, &high), 1);
        assert_eq!(points.fraction[Realm::Divine], 0.5);
    }

    #[test]
    fn points_recover_by_retail_rate_and_never_beyond_the_maximum() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 20;
        let mut points = SpellPoints { spent: maximum.map(|_, max| i64::from(max)), ..default() };
        // (0.65·20 + 20)/240 = 0.1375 a tick at half rate: 0.06875.
        points.recover(100, recovery_scale(0.0), &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 6);
        points.recover(10_000, 1.0, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 20);
        assert_eq!(points.fraction[Realm::Divine], 0.0);
        points.spent[Realm::Divine] = -5;
        points.fraction[Realm::Divine] = 0.5;
        points.recover(1, 1.0, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 25, "retail skips full realms");
        assert_eq!(points.fraction[Realm::Divine], 0.0);
    }

    #[test]
    fn idle_recovery_keeps_surplus_but_clears_its_fraction() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 10;
        let mut points = SpellPoints::default();
        points.spent[Realm::Divine] = -5;
        points.fraction[Realm::Divine] = 0.5;
        points.recover(1, 0.5, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 15);
        assert_eq!(points.fraction[Realm::Divine], 0.0);
        assert!(!points.needs_recovery());
        points.spend(Realm::Divine, 6);
        points.recover(100, 0.5, &maximum);
        assert_eq!(points.current(Realm::Divine, &maximum), 10);
    }

    #[test]
    fn affordable_power_is_points_over_cost_within_seven() {
        let mut points = SpellPoints::default();
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 100;
        points.spent[Realm::Divine] = 90;
        assert_eq!(points.affordable_power(&heal_wounds(), &maximum), 3);
        points.spent[Realm::Divine] = 0;
        assert_eq!(points.affordable_power(&heal_wounds(), &maximum), 7);
    }

    #[test]
    fn failure_and_backfire_share_one_roll_and_guaranteed_casts_draw_none() {
        let spell =
            SpellDefinition { id: SpellId::ENERGY_BLAST, targeting: Targeting::Enemy, ..default() };
        let mut rng = GameRng::seeded(17);
        let mut expected = rng.clone();
        let rolled = expected.below(100);
        let outcome = if rolled < 33 { Outcome::Backfired } else { Outcome::Fizzled };
        assert_eq!(roll(&mut rng, 100, &spell, false, false), outcome);
        assert_eq!(rng, expected, "backfire must not consume a second random draw");
        assert_eq!(roll(&mut rng, 0, &spell, false, false), Outcome::Cast);
        assert_eq!(rng, expected, "a guaranteed cast consumes no failure draw");
        expected.below(100);
        assert_eq!(roll(&mut rng, 100, &spell, true, false), Outcome::Backfired);
        assert_eq!(rng, expected);
    }

    #[test]
    fn healing_cannot_backfire_and_an_untrained_caster_fails_more_at_higher_power() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let heal = heal_wounds();
        assert!(fail_chance(&priest, &heal, 7) > fail_chance(&priest, &heal, 1));
        let mut rng = GameRng::seeded(1);
        for _ in 0..200 {
            assert_ne!(roll(&mut rng, 100, &heal, false, false), Outcome::Backfired);
        }
    }
}
