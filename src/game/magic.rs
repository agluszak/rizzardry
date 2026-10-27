//! Spellcasting rules: who can cast what, spell points per realm, whether a
//! cast comes off, and what the modelled spells do to their victims (retail
//! `Magic.cpp`, `Magic Effects.cpp`, `Health Stamina Mana.cpp`).
//!
//! A learned spell and an item's spell share effect resolution but keep
//! their own validation and payment: casting spends spell points, using an
//! item spends a use (see `game::magic::casting`).

pub mod casting;

use std::collections::BTreeSet;

use bevy::prelude::*;
use enum_map::EnumMap;
use enumset::EnumSet;

use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::items::{Dice, ItemCatalog};
use crate::content::species::MonsterCatalog;
use crate::content::spells::{
    MAX_POWER, SpellCatalog, SpellDefinition, SpellId, Spellbook, Targeting, min_caster_level,
};
use crate::game::characters::{
    Character, Condition, Lasting, Profession, Vitals, inflict, resistance::resistances,
    wake_on_hit,
};
use crate::game::inventory::Carried;
use crate::game::party::DamageApplied;
use crate::game::party::formation::Formation;
use crate::game::rng::GameRng;
use crate::game::world::damage::{DamageOutcome, MonsterDamage};

/// The spells a character has learned. Creation picks the first ones.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct LearnedSpells(pub BTreeSet<SpellId>);

/// Spell points left per realm, and recovery not yet whole points. The
/// maximum is derived from the profile and learned spells ([`max_points`]).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct SpellPoints {
    pub current: EnumMap<Realm, u32>,
    pub fraction: EnumMap<Realm, f32>,
}

impl SpellPoints {
    pub fn full(maximum: EnumMap<Realm, u32>) -> Self {
        SpellPoints { current: maximum, ..default() }
    }

    /// The highest power the realm's points pay for, 0 if none.
    pub fn affordable_power(&self, spell: &SpellDefinition) -> u8 {
        let Some(realm) = spell.realm else { return 0 };
        if spell.cost == 0 {
            return MAX_POWER;
        }
        (self.current[realm] / spell.cost).min(u32::from(MAX_POWER)) as u8
    }

