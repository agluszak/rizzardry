//! Casting a learned spell or using an item's spell: one operation that
//! revalidates the source and target when it runs, captures the victims,
//! rolls the cast, pays for it and resolves its effect together (retail
//! `ExecuteCharacterSpellCast`, `UseItem`, `CastItemSpell`,
//! `CastSpellFromSource`).
//!
//! Choosing never spends anything. A refused attempt spends nothing; an
//! accepted one pays even when it fizzles or backfires: spell points for a
//! learned spell, one use for an item. The effect keeps its own copy of the
//! spell and victims, so using the last potion still heals, and a missile in
//! flight still lands after its caster falls.

use bevy::prelude::*;

use super::effects::{self, Effect, Field, Victim};
use super::{LearnedSpells, Outcome, SpellPoints, Strength};
use crate::content::characters::Skill;
use crate::content::items::{ItemCatalog, ItemDefinition, UseType};
use crate::content::spells::{
    MAX_POWER, PowerChoice, SpellCatalog, SpellDefinition, SpellId, Targeting, Usable,
    min_caster_level,
};
use crate::game::characters::{Character, Condition, Lasting, Profession, Vitals};
use crate::game::difficulty::Difficulty;
use crate::game::inventory::{Carried, ItemOwner, PartyInventory, StackId};
use crate::game::rng::GameRng;

/// The power that asks the caster to choose it: as much as the victim needs
/// (retail "Auto Calibrate"), or, outside a fight, as much and as often as
/// it takes (retail "Auto Recast").
pub const AUTO_POWER: u8 = MAX_POWER + 1;

/// How fast spell missiles fly: retail's `energy_blast.mls` velocity of 30,
/// advanced per two-millisecond step (`Missile.cpp`), comes to 15 m/s.
const MISSILE_SPEED: f32 = 15.0;

/// What pays for a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A learned spell at the chosen power, or [`AUTO_POWER`]; it is
    /// clamped to what the caster can afford when it runs.
    Spell { spell: SpellId, power: u8 },
    /// A stack the actor carries, identified, not reserved.
    Item(StackId),
    /// A shared-pool stack; payment never silently changes owners.
    PoolItem(StackId),
}

/// Whom the actor aimed at; group, area, party and self spells widen or
/// replace it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    Member(Entity),
    Monster(Entity),
    Device(Entity),
}

/// An accepted magic action: the source and the aim, nothing resolved yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MagicChoice {
    pub source: Source,
    pub aim: Aim,
}

/// Why nothing was spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    CannotAct,
    Silenced,
    NotLearned,
    NoSpellPoints,
    NoItem,
    NoCharges,
    CannotUse,
    /// Its effect is not implemented yet.
    Unsupported,
    /// Not now: only in combat, or only out of it.
    WrongTime,
    NoTarget,
    /// An instrument or gadget that could not be brought to play: its
    /// fatigue is paid, the item is not (retail string 0x1f3).
    PlayedBadly,
    MissingContent,
}

/// What became of a magic action, for whoever tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MagicNews {
    Done(Report),
    /// An indirect trap source has no character caster or payment.
    Triggered {
        spell: SpellId,
        effects: Vec<Effect>,
        improved: Vec<(Entity, Skill)>,
    },
    /// A missile reached its victims.
    Landed {
        caster: Entity,
        spell: SpellId,
        effects: Vec<Effect>,
    },
    Refused {
        caster: Entity,
        refusal: Refusal,
    },
}

/// What a cast did.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub caster: Entity,
    pub spell: SpellId,
    pub power: u8,
    /// The item it came from, already spent.
    pub item: Option<String>,
    pub outcome: Outcome,
    /// What landed at once; missiles land later as [`MagicNews::Landed`].
    pub effects: Vec<Effect>,
    /// Skills practice raised, by whom.
    pub improved: Vec<(Entity, Skill)>,
    /// Casts repeated after the first: an auto cast trying again.
    pub repeats: u32,
}