    /// Recover over `ticks` aging ticks at `(0.65·max + 20)/240` per tick
    /// times `scale` (retail aging, `GameplayTime.cpp`).
    pub fn recover(&mut self, maximum: &EnumMap<Realm, u32>, ticks: u32, scale: f32) {
        for (realm, &max) in maximum {
            if max == 0 || self.current[realm] >= max {
                self.fraction[realm] = 0.0;
                self.current[realm] = self.current[realm].min(max);
                continue;
            }
            self.fraction[realm] += ticks as f32 * (0.65 * max as f32 + 20.0) / 240.0 * scale;
            let whole = self.fraction[realm].trunc();
            self.fraction[realm] -= whole;
            self.current[realm] = (self.current[realm] + whole as u32).min(max);
            if self.current[realm] >= max {
                self.fraction[realm] = 0.0;
            }
        }
    }
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
    caster_offset(profession)
        .map_or(-1, |offset| i32::from(character.profession_levels[profession]) + offset)
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

fn book_skill(book: Spellbook) -> Skill {
    match book {
        Spellbook::Wizardry => Skill::Wizardry,
        Spellbook::Divinity => Skill::Divinity,
        Spellbook::Alchemy => Skill::Alchemy,
        Spellbook::Psionics => Skill::Psionics,
    }
}

fn realm_skill(realm: Realm) -> Skill {
    match realm {
        Realm::Fire => Skill::FireMagic,
        Realm::Water => Skill::WaterMagic,
        Realm::Air => Skill::AirMagic,
        Realm::Earth => Skill::EarthMagic,
        Realm::Mental => Skill::MentalMagic,
        Realm::Divine => Skill::DivineMagic,
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

/// The character's best book skill for a spell (retail
/// `GetBestSpellbookSkillForSpell`).
fn best_book_skill(character: &Character, spell: &SpellDefinition) -> u32 {
    let books = spell.books & character_books(character);
    let books = if books.is_empty() { spell.books } else { books };
    books.iter().map(|b| character.skill(book_skill(b))).max().unwrap_or(0)
}

/// Retail `GetSpellCastingSkillLevel`: the book counts once, the realm four
/// times.
fn casting_skill(character: &Character, spell: &SpellDefinition) -> u32 {
    let realm = spell.realm.map_or(0, |r| character.skill(realm_skill(r)));
    (best_book_skill(character, spell) + realm * 4) / 5
}

/// Whether the character may learn a spell now (retail `CanLearnSpell`): it is
/// in one of their books and its level is within both their caster level and
/// their realm and book training.
pub fn can_learn(character: &Character, spell: &SpellDefinition) -> bool {
    let books = profession_books(character.profession);
    if spell.id.0 == 0 || spell.id.0 >= SpellId::LEARNABLE || (spell.books & books).is_empty() {
        return false;
    }
    let caster = total_caster_level(character, books);
    let by_level = (1..=7u8).filter(|&l| min_caster_level(l) <= caster).max().unwrap_or(0);
    let realm_points = spell.realm.map_or(0, |r| u32::from(character.skill_points[realm_skill(r)]));
    let by_skill = (realm_points / 10 + best_book_skill(character, spell)) / 15 + 1;
    u32::from(spell.level) <= u32::from(by_level).min(by_skill)
}

/// How many first spells a new character picks: two for the pure casters,
/// none for the others (retail `CharGeneration.cpp`).
pub fn starting_picks(character: &Character) -> usize {
    if caster_level(character, character.profession) > 0 { 2 } else { 0 }
}

/// Spell points per realm (retail `RecalculateRealmSpellPoints`): none in a
/// realm without a learned spell, and never under its dearest learned spell.
pub fn max_points(
    character: &Character,
    learned: &LearnedSpells,
    spells: &SpellCatalog,
) -> EnumMap<Realm, u32> {
    let mut books: Vec<u32> = [Skill::Wizardry, Skill::Divinity, Skill::Alchemy, Skill::Psionics]
        .map(|s| character.skill(s))
        .to_vec();
    books.sort_unstable_by(|a, b| b.cmp(a));
    let weight =
        (books[0] as f32 + (books[1] / 2) as f32 + (books[2] / 4) as f32 + (books[3] / 8) as f32)
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
        let skill = character.skill(realm_skill(realm)) as f32;
        let levels = (known.len() + usize::from(character.level) + 1) as f32;
        let points = ((weight + 3.0 * skill + piety) * 0.02 * levels + 0.5) as u32;
        points.max(known.iter().map(|s| s.cost).max().unwrap_or(0))
    })
}

/// The skill threshold a spell's difficulty band needs at power 7 (retail
/// `g_combat_effect_slot_spells_and_cast_success`).
const CAST_THRESHOLDS: [u32; 17] =
    [30, 40, 50, 58, 64, 70, 76, 81, 86, 90, 94, 97, 100, 102, 105, 107, 110];

/// The percent chance a learned cast fails (retail `ExecuteCharacterSpellCast`).
/// Combat pace and difficulty are not modelled; retail's Normal setting
/// leaves a cast that is not late in the round unchanged.
pub fn fail_chance(character: &Character, spell: &SpellDefinition, power: u8) -> u32 {
    let band = (spell.cost / 2 + u32::from(spell.level)).min(16) as usize;
    let need = CAST_THRESHOLDS[band] * u32::from(power) / 7;
    let skill = casting_skill(character, spell);
    let mut chance =
        if skill < need { ((need - skill) * 70 / need.max(1)).min(100) as i32 } else { 0 };
    let caster = total_caster_level(character, spell.books);
    let short = min_caster_level(spell.level) - caster - 1 + i32::from(power);
    if short > 0 {
        chance += i32::from(spell.level) * short;
    }
    chance.clamp(0, 100) as u32
}

/// Retail's Power Cast bonus: `(level >> 2) + 1` while the skill is available.
pub fn power_cast(character: &Character) -> u32 {
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
    /// A character's cast: `caster/2 + power + (cost/2 + level)/2`, three
    /// less against all enemies, then raised by half the Power Cast bonus.
    pub fn of(character: &Character, spell: &SpellDefinition, power: u8) -> Self {
        let boost = power_cast(character);
        let caster = total_caster_level(character, spell.books).max(0) as u32;
        let mut against =
            caster / 2 + u32::from(power) + (spell.cost / 2 + u32::from(spell.level)) / 2;
        if spell.targeting == Targeting::AllEnemies {
            against = against.saturating_sub(3);
        }
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
/// a failure backfires a third of the time it could, never under 6%.
pub fn roll(rng: &mut GameRng, chance: u32, spell: &SpellDefinition) -> Outcome {
    if rng.below(100) >= chance {
        return Outcome::Cast;
    }
    let backfire = if chance < 6 || !spell.can_backfire() { 0 } else { chance / 3 };
    if rng.below(100) < backfire { Outcome::Backfired } else { Outcome::Fizzled }
}

/// The magnitude a spell rolls at `power`: its dice count times power, then
/// raised by the boost percent.
fn magnitude(rng: &mut GameRng, spell: &SpellDefinition, strength: Strength) -> u32 {
    let Some(dice) = spell.dice else { return 0 };
    let count = (u32::from(dice.count) * u32::from(strength.power)).min(255) as u8;
    let rolled = rng.roll(Dice { count, ..dice });
    rolled + rolled * strength.boost / 100
}

/// Retail `ReduceMagnitudeByResistance`: resistance and the victim's level
/// over the cast's strength take a random share off.
fn resist(rng: &mut GameRng, amount: u32, resistance: i32, level: u32, against: u32) -> u32 {
    let mut c = resistance + (level as i32 - against as i32) * 3;
    if c <= 0 {
        return amount;
    }
    c += 1;
    let half = c / 2;
    let share =
        (rng.below((c - half) as u32) as i32 + half + rng.below(half as u32) as i32).min(100);
    amount - amount * share as u32 / 100
}

/// Each condition's base save (retail `g_condition_resist_base`), by retail
/// number (Drained is 1).
const CONDITION_SAVE: [i32; 20] =
    [0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 18, 22, 24, 22];

fn condition_number(condition: Condition) -> usize {
    condition as usize + 1
}

/// Whether a victim shrugs a condition off (retail `ResolveAttackOnTarget`
/// on Normal difficulty): `(level − strength)·3 + base + resistance`
/// percent, within 5..=95.
fn saves(
    rng: &mut GameRng,
    condition: Condition,
    resistance: i32,
    level: u32,
    against: u32,
) -> bool {
    let chance = (level as i32 - against as i32) * 3
        + CONDITION_SAVE[condition_number(condition)]
        + resistance;
    (rng.below(100) as i32) < chance.clamp(5, 95)
}

/// Rounds a condition spell inflicts (retail `RollConditionDuration`).
fn duration(rng: &mut GameRng, spell: &SpellDefinition, strength: Strength) -> u32 {
    let mut rounds =
        (spell.duration_per_power * i32::from(strength.power) + spell.duration_base + 1).max(0)
            as u32;
    if rng.below(4) == 0 && rounds > 1 {
        rounds += 1;
    }
    if rng.below(4) == 0 && rounds <= 2 {
        rounds += 1;
    }
    rounds + rounds * strength.boost / 100
}

/// Whom a spell lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Victim {
    Member(Entity),
    Monster(Entity),
}

/// What a spell did to one victim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Healed { victim: Victim, amount: u32 },
    Damaged { victim: Victim, amount: u32, outcome: DamageOutcome },
    Inflicted { victim: Victim, condition: Condition },
    Resisted { victim: Victim },
    Nothing { victim: Victim },
}

/// The party members' profile, vitals and belongings, as spells and combat
/// change them.
pub type Members<'w, 's> =
    Query<'w, 's, (&'static mut Character, &'static mut Vitals, &'static mut Carried)>;

/// What a spell's effects need from the world.
pub struct World<'a, 'm, 'ms, 'd, 'ds, 'f, 'p> {
    pub members: &'a mut Members<'m, 'ms>,
    pub monsters: &'a mut MonsterDamage<'d, 'ds>,
    pub formation: &'a mut ResMut<'f, Formation>,
    pub party_damage: &'a mut MessageWriter<'p, DamageApplied>,
    pub items: &'a ItemCatalog,
    pub species: &'a MonsterCatalog,
    /// What monster loot tables scale by.
    pub party_level: u32,
}

/// Resolve a spell on the victims captured when it was cast (retail
/// `ProcessSpellEffectTargets`), in their captured order. Victims that died
/// or left since are skipped; nobody new is added.
pub fn resolve(
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut World,
) -> Vec<Effect> {
    victims.iter().filter_map(|&victim| resolve_one(spell, strength, victim, rng, world)).collect()
}

fn resolve_one(
    spell: &SpellDefinition,
    strength: Strength,
    victim: Victim,
    rng: &mut GameRng,
    world: &mut World,
) -> Option<Effect> {
    let realm = spell.realm?;
    match spell.id {
        SpellId::HEAL_WOUNDS => {
            // Retail `HealTargets`: no resistance.
            let Victim::Member(member) = victim else { return None };
            let (character, mut vitals, _) = world.members.get_mut(member).ok()?;
            let missing = character.max_hp().saturating_sub(vitals.hp);
            if vitals.is_dead() || missing == 0 {
                return Some(Effect::Nothing { victim });
            }
            let amount = magnitude(rng, spell, strength).min(missing);
            vitals.heal(amount, character.max_hp());
            Some(Effect::Healed { victim, amount })
        }
        SpellId::ENERGY_BLAST => {
            let rolled = magnitude(rng, spell, strength);
            match victim {
                Victim::Monster(monster) => {
                    let (target, ..) = world.monsters.monsters.get(monster).ok()?;
                    if target.is_dead() {
                        return None;
                    }
                    let kind = world.species.0.get(target.species as usize)?;
                    let amount = resist(
                        rng,
                        rolled,
                        i32::from(kind.resistances[realm]),
                        u32::from(kind.level),
                        strength.against,
                    );
                    let outcome =
                        world.monsters.apply(monster, amount, world.party_level, rng).ok()?;
                    Some(Effect::Damaged { victim, amount: outcome.damage, outcome })
                }
                Victim::Member(member) => {
                    let (character, mut vitals, carried) = world.members.get_mut(member).ok()?;
                    if vitals.is_dead() {
                        return None;
                    }
                    let resistance = resistances(&character, &carried, world.items)[realm].total;
                    let amount = resist(
                        rng,
                        rolled,
                        resistance,
                        u32::from(character.level),
                        strength.against,
                    );
                    let lost = crate::game::party::damage_member(
                        member,
                        &mut vitals,
                        world.formation,
                        amount,
                        world.party_damage,
                    );
                    if !vitals.is_dead() {
                        let senses = character.attribute(Attribute::Senses);
                        wake_on_hit(&mut vitals.conditions, senses, rng);
                    }
                    let outcome =
                        DamageOutcome { damage: lost, died: vitals.is_dead(), experience: 0 };
                    Some(Effect::Damaged { victim, amount: lost, outcome })
                }
            }
        }
        SpellId::SLEEP => {
            let condition = Condition::Asleep;
            let (level, resistance) = match victim {
                Victim::Monster(monster) => {
                    let (target, ..) = world.monsters.monsters.get(monster).ok()?;
                    if target.is_dead() {
                        return None;
                    }
                    let kind = world.species.0.get(target.species as usize)?;
                    (u32::from(kind.level), i32::from(kind.resistances[realm]))
                }
                Victim::Member(member) => {
                    let (character, vitals, carried) = world.members.get(member).ok()?;
                    if vitals.is_dead() {
                        return None;
                    }
                    (
                        u32::from(character.level),
                        resistances(character, carried, world.items)[realm].total,
                    )
                }
            };
            let rounds = duration(rng, spell, strength);
            if saves(rng, condition, resistance, level, strength.against) {
                return Some(Effect::Resisted { victim });
            }
            let rounds = match resist(rng, rounds, resistance, level, strength.against) {
                0 => return Some(Effect::Resisted { victim }),
                1 => 2,
                rounds => rounds,
            };
            let lasting = Lasting::Rounds(rounds.min(u32::from(u16::MAX)) as u16);
            match victim {
                Victim::Monster(monster) => {
                    let (mut target, ..) = world.monsters.monsters.get_mut(monster).ok()?;
                    inflict(&mut target.conditions, condition, lasting);
                }
                Victim::Member(member) => {
                    let (_, mut vitals, _) = world.members.get_mut(member).ok()?;
                    vitals.inflict(condition, lasting);
                }
            }
            Some(Effect::Inflicted { victim, condition })
        }
        _ => None,
    }
}

/// The spells whose effects are modelled; others are refused before
/// anything is spent.
pub fn modelled(spell: SpellId) -> bool {
    matches!(spell, SpellId::HEAL_WOUNDS | SpellId::ENERGY_BLAST | SpellId::SLEEP)
}

/// Retail fatigue for a learned cast: the spell's level plus its power.
pub fn cast_fatigue(spell: &SpellDefinition, power: u8) -> u32 {
    u32::from(spell.level) + u32::from(power)
}

pub struct MagicPlugin;

impl Plugin for MagicPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MagicRequest>().add_message::<casting::MagicNews>().add_systems(
            Update,
            (
                fill_spell_points.in_set(crate::game::GameplaySet::Equipment),
                accept_magic
                    .after(crate::game::GameplaySet::LevelActivation)
                    .before(crate::game::GameplaySet::Combat)
                    .run_if(crate::game::world::level::level_active),
            ),
        );
    }
}