/// A missile on its way: its own copy of the spell's strength and victims.
#[derive(Component, Debug, Clone)]
pub struct SpellInFlight {
    pub caster: Entity,
    pub spell: SpellId,
    pub strength: Strength,
    pub victims: Vec<Victim>,
    pub arrives: Timer,
}

/// A monster the actor may aim at: alive, in the fight if there is one, in
/// sight, with where it stands and how far it is from the party.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub entity: Entity,
    pub at: Vec3,
    pub distance: f32,
}

/// Mutually exclusive execution contexts, shared by menus and commitment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseContext {
    World,
    Combat,
    Camp,
    Lock,
    Trap,
}

pub fn usable_now(spell: &SpellDefinition, context: UseContext) -> bool {
    match context {
        UseContext::Lock => spell.id == SpellId(39),
        UseContext::Trap => matches!(spell.id.0, 18 | 39),
        _ => match spell.usable {
            Usable::Always => true,
            Usable::InCombat => context == UseContext::Combat,
            Usable::OutOfCombat => context != UseContext::Combat,
            Usable::Camped => context == UseContext::Camp,
            Usable::OnLockOrTrap => false,
        },
    }
}

/// Where the actor is and what it can reach, as the caller sees it.
pub struct Situation<'a> {
    pub context: UseContext,
    /// The party, in roster order, the dead included.
    pub party: &'a [Entity],
    pub party_at: Vec3,
    pub monsters: &'a [Candidate],
    pub group_of: &'a dyn Fn(Entity) -> Vec<Entity>,
}

impl Situation<'_> {
    fn in_combat(&self) -> bool {
        self.context == UseContext::Combat
    }
    fn candidate(&self, monster: Entity, spell: &SpellDefinition) -> Option<&Candidate> {
        self.monsters.iter().find(|c| c.entity == monster && c.distance <= spell.reach.metres())
    }

    fn party_victims(&self) -> Vec<Victim> {
        self.party.iter().map(|&m| Victim::Member(m)).collect()
    }
}

/// The victims a spell takes when it is cast (retail
/// `PopulateSpellTargetMarkers`), and where its missile flies. Later movement
/// adds nobody.
fn capture(
    caster: Entity,
    spell: &SpellDefinition,
    power: u8,
    aim: Aim,
    situation: &Situation,
    forced_self: bool,
) -> Result<(Vec<Victim>, Option<Vec3>), Refusal> {
    if forced_self {
        return Ok((vec![Victim::Member(caster)], None));
    }
    let aimed = || match aim {
        Aim::Monster(m) => situation.candidate(m, spell).copied().ok_or(Refusal::NoTarget),
        Aim::Member(_) | Aim::Device(_) => Err(Refusal::NoTarget),
    };
    let reach = spell.reach.metres();
    Ok(match spell.targeting {
        Targeting::Caster => (vec![Victim::Member(caster)], None),
        Targeting::Ally => match aim {
            Aim::Member(member) if situation.party.contains(&member) => {
                (vec![Victim::Member(member)], None)
            }
            _ => return Err(Refusal::NoTarget),
        },
        Targeting::Party => (situation.party_victims(), None),
        Targeting::Enemy => {
            let target = aimed()?;
            (vec![Victim::Monster(target.entity)], Some(target.at))
        }
        Targeting::EnemyGroup => {
            let first = aimed()?;
            let mut victims = vec![Victim::Monster(first.entity)];
            for mate in (situation.group_of)(first.entity) {
                if mate != first.entity && situation.candidate(mate, spell).is_some() {
                    victims.push(Victim::Monster(mate));
                }
            }
            (victims, Some(first.at))
        }
        Targeting::AllEnemies => {
            let victims: Vec<_> = situation
                .monsters
                .iter()
                .filter(|c| c.distance <= reach)
                .map(|c| Victim::Monster(c.entity))
                .collect();
            if victims.is_empty() {
                return Err(Refusal::NoTarget);
            }
            (victims, None)
        }
        // A cone fans out from the party toward the aimed monster.
        Targeting::Cone => {
            let target = aimed()?;
            let toward = (target.at - situation.party_at).with_y(0.0);
            let victims = situation
                .monsters
                .iter()
                .filter(|c| {
                    c.distance <= reach
                        && (c.at - situation.party_at).with_y(0.0).angle_between(toward)
                            < std::f32::consts::FRAC_PI_4
                })
                .map(|c| Victim::Monster(c.entity))
                .collect();
            (victims, Some(target.at))
        }
        // A blast centred where it lands, the party too if it is that close.
        Targeting::Radius => {
            let target = aimed()?;
            let radius = spell.radius + spell.radius_per_power * f32::from(power);
            let mut victims: Vec<_> = situation
                .monsters
                .iter()
                .filter(|c| c.at.distance(target.at) <= radius)
                .map(|c| Victim::Monster(c.entity))
                .collect();
            if situation.party_at.distance(target.at) <= radius {
                victims.extend(situation.party_victims());
            }
            (victims, Some(target.at))
        }
        Targeting::Point | Targeting::Item | Targeting::LockOrTrap => {
            return Err(Refusal::Unsupported);
        }
    })
}

/// Whom a backfire turns on (retail `PrepareSpellTarget`): a spell aimed at
/// one victim turns on its caster; a group, cone, blast or all-enemies spell
/// on the whole party.
fn backfire_victims(caster: Entity, spell: &SpellDefinition, situation: &Situation) -> Vec<Victim> {
    match spell.targeting {
        Targeting::Caster | Targeting::Ally | Targeting::Enemy => vec![Victim::Member(caster)],
        _ => situation.party_victims(),
    }
}

/// Potions and food: usable by anyone who holds them.
fn consumed_by_anyone(definition: &ItemDefinition) -> bool {
    matches!(definition.use_type, UseType::Potion | UseType::Food)
}

/// How likely using an item's spell fails, and whether it is a quiet
/// (thrown) use, every failure of which can backfire (retail
/// `GetItemUseDifficulty`, `CastItemSpell`, `CastSpellFromSource`).
fn item_failure(
    character: &Character,
    definition: &ItemDefinition,
    spell: &SpellDefinition,
    power: u8,
    identified: bool,
    difficulty: Difficulty,
) -> (u32, bool) {
    let Some(skill) = definition.use_type.skill() else { return (0, false) };
    let level = character.skill(skill);
    let mut failure = super::spell_failure(level, spell, power);
    match definition.use_type {
        UseType::Thrown => failure = failure * 8 / 10,
        UseType::Charged | UseType::Scroll => failure = failure * 9 / 10,
        UseType::Instrument | UseType::Gadget => {
            let shortfall = u32::from(spell.level) * 15;
            if level < shortfall {
                failure += (shortfall - level) / 3;
            }
            let profession = if definition.use_type == UseType::Instrument {
                Profession::Bard
            } else {
                Profession::Gadgeteer
            };
            let adjust = min_caster_level(spell.level)
                - i32::from(character.profession_levels[profession])
                - 1
                + i32::from(power);
            if adjust > 0 {
                failure += u32::from(spell.level) * adjust as u32;
            }
        }
        _ => {}
    }
    // Choices are made between rounds, so none is late in one.
    let mut failure = difficulty.scale_by_pace(failure, 0);
    if !identified {
        failure += 30;
    }
    let quiet = definition.use_type == UseType::Thrown;
    if quiet {
        failure /= 2;
    }
    (failure.min(100), quiet)
}