/// A member asks to cast or use an item. In a fight it becomes their order
/// for the next round; otherwise it happens at once.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MagicRequest {
    pub caster: Entity,
    pub choice: casting::MagicChoice,
}

/// Accept magic requests (retail `ChooseAction`): in combat only record the
/// choice; outside it, execute it now.
#[allow(clippy::too_many_arguments)]
pub fn accept_magic(
    mut requests: MessageReader<MagicRequest>,
    battle: Option<Res<crate::game::combat::Battle>>,
    roster: Res<crate::game::party::roster::PartyRoster>,
    mut orders: Query<&mut crate::game::party::Order>,
    (mut members, mut monsters): (Members, MonsterDamage),
    (mut formation, mut party_damage): (ResMut<Formation>, MessageWriter<DamageApplied>),
    data: Option<Res<crate::game::data::GameData>>,
    (items, species, spells): (
        Res<Assets<ItemCatalog>>,
        Res<Assets<MonsterCatalog>>,
        Res<Assets<SpellCatalog>>,
    ),
    mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<casting::MagicNews>,
) {
    for &MagicRequest { caster, choice } in requests.read() {
        if !roster.0.contains(&caster) {
            continue;
        }
        if battle.is_some() {
            if let Ok(mut order) = orders.get_mut(caster) {
                order.set_if_neq(crate::game::party::Order::Magic(choice));
            }
            continue;
        }
        let catalogs = data.as_ref().and_then(|d| {
            Some((items.get(&d.items)?, species.get(&d.species)?, spells.get(&d.spells)?))
        });
        let Some((items, species, spells)) = catalogs else {
            news.write(casting::MagicNews::Refused {
                caster,
                refusal: casting::Refusal::MissingContent,
            });
            continue;
        };
        let living: Vec<Entity> = roster
            .0
            .iter()
            .copied()
            .filter(|&m| members.get(m).is_ok_and(|(_, v, _)| !v.is_dead()))
            .collect();
        let levels: u32 =
            members.iter_many(&roster.0).matched().map(|(c, _, _)| u32::from(c.level)).sum();
        // Outside a fight no monster is a target: the hostile spells
        // modelled are combat spells.
        let situation = casting::Situation {
            in_combat: false,
            party: &living,
            reachable: &|_, _| false,
            group_of: &|m| vec![m],
        };
        let mut world = World {
            members: &mut members,
            monsters: &mut monsters,
            formation: &mut formation,
            party_damage: &mut party_damage,
            items,
            species,
            party_level: levels / roster.0.len().max(1) as u32,
        };
        news.write(
            match casting::perform(
                caster,
                choice,
                &situation,
                spells,
                &mut points,
                &mut rng,
                &mut world,
            ) {
                Ok(report) => casting::MagicNews::Done(report),
                Err(refusal) => casting::MagicNews::Refused { caster, refusal },
            },
        );
    }
}