/// The strength an item's spell is cast at: by the user's level, or their
/// Bard or Gadgeteer levels for instruments and gadgets (retail
/// `CastItemSpell`).
fn item_strength(
    character: &Character,
    definition: &ItemDefinition,
    spell: &SpellDefinition,
    power: u8,
) -> Strength {
    let figure = match definition.use_type {
        UseType::Instrument => u32::from(character.profession_levels[Profession::Bard]),
        UseType::Gadget => u32::from(character.profession_levels[Profession::Gadgeteer]),
        _ => u32::from(character.level),
    };
    Strength::at(figure, spell, power, 0)
}

/// The power an auto cast takes (retail `ChooseSpellPowerLevelForTarget`):
/// for healing, the lowest whose average covers what the victim lacks,
/// stopping short of a failure chance over 10%; for cures, the cheapest to
/// outlast the condition; otherwise 1.
fn auto_power(
    character: &Character,
    spell: &SpellDefinition,
    victim: Option<(&Character, &Vitals)>,
    affordable: u8,
) -> u8 {
    let fail = |p| super::fail_chance(character, spell, p);
    let Some((profile, vitals)) = victim else { return 1 };
    let power = if effects::restores(spell.id) {
        let missing = if spell.id == SpellId(13) {
            profile.max_stamina().saturating_sub(vitals.stamina)
        } else {
            profile.max_hp().saturating_sub(vitals.hp)
        } as f32;
        let average = |p: u8| {
            spell.dice.map_or(0.0, |d| {
                f32::from(d.count) * f32::from(p) * (f32::from(d.sides) + 1.0) / 2.0
                    + f32::from(d.base)
            })
        };
        if missing == 0.0 {
            1
        } else {
            (1..=MAX_POWER)
                .find_map(|p| {
                    if fail(p) > 10 {
                        Some(p.saturating_sub(1).max(1))
                    } else {
                        (missing < average(p)).then_some(p)
                    }
                })
                .unwrap_or(MAX_POWER)
        }
    } else {
        let turns = vitals
            .conditions
            .iter()
            .filter_map(|&(_, lasting)| match lasting {
                Lasting::Rounds(t) => Some(u32::from(t)),
                Lasting::Indefinite => None,
            })
            .max();
        match turns {
            None => 1,
            Some(turns) => (1..=affordable.max(1))
                .map(|p| {
                    let square = u32::from(p).pow(2);
                    let per = (square - fail(p) * square / 100).max(1);
                    (spell.cost * turns.div_ceil(per) * u32::from(p), p)
                })
                .min()
                .map_or(1, |(_, p)| p),
        }
    };
    power.min(affordable)
}

/// An item about to be used.
struct ItemUse<'d> {
    id: StackId,
    definition: &'d ItemDefinition,
    identified: bool,
    owner: ItemOwner,
}

struct Prepared<'a> {
    character: Character,
    spell: &'a SpellDefinition,
    power: u8,
    item: Option<ItemUse<'a>>,
    forced_self: bool,
}

/// Shared menu/commit activation rule; alternate hands are not worn.
pub fn can_activate(character: &Character, item: &ItemDefinition, worn: bool) -> bool {
    !matches!(item.use_type, UseType::None | UseType::Spellbook)
        && (!item.requires_worn() || worn)
        && (character.can_use(item) || consumed_by_anyone(item))
}

fn prepare_source<'a>(
    caster: Entity,
    source: Source,
    in_combat: bool,
    (spells, items): (&'a SpellCatalog, &'a ItemCatalog),
    members: &super::Members,
    points: &Query<(&LearnedSpells, &mut SpellPoints)>,
    inventory: &PartyInventory,
) -> Result<Prepared<'a>, Refusal> {
    let (profile, vitals, carried) = members.get(caster).map_err(|_| Refusal::CannotAct)?;
    if !vitals.can_fight() {
        return Err(Refusal::CannotAct);
    }
    let character = profile.clone();
    // What the source is, before anything is spent.
    let (spell, power, item, forced_self) = match source {
        Source::Spell { spell, power } => {
            if power == 0 {
                return Err(Refusal::CannotUse);
            }
            if vitals.has(Condition::Silenced) {
                return Err(Refusal::Silenced);
            }
            let (learned, pool) = points.get(caster).map_err(|_| Refusal::NotLearned)?;
            if !learned.0.contains(&spell) {
                return Err(Refusal::NotLearned);
            }
            let definition = spells.get(spell).ok_or(Refusal::MissingContent)?;
            if pool.affordable_power(definition) == 0 {
                return Err(Refusal::NoSpellPoints);
            }
            (definition, power.min(AUTO_POWER), None, false)
        }
        Source::Item(id) | Source::PoolItem(id) => {
            let owner = if matches!(source, Source::PoolItem(_)) {
                ItemOwner::Pool
            } else {
                ItemOwner::Member(caster)
            };
            let stack = match owner {
                ItemOwner::Pool => inventory.pool.stacks.iter().find(|s| s.id == id),
                ItemOwner::Member(_) => carried.find_stack(id),
            }
            .ok_or(Refusal::NoItem)?;
            let definition = items.get(stack.item).ok_or(Refusal::MissingContent)?;
            let cast = definition.spell.ok_or(Refusal::CannotUse)?;
            if cast.power == 0
                && !matches!(definition.use_type, UseType::Instrument | UseType::Gadget)
            {
                return Err(Refusal::CannotUse);
            }
            let worn = owner == ItemOwner::Member(caster)
                && carried.active_equipment().any(|(_, e)| e.stack.id == id);
            if !can_activate(&character, definition, worn) {
                return Err(Refusal::CannotUse);
            }
            if stack.remaining_uses() == Some(0) {
                return Err(Refusal::NoCharges);
            }
            if definition.use_type == UseType::Instrument && vitals.has(Condition::Silenced) {
                return Err(Refusal::Silenced);
            }
            let spell = spells.get(cast.spell).ok_or(Refusal::MissingContent)?;
            // Drinking in combat is always done by the drinker (retail
            // `UseItem` aims potions at their user).
            let forced = in_combat && consumed_by_anyone(definition);
            (
                spell,
                cast.power,
                Some(ItemUse { id, definition, identified: stack.identified, owner }),
                forced,
            )
        }
    };
    Ok(Prepared { character, spell, power, item, forced_self })
}