/// Give new members whose spells are known their full spell points once
/// the spell catalog has loaded.
pub fn fill_spell_points(
    mut commands: Commands,
    data: Option<Res<crate::game::data::GameData>>,
    spells: Res<Assets<SpellCatalog>>,
    members: Query<(Entity, &Character, &LearnedSpells), Without<SpellPoints>>,
) {
    let Some(catalog) = data.and_then(|d| spells.get(&d.spells)) else { return };
    for (entity, character, learned) in &members {
        commands.entity(entity).insert(SpellPoints::full(max_points(character, learned, catalog)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_priest_picks_two_first_level_divine_spells_and_gains_their_points() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let fighter = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let heal = heal_wounds();
        assert_eq!((starting_picks(&priest), starting_picks(&fighter)), (2, 0));
        assert!(can_learn(&priest, &heal) && !can_learn(&fighter, &heal));
        let higher = SpellDefinition { level: 2, ..heal_wounds() };
        assert!(!can_learn(&priest, &higher), "level 1 casters learn level 1 spells only");
        let catalog =
            SpellCatalog((0..=6).map(|i| if i == 6 { heal_wounds() } else { default() }).collect());
        let learned = LearnedSpells([SpellId::HEAL_WOUNDS].into());
        let points = max_points(&priest, &learned, &catalog);
        assert!(points[Realm::Divine] >= 3, "never under the dearest learned spell");
        assert_eq!(points[Realm::Fire], 0, "no spell, no points");
    }

    #[test]
    fn points_recover_by_retail_rate_and_never_beyond_the_maximum() {
        let mut maximum = EnumMap::default();
        maximum[Realm::Divine] = 20;
        let mut points = SpellPoints::default();
        // (0.65·20 + 20)/240 = 0.1375 a tick at half rate: 0.06875.
        points.recover(&maximum, 100, recovery_scale(0.0));
        assert_eq!(points.current[Realm::Divine], 6);
        points.recover(&maximum, 10_000, 1.0);
        assert_eq!(points.current[Realm::Divine], 20);
        assert_eq!(points.fraction[Realm::Divine], 0.0);
    }

    #[test]
    fn affordable_power_is_points_over_cost_within_seven() {
        let mut points = SpellPoints::default();
        points.current[Realm::Divine] = 10;
        assert_eq!(points.affordable_power(&heal_wounds()), 3);
        points.current[Realm::Divine] = 100;
        assert_eq!(points.affordable_power(&heal_wounds()), 7);
    }

    #[test]
    fn healing_cannot_backfire_and_an_untrained_caster_fails_more_at_higher_power() {
        let priest = Character::new("Cleric", Race::Human, Gender::Male, Profession::Priest);
        let heal = heal_wounds();
        assert!(fail_chance(&priest, &heal, 7) > fail_chance(&priest, &heal, 1));
        let mut rng = GameRng::seeded(1);
        for _ in 0..200 {
            assert_ne!(roll(&mut rng, 100, &heal), Outcome::Backfired);
        }
    }
}