/// Cast or use as `caster`, now: revalidate, capture victims, roll, pay and
/// resolve. Nothing is spent on refusal. Missiles are returned to be flown.
#[allow(clippy::too_many_arguments)]
pub fn perform(
    caster: Entity,
    choice: MagicChoice,
    situation: &Situation,
    spells: &SpellCatalog,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    rng: &mut GameRng,
    world: &mut Field,
) -> Result<(Report, Vec<SpellInFlight>), Refusal> {
    let Prepared { character, spell, power, item, forced_self } = prepare_source(
        caster,
        choice.source,
        situation.in_combat(),
        (spells, world.items),
        world.members,
        points,
        &world.monsters.inventory,
    )?;
    if !effects::modelled(spell.id) {
        return Err(Refusal::Unsupported);
    }
    if !usable_now(spell, situation.context) {
        return Err(Refusal::WrongTime);
    }
    let auto = item.is_none() && power == AUTO_POWER;
    // Auto recast belongs outside a fight (retail refuses it in one).
    let repeat = auto && spell.power == PowerChoice::ChosenOrRepeat;
    if repeat && situation.in_combat() {
        return Err(Refusal::WrongTime);
    }
    // Instruments can pay fatigue while finding power, so validate the target
    // first. Radius changes may widen victims but cannot rescue an invalid aim.
    capture(caster, spell, power.min(MAX_POWER), choice.aim, situation, forced_self)?;
    let power = match &item {
        Some(use_) if matches!(use_.definition.use_type, UseType::Instrument | UseType::Gadget) => {
            play(caster, &character, use_, spell, rng, world)?
        }
        _ => power,
    };
    let mut report = Report {
        caster,
        spell: spell.id,
        power: 0,
        item: None,
        outcome: Outcome::Fizzled,
        effects: Vec::new(),
        improved: Vec::new(),
        repeats: 0,
    };
    let mut flights = Vec::new();
    loop {
        let first = report.power == 0;
        // This attempt's power, against what the caster has now.
        let cast_power = match &item {
            Some(_) => power,
            None => {
                let (_, pool) = points.get(caster).map_err(|_| Refusal::NotLearned)?;
                let affordable = pool.affordable_power(spell);
                if affordable == 0 {
                    break;
                }
                if auto {
                    let victim = match choice.aim {
                        Aim::Member(m) => world.members.get(m).ok(),
                        Aim::Monster(_) | Aim::Device(_) => None,
                    };
                    auto_power(&character, spell, victim.map(|(c, v, _)| (c, v)), affordable)
                } else {
                    power.min(affordable)
                }
            }
        };
        let (victims, landing) =
            capture(caster, spell, cast_power, choice.aim, situation, forced_self)?;
        // Accepted: roll, then pay whatever the roll.
        let (chance, quiet) = match &item {
            Some(use_) => item_failure(
                &character,
                use_.definition,
                spell,
                cast_power,
                use_.identified,
                world.difficulty,
            ),
            None => {
                let chance = super::fail_chance(&character, spell, cast_power);
                (world.difficulty.scale_by_pace(chance, 0), false)
            }
        };
        let strength = match &item {
            Some(use_) => item_strength(&character, use_.definition, spell, cast_power),
            None => Strength::of(&character, spell, cast_power),
        };
        let outcome = super::roll(rng, chance, spell, quiet);
        pay(caster, spell, cast_power, item.as_ref(), points, rng, world, &mut report)?;
        let victims = match outcome {
            Outcome::Cast => victims,
            Outcome::Backfired => backfire_victims(caster, spell, situation),
            Outcome::Fizzled => Vec::new(),
        };
        // Missiles fly to their victims, one each, or to where they burst; a
        // backfire turns on the party at once.
        let flies = spell.missile.is_some() && outcome == Outcome::Cast && !victims.is_empty();
        let landed = if flies {
            let arrives = |to: Vec3| {
                let seconds = to.distance(situation.party_at) / MISSILE_SPEED;
                Timer::from_seconds(seconds, TimerMode::Once)
            };
            if spell.targeting == Targeting::Radius {
                let to = landing.unwrap_or(situation.party_at);
                flights.push(SpellInFlight {
                    caster,
                    spell: spell.id,
                    strength,
                    victims: victims.clone(),
                    arrives: arrives(to),
                });
            } else {
                for &victim in &victims {
                    let to = match victim {
                        Victim::Monster(m) => situation
                            .monsters
                            .iter()
                            .find(|c| c.entity == m)
                            .map_or(situation.party_at, |c| c.at),
                        Victim::Member(_) => situation.party_at,
                    };
                    let victims = vec![victim];
                    flights.push(SpellInFlight {
                        caster,
                        spell: spell.id,
                        strength,
                        victims,
                        arrives: arrives(to),
                    });
                }
            }
            Vec::new()
        } else {
            effects::resolve(spell, strength, &victims, rng, world, (spells, &mut *points))
        };
        // Practice follows what the cast did; a missile on its way counts.
        if outcome != Outcome::Fizzled && (flies || effects::affected(&landed)) {
            practise(caster, &character, spell, cast_power, item.as_ref(), rng, world);
        }
        if first {
            report.power = cast_power;
            report.outcome = outcome;
        } else {
            report.repeats += 1;
            if outcome != Outcome::Fizzled {
                report.outcome = outcome;
            }
        }
        report.effects.extend(landed);
        // An auto cast tries again while it fizzles, and auto recast goes on
        // until its victim is whole, while stamina and points last.
        let stamina = world.members.get(caster).map_or(0, |(_, v, _)| v.stamina);
        let left = points.get(caster).map_or(0, |(_, p)| spell.realm.map_or(0, |r| p.current[r]));
        let again = auto
            && stamina > u32::from(spell.level)
            && left >= spell.cost
            && match outcome {
                Outcome::Fizzled => true,
                _ => repeat && !effects::all_restored(spell.id, &victims, world.members),
            };
        if !again || report.repeats >= 50 {
            break;
        }
    }
    if report.power == 0 {
        return Err(Refusal::NoSpellPoints);
    }
    report.improved = std::mem::take(&mut world.improved);
    Ok((report, flights))
}

/// Execute the shared source side of a device operation while the exclusive
/// caller owns target validation and applies the device consequence immediately.
pub(crate) fn perform_device(
    caster: Entity,
    source: Source,
    context: UseContext,
    spells: &SpellCatalog,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    rng: &mut GameRng,
    world: &mut Field,
) -> Result<(Report, Strength), Refusal> {
    let Prepared { character, spell, power, item, .. } = prepare_source(
        caster,
        source,
        false,
        (spells, world.items),
        world.members,
        points,
        &world.monsters.inventory,
    )?;
    if !matches!(context, UseContext::Lock | UseContext::Trap) || !usable_now(spell, context) {
        return Err(Refusal::WrongTime);
    }
    if spell.realm.is_none() {
        return Err(Refusal::MissingContent);
    }
    let power = match &item {
        Some(use_) if matches!(use_.definition.use_type, UseType::Instrument | UseType::Gadget) => {
            play(caster, &character, use_, spell, rng, world)?
        }
        Some(_) => power,
        None => power
            .min(MAX_POWER)
            .min(points.get(caster).map_err(|_| Refusal::NotLearned)?.1.affordable_power(spell)),
    };
    if power == 0 {
        return Err(Refusal::NoSpellPoints);
    }
    let (chance, quiet, strength) = match &item {
        Some(use_) => {
            let (chance, quiet) = item_failure(
                &character,
                use_.definition,
                spell,
                power,
                use_.identified,
                world.difficulty,
            );
            (chance, quiet, item_strength(&character, use_.definition, spell, power))
        }
        None => (
            world.difficulty.scale_by_pace(super::fail_chance(&character, spell, power), 0),
            false,
            Strength::of(&character, spell, power),
        ),
    };
    let outcome = super::roll(rng, chance, spell, quiet);
    let mut report = Report {
        caster,
        spell: spell.id,
        power,
        item: None,
        outcome,
        effects: Vec::new(),
        improved: Vec::new(),
        repeats: 0,
    };
    pay(caster, spell, power, item.as_ref(), points, rng, world, &mut report)?;
    // Retail considers these utility spells affected even on a fizzle.
    practise(caster, &character, spell, power, item.as_ref(), rng, world);
    report.improved = std::mem::take(&mut world.improved);
    Ok((report, strength))
}

/// Instruments and gadgets find the power their player can manage, paying
/// their fatigue whether or not they can (retail `UseItem`).
fn play(
    caster: Entity,
    character: &Character,
    use_: &ItemUse,
    spell: &SpellDefinition,
    rng: &mut GameRng,
    world: &mut Field,
) -> Result<u8, Refusal> {
    let mut power = MAX_POWER;
    while power > 0
        && item_failure(character, use_.definition, spell, power, true, Difficulty::Normal).0 > 15
    {
        power -= 1;
    }
    let mut chance = u32::from(power) * 7;
    while power > 0 && chance > rng.below(100) {
        power -= 1;
        chance = chance.saturating_sub(7);
    }
    let (mut profile, mut vitals, _) =
        world.members.get_mut(caster).map_err(|_| Refusal::CannotAct)?;
    vitals.spend_stamina(u32::from(spell.level) * 8 + 20);
    if power == 0 {
        if let Some(skill) = use_.definition.use_type.skill()
            && profile.practise(skill, 1, rng)
        {
            world.improved.push((caster, skill));
        }
        return Err(Refusal::PlayedBadly);
    }
    Ok(power)
}

/// Pay for one accepted attempt: points or a use, then fatigue.
#[allow(clippy::too_many_arguments)]
fn pay(
    caster: Entity,
    spell: &SpellDefinition,
    power: u8,
    item: Option<&ItemUse>,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    rng: &mut GameRng,
    world: &mut Field,
    report: &mut Report,
) -> Result<(), Refusal> {
    let fatigue = match item {
        None => {
            let (_, mut pool) = points.get_mut(caster).map_err(|_| Refusal::NotLearned)?;
            let realm = spell.realm.ok_or(Refusal::MissingContent)?;
            pool.current[realm] = pool.current[realm].saturating_sub(spell.cost_at(power));
            super::cast_fatigue(spell, power)
        }
        Some(use_) => {
            let consumed = match use_.owner {
                ItemOwner::Pool => {
                    world.monsters.inventory.pool.consume_use(use_.id, use_.definition)
                }
                ItemOwner::Member(owner) => world
                    .members
                    .get_mut(owner)
                    .map_err(|_| Refusal::NoItem)?
                    .2
                    .consume_use(use_.id, use_.definition, use_.definition.keeps_empty()),
            };
            if !consumed {
                return Err(Refusal::NoItem);
            }
            let (_, vitals, _) = world.members.get(caster).map_err(|_| Refusal::CannotAct)?;
            report.item = Some(use_.definition.name.clone());
            match use_.definition.use_type {
                UseType::Thrown => 10,
                // Already paid when the instrument or gadget was played.
                UseType::Instrument | UseType::Gadget => 0,
                _ => {
                    let base = rng.below(2) + 1;
                    if vitals.has(Condition::Diseased) { base * 2 } else { base }
                }
            }
        }
    };
    world.members.get_mut(caster).map_err(|_| Refusal::CannotAct)?.1.spend_stamina(fatigue);
    Ok(())
}

/// Practice from a cast that did something (retail `Magic.cpp:2682-2692`,
/// `PC Item.cpp:3792-3796`): a learned spell trains its book, its realm and
/// Power Cast; an item its use's skill.
fn practise(
    caster: Entity,
    character: &Character,
    spell: &SpellDefinition,
    power: u8,
    item: Option<&ItemUse>,
    rng: &mut GameRng,
    world: &mut Field,
) {
    let practice: Vec<(Skill, u32)> = match item {
        None => {
            let index = u32::from(spell.level) + u32::from(power);
            let mut practice = vec![(super::best_book(character, spell), (index + 2) >> 2)];
            if let Some(realm) = spell.realm {
                practice.push((super::realm_skill(realm), index));
            }
            if character.skill_available(Skill::PowerCast) {
                practice.push((Skill::PowerCast, (index + 2) >> 2));
            }
            practice
        }
        Some(use_) => use_
            .definition
            .use_type
            .skill()
            .map(|skill| vec![(skill, u32::from(spell.level) + u32::from(power) / 2)])
            .unwrap_or_default(),
    };
    if let Ok((mut profile, ..)) = world.members.get_mut(caster) {
        for (skill, usage) in practice {
            if profile.practise(skill, usage, rng) {
                world.improved.push((caster, skill));
            }
        }
    }
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    character: &Character,
    carried: &Carried,
    items: &crate::content::items::ItemCatalog,
) -> Vec<(StackId, SpellId)> {
    carried
        .stacks()
        .filter_map(|stack| {
            let definition = items.get(stack.item)?;
            let cast = definition.spell?;
            let worn =
                carried.active_equipment().any(|(_, equipped)| equipped.stack.id == stack.id);
            let usable = can_activate(character, definition, worn);
            let charged = stack.remaining_uses() != Some(0);
            (usable
                && charged
                && (effects::modelled(cast.spell) || matches!(cast.spell.0, 18 | 39)))
            .then_some((stack.id, cast.spell))
        })
        .collect()
}
