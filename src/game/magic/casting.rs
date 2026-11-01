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

use super::effects::{self, Effect, Field, Landed, Victim};
use super::{Outcome, Strength};
use crate::content::characters::Skill;
use crate::content::items::{ItemCatalog, ItemDefinition, UseType};
use crate::content::spells::{
    MAX_POWER, PowerChoice, SpellDefinition, SpellId, Targeting, Usable, min_caster_level,
};
use crate::game::characters::{Character, Condition, Lasting, Profession, Vitals};
use crate::game::difficulty::Difficulty;
use crate::game::inventory::{Carried, PartyInventory, StackId};

/// The power that asks the caster to choose it: as much as the victim needs
/// (retail "Auto Calibrate"), or, outside a fight, as much and as often as
/// it takes (retail "Auto Recast").
pub const AUTO_POWER: u32 = MAX_POWER + 1;

/// How fast spell missiles fly: retail's `energy_blast.mls` velocity of 30,
/// advanced per two-millisecond step (`Missile.cpp`), comes to 15 m/s.
pub(crate) const MISSILE_SPEED: f32 = 15.0;

/// What pays for a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A learned spell at the chosen power, or [`AUTO_POWER`]; it is
    /// clamped to what the caster can afford when it runs.
    Spell { spell: SpellId, power: u32 },
    /// A stack the actor carries, identified, not reserved.
    Item(StackId),
    /// A shared-pool stack; payment never silently changes owners.
    PoolItem(StackId),
}

impl Source {
    /// Resolve an item source by its stable stack identity. This is routing,
    /// not acceptance: the cast revalidates eligibility and payment.
    pub fn spell_id(
        self,
        carried: &Carried,
        inventory: &PartyInventory,
        items: &ItemCatalog,
    ) -> Option<SpellId> {
        match self {
            Self::Spell { spell, .. } => Some(spell),
            Self::Item(id) => items[carried.find_stack(id)?.item].spell.map(|s| s.spell),
            Self::PoolItem(id) => items[inventory.pool.stacks.iter().find(|s| s.id == id)?.item]
                .spell
                .map(|s| s.spell),
        }
    }
}

/// Whom the actor aimed at; group, area, party and self spells widen or
/// replace it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Point(Vec3),
    Member(Entity),
    Monster(Entity),
    /// Whoever stands nearest the party's heading within the spell's reach
    /// when it is cast (retail's automatic targeting), the nearer of equals.
    Ahead,
    /// The device attempt under way (its lock or trap), by the attempt.
    Device(Entity),
    /// The selected stack, retained across any delayed request.
    Item(StackId),
}

/// An accepted magic action: the source and the aim, nothing resolved yet.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// fatigue is paid, the item is not (retail string 0x1f3). The attempt
    /// is practice, and `improved` is the skill it raised, if any.
    PlayedBadly {
        improved: Option<Skill>,
    },
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
    /// A missile reached its victims, with the skills its impact improved.
    Landed {
        caster: Entity,
        spell: SpellId,
        effects: Vec<Effect>,
        improved: Vec<(Entity, Skill)>,
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
    pub power: u32,
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

pub use super::impacts::{Finalization, Origin};

/// A missile on its way: its own copy of the spell's strength and victims.
#[derive(Component, Debug, Clone)]
pub struct SpellInFlight {
    pub origin: Origin,
    pub finish: Option<Finalization>,
    pub caster: Entity,
    pub spell: SpellId,
    pub strength: Strength,
    pub victims: Vec<Victim>,
    pub arrives: Timer,
    /// Its place in the order flights were launched ([`FlightOrder`]).
    pub launched: u64,
}

/// Launches flights in the order casts are accepted. Impacts land by
/// arrival time, and impacts arriving together in this order, so the
/// outcome never depends on how updates batch time or on entity order.
#[derive(Resource, Debug, Default)]
pub struct FlightOrder(u64);

impl FlightOrder {
    pub(crate) fn stamp(&mut self, flight: &mut SpellInFlight) {
        if flight.launched == 0 {
            self.0 += 1;
            flight.launched = self.0;
        }
    }
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
    Conversation,
    Lock,
    Trap,
}

/// Whether the game can resolve a spell: its effect is modelled, or it is
/// one of the device spells the device interaction resolves.
pub fn resolvable(spell: SpellId) -> bool {
    effects::modelled(spell)
        || super::utilities::spell(spell)
        || matches!(spell, SpellId::DIVINE_TRAP | SpellId::KNOCK_KNOCK)
}

pub fn usable_now(spell: &SpellDefinition, context: UseContext) -> bool {
    match context {
        UseContext::Conversation => super::dialogue::spell(spell.id),
        UseContext::Lock => spell.id == SpellId::KNOCK_KNOCK,
        UseContext::Trap => matches!(spell.id, SpellId::DIVINE_TRAP | SpellId::KNOCK_KNOCK),
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
    /// The way the party faces, level ([`crate::game::party::body::PartyBody::forward`]).
    pub facing: Vec3,
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

    /// The monster an automatic aim takes: of those within reach, the one
    /// nearest the party's heading, the nearer of equals.
    fn ahead(&self, spell: &SpellDefinition) -> Option<&Candidate> {
        let entity = super::target_ahead(
            self.party_at,
            self.facing,
            spell.reach.metres(),
            self.monsters.iter().map(|c| (c.entity, c.at, c.distance)),
        )?;
        self.monsters.iter().find(|c| c.entity == entity)
    }

    fn party_victims(&self) -> Vec<Victim> {
        self.party.iter().map(|&m| Victim::Member(m)).collect()
    }
}

/// Whether `target` lies within a cone's arcs along `aim`, both directions
/// from its apex (retail `TargetInRangeAndArcs`, which tests heading and
/// elevation independently against a 45° half arc). Range is the caller's.
pub(crate) fn in_cone(aim: Vec3, target: Vec3) -> bool {
    let heading = |v: Vec3| v.x.atan2(v.z);
    let elevation = |v: Vec3| v.y.atan2(v.x.hypot(v.z));
    let within = |a: f32, b: f32| {
        let difference = a - b;
        difference.sin().atan2(difference.cos()).abs() < std::f32::consts::FRAC_PI_4
    };
    within(heading(aim), heading(target)) && within(elevation(aim), elevation(target))
}

/// The victims a spell takes when it is cast (retail
/// `PopulateSpellTargetMarkers`), and where its missile flies. Later movement
/// adds nobody.
pub(crate) fn capture(
    caster: Entity,
    spell: &SpellDefinition,
    aim: Aim,
    situation: &Situation,
    forced_self: bool,
) -> Result<(Vec<Victim>, Option<Vec3>), Refusal> {
    if forced_self {
        return Ok((vec![Victim::Member(caster)], None));
    }
    let aimed = || match aim {
        Aim::Monster(m) => situation.candidate(m, spell).copied().ok_or(Refusal::NoTarget),
        Aim::Ahead => situation.ahead(spell).copied().ok_or(Refusal::NoTarget),
        Aim::Point(_) | Aim::Member(_) | Aim::Device(_) | Aim::Item(_) => Err(Refusal::NoTarget),
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
            let toward = target.at - situation.party_at;
            let victims = situation
                .monsters
                .iter()
                .filter(|c| c.distance <= reach && in_cone(toward, c.at - situation.party_at))
                .map(|c| Victim::Monster(c.entity))
                .collect();
            (victims, Some(target.at))
        }
        // A blast centred where it lands, the party too if it is that close.
        Targeting::Radius => {
            let target = aimed()?;
            (Vec::new(), Some(target.at))
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
    power: u32,
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
            let shortfall = spell.level * 15;
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
                + power as i32;
            if adjust > 0 {
                failure += spell.level * adjust as u32;
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
    power: u32,
) -> Strength {
    let figure = match definition.use_type {
        UseType::Instrument => u32::from(character.profession_levels[Profession::Bard]),
        UseType::Gadget => u32::from(character.profession_levels[Profession::Gadgeteer]),
        _ => u32::from(character.level()),
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
    affordable: u32,
) -> u32 {
    let fail = |p| super::fail_chance(character, spell, p);
    let Some((profile, vitals)) = victim else { return 1 };
    let power = if effects::restores(spell.id) {
        let missing = if spell.id == SpellId::STAMINA {
            profile.stats(&vitals.enchantments).max_stamina() as f32 - vitals.stamina as f32
        } else {
            profile.stats(&vitals.enchantments).max_hp() as f32 - vitals.hp as f32
        }
        .max(0.0);
        let average = |p: u32| {
            spell.dice.map_or(0.0, |d| {
                d.count as f32 * p as f32 * (d.sides as f32 + 1.0) / 2.0 + d.base as f32
            })
        };
        if missing == 0.0 {
            1
        } else {
            (1..=MAX_POWER)
                .find_map(|p| {
                    if fail(p) > 10 {
                        Some((p - 1).max(1))
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
                Lasting::Rounds(t) => Some(t),
                Lasting::Indefinite => None,
            })
            .max();
        match turns {
            None => 1,
            Some(turns) => (1..=affordable.max(1))
                .map(|p| {
                    let square = p.pow(2);
                    let per = (square - fail(p) * square / 100).max(1);
                    (spell.cost * turns.div_ceil(per) * p, p)
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
    from_pool: bool,
}

struct Prepared<'a> {
    spell: &'a SpellDefinition,
    power: u32,
    item: Option<ItemUse<'a>>,
    forced_self: bool,
}

/// Shared menu/commit activation rule; alternate hands are not worn.
pub fn can_activate(character: &Character, item: &ItemDefinition, worn: bool) -> bool {
    !matches!(item.use_type, UseType::None | UseType::Spellbook)
        && (!item.requires_worn() || worn)
        && (character.can_use(item) || consumed_by_anyone(item))
}

impl<'w> Field<'w, '_> {
    /// The source as it stands now, refused if it cannot be used.
    fn prepare(
        &self,
        caster: Entity,
        source: Source,
        in_combat: bool,
    ) -> Result<Prepared<'w>, Refusal> {
        let (spells, items) = (self.spells(), self.items());
        let (profile, vitals, carried, ..) = self.members.get(caster).unwrap();
        if !vitals.can_fight() {
            return Err(Refusal::CannotAct);
        }
        // What the source is, before anything is spent.
        let (spell, power, item, forced_self) = match source {
            Source::Spell { spell, power } => {
                if power == 0 {
                    return Err(Refusal::CannotUse);
                }
                if vitals.conditions.has(Condition::Silenced) {
                    return Err(Refusal::Silenced);
                }
                let (learned, _) = self.points.get(caster).unwrap();
                if !learned.0.contains(&spell) {
                    return Err(Refusal::NotLearned);
                }
                let definition = &spells[spell];
                if self.affordable(caster, definition) == 0 {
                    return Err(Refusal::NoSpellPoints);
                }
                (definition, power.min(AUTO_POWER), None, false)
            }
            Source::Item(id) | Source::PoolItem(id) => {
                let from_pool = matches!(source, Source::PoolItem(_));
                let stack = if from_pool {
                    self.inventory.pool.stacks.iter().find(|s| s.id == id)
                } else {
                    carried.find_stack(id)
                }
                .ok_or(Refusal::NoItem)?;
                let definition = &items[stack.item];
                let cast = definition.spell.ok_or(Refusal::CannotUse)?;
                if cast.power == 0
                    && !matches!(definition.use_type, UseType::Instrument | UseType::Gadget)
                {
                    return Err(Refusal::CannotUse);
                }
                let worn = !from_pool && carried.active_equipment().any(|(_, e)| e.stack.id == id);
                if !can_activate(profile, definition, worn) {
                    return Err(Refusal::CannotUse);
                }
                if stack.remaining_uses() == Some(0) {
                    return Err(Refusal::NoCharges);
                }
                if definition.use_type == UseType::Instrument
                    && vitals.conditions.has(Condition::Silenced)
                {
                    return Err(Refusal::Silenced);
                }
                let spell = &spells[cast.spell];
                // Drinking in combat is always done by the drinker (retail
                // `UseItem` aims potions at their user).
                let forced = in_combat && consumed_by_anyone(definition);
                (
                    spell,
                    cast.power,
                    Some(ItemUse { id, definition, identified: stack.identified, from_pool }),
                    forced,
                )
            }
        };
        Ok(Prepared { spell, power, item, forced_self })
    }
}

/// Cast or use as `caster`, now: revalidate, capture victims, roll, pay and
/// resolve. Nothing is spent on refusal. Missiles are returned to be flown.
pub fn perform(
    caster: Entity,
    choice: MagicChoice,
    situation: &Situation,
    world: &mut Field,
) -> Result<(Report, Vec<SpellInFlight>), Refusal> {
    let Prepared { spell, power, item, forced_self } =
        world.prepare(caster, choice.source, situation.in_combat())?;
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
    let (captured, landing) = capture(caster, spell, choice.aim, situation, forced_self)?;
    let power = world.play(caster, item.as_ref(), spell, power)?;
    let mut report = Report {
        caster,
        spell: spell.id,
        power: 0,
        item: item.as_ref().map(|use_| use_.definition.name.clone()),
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
                let affordable = world.affordable(caster, spell);
                if affordable == 0 {
                    break;
                }
                if auto {
                    let victim = match choice.aim {
                        Aim::Member(m) => Some(world.members.get(m).unwrap()),
                        _ => None,
                    };
                    auto_power(
                        world.members.get(caster).unwrap().0,
                        spell,
                        victim.map(|(c, v, ..)| (c, v)),
                        affordable,
                    )
                } else {
                    power.min(affordable)
                }
            }
        };
        let mut victims = captured.clone();
        if spell.targeting == Targeting::Radius && !forced_self {
            let centre = landing.unwrap();
            victims = super::targeting::radius(
                world,
                centre,
                centre,
                spell.radius + spell.radius_per_power * cast_power as f32,
                Some(situation.party_at),
                super::targeting::Recipients::Opponents(crate::game::world::hostility::own_stance(
                    crate::game::world::hostility::Stance::Friendly,
                    world.members.get(caster).unwrap().1.conditions.has(Condition::Turncoat),
                )),
            );
        }
        // A spell aimed at enemies provokes those it is cast at, whatever
        // the roll (retail `MakeTargetGroupHostile` as the action starts).
        if first && spell.aimed_at_enemies() {
            for victim in &victims {
                if let Victim::Monster(monster) = *victim {
                    world.provoke(monster);
                }
            }
        }
        // Accepted: roll, then pay whatever the roll.
        let (outcome, strength) = world.attempt(caster, spell, cast_power, item.as_ref());
        let victims = match outcome {
            Outcome::Cast => victims,
            Outcome::Backfired => backfire_victims(caster, spell, situation),
            Outcome::Fizzled => Vec::new(),
        };
        // Missiles fly to their victims, one each, or to where they burst; a
        // backfire turns on the party at once.
        let flies = spell.missile.is_some() && outcome == Outcome::Cast && !victims.is_empty();
        let origin = if outcome == Outcome::Backfired {
            match spell.targeting {
                Targeting::Enemy | Targeting::Ally => match choice.aim {
                    Aim::Monster(entity) => Origin::Actor(Victim::Monster(entity)),
                    Aim::Member(entity) => Origin::Actor(Victim::Member(entity)),
                    _ => Origin::Actor(Victim::Member(caster)),
                },
                Targeting::EnemyGroup => match choice.aim {
                    Aim::Monster(entity) => {
                        Origin::Actor(Victim::Monster(world.monsters.leader(entity)))
                    }
                    _ => Origin::Actor(Victim::Member(caster)),
                },
                Targeting::Radius | Targeting::Cone => Origin::Point {
                    at: landing.unwrap_or(situation.party_at),
                    owner: Some(Victim::Member(caster)),
                },
                _ => Origin::Actor(Victim::Member(caster)),
            }
        } else {
            Origin::Actor(Victim::Member(caster))
        };
        let strength = if outcome == Outcome::Backfired {
            Strength::at(origin.figure(world, spell), spell, strength.power, strength.boost)
        } else {
            strength
        };
        let finish = Finalization {
            aim: landing
                .filter(|_| matches!(spell.targeting, Targeting::Radius | Targeting::Cone))
                .map_or(choice.aim, Aim::Point),
            victims: victims.clone(),
            reflected: false,
        };
        let first_flight = flights.len();
        let landed = if flies {
            // One missile to where a burst lands, else one to each victim.
            let flight = |victims: Vec<Victim>, to: Vec3| SpellInFlight {
                origin,
                finish: None,
                caster,
                spell: spell.id,
                strength,
                victims,
                arrives: Timer::from_seconds(
                    to.distance(situation.party_at) / MISSILE_SPEED,
                    TimerMode::Once,
                ),
                launched: 0,
            };
            if spell.targeting == Targeting::Radius {
                flights.push(flight(victims.clone(), landing.unwrap_or(situation.party_at)));
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
                    flights.push(flight(vec![victim], to));
                }
            }
            super::impacts::attach(&mut flights[first_flight..], finish);
            Landed::default()
        } else {
            super::impacts::resolve(
                caster,
                origin,
                outcome == Outcome::Backfired,
                spell,
                strength,
                &victims,
                Some(&finish),
                situation.party_at,
                world,
                &mut flights,
            )
        };
        // Practice follows what the cast did; a missile on its way counts.
        report.improved.extend(landed.improved);
        if outcome != Outcome::Fizzled && (flies || effects::affected(&landed.effects)) {
            report.improved.extend(world.practise(caster, spell, cast_power, item.as_ref()));
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
        report.effects.extend(landed.effects);
        // An auto cast tries again while it fizzles, and auto recast goes on
        // until its victim is whole, while stamina and points last.
        if !auto || report.repeats >= 50 {
            break;
        }
        let (_, vitals, ..) = world.members.get(caster).unwrap();
        let (_, pool) = world.points.get(caster).unwrap();
        let maximum = world.mana_maximum(caster);
        if vitals.stamina <= spell.level
            || pool.available(spell.realm.unwrap(), &maximum) < spell.cost
            || outcome != Outcome::Fizzled
                && (!repeat || effects::all_restored(spell.id, &victims, &world.members))
        {
            break;
        }
    }
    Ok((report, flights))
}

impl Field<'_, '_> {
    /// Send missiles on their way, owned by the level being played.
    pub fn launch(&mut self, commands: &mut Commands, flights: Vec<SpellInFlight>) {
        let level = self.level.0;
        for mut flight in flights {
            self.flight_order.stamp(&mut flight);
            commands.spawn((flight, crate::game::world::level::LevelOwned(level)));
        }
    }

    /// The most power the caster's points pay for now.
    fn affordable(&self, caster: Entity, spell: &SpellDefinition) -> u32 {
        let (_, pool) = self.points.get(caster).unwrap();
        pool.affordable_power(spell, &self.mana_maximum(caster))
    }

    /// Cast from the source: roll, pay whatever the roll and practise. The
    /// caller validates its target first and applies the consequence.
    pub fn pay(
        &mut self,
        caster: Entity,
        source: Source,
        context: UseContext,
    ) -> Result<(Report, Strength), Refusal> {
        if !self.party.contains(&caster) {
            return Err(Refusal::CannotAct);
        }
        let Prepared { spell, power, item, .. } = self.prepare(caster, source, false)?;
        if !usable_now(spell, context) {
            return Err(Refusal::WrongTime);
        }
        let item = item.as_ref();
        let mut power = self.play(caster, item, spell, power)?;
        if item.is_none() {
            power = power.min(self.affordable(caster, spell));
        }
        let (outcome, strength) = self.attempt(caster, spell, power, item);
        // Retail considers these utility spells affected even on a fizzle.
        let report = Report {
            caster,
            spell: spell.id,
            power,
            item: item.map(|use_| use_.definition.name.clone()),
            outcome,
            effects: Vec::new(),
            improved: self.practise(caster, spell, power, item),
            repeats: 0,
        };
        Ok((report, strength))
    }

    /// One accepted attempt at `power`: roll, then pay whatever the roll,
    /// points or a use, then fatigue.
    fn attempt(
        &mut self,
        caster: Entity,
        spell: &SpellDefinition,
        power: u32,
        item: Option<&ItemUse>,
    ) -> (Outcome, Strength) {
        let underwater = self.underwater();
        let difficulty = self.difficulty();
        let character = self.members.get(caster).unwrap().0;
        let (chance, quiet, strength) = match item {
            Some(use_) => {
                let (chance, quiet) = item_failure(
                    character,
                    use_.definition,
                    spell,
                    power,
                    use_.identified,
                    difficulty,
                );
                (chance, quiet, item_strength(character, use_.definition, spell, power))
            }
            None => (
                difficulty.scale_by_pace(super::fail_chance(character, spell, power), 0),
                false,
                Strength::of(character, spell, power),
            ),
        };
        let outcome = super::roll(&mut self.rng, chance, spell, quiet, underwater);
        let (_, mut vitals, mut carried, burden) = self.members.get_mut(caster).unwrap();
        let fatigue = match item {
            None => {
                let (_, mut pool) = self.points.get_mut(caster).unwrap();
                let realm = spell.realm.expect("a cast spell has a realm");
                pool.spend(realm, spell.cost_at(power));
                spell.level + power
            }
            Some(use_) => {
                if use_.from_pool {
                    self.inventory.pool.consume_use(use_.id, use_.definition);
                } else {
                    carried.consume_use(use_.id, use_.definition, use_.definition.keeps_empty());
                }
                match use_.definition.use_type {
                    UseType::Thrown => 10,
                    // Already paid when the instrument or gadget was played.
                    UseType::Instrument | UseType::Gadget => 0,
                    _ => {
                        let base = self.rng.below(2) + 1;
                        if vitals.conditions.has(Condition::Diseased) { base * 2 } else { base }
                    }
                }
            }
        };
        // Action fatigue weighs more under load (retail `FatigueCharacter`).
        let slowed = vitals.conditions.has(Condition::Slowed);
        let hasted = vitals.enchantments.has(SpellId::HASTE);
        vitals.spend_stamina(burden.load.scale_fatigue(fatigue, slowed, hasted));
        (outcome, strength)
    }

    /// Instruments and gadgets find the power their player can manage, paying
    /// their fatigue whether or not they can (retail `UseItem`); any other
    /// source keeps its `power`.
    fn play(
        &mut self,
        caster: Entity,
        item: Option<&ItemUse>,
        spell: &SpellDefinition,
        power: u32,
    ) -> Result<u32, Refusal> {
        let Some(use_) = item.filter(|use_| {
            matches!(use_.definition.use_type, UseType::Instrument | UseType::Gadget)
        }) else {
            return Ok(power);
        };
        let (mut profile, mut vitals, ..) = self.members.get_mut(caster).unwrap();
        let mut power = MAX_POWER;
        while power > 0
            && item_failure(&profile, use_.definition, spell, power, true, Difficulty::Normal).0
                > 15
        {
            power -= 1;
        }
        while power > 0 && power * 7 > self.rng.below(100) {
            power -= 1;
        }
        vitals.spend_stamina(spell.level * 8 + 20);
        if power == 0 {
            // The refusal carries the practice, since no report follows it.
            let improved = use_
                .definition
                .use_type
                .skill()
                .filter(|&skill| profile.practise(skill, 1, &mut self.rng));
            return Err(Refusal::PlayedBadly { improved });
        }
        Ok(power)
    }

    /// Practice from a cast that did something (retail `Magic.cpp:2682-2692`,
    /// `PC Item.cpp:3792-3796`): a learned spell trains its book, its realm and
    /// Power Cast; an item its use's skill.
    fn practise(
        &mut self,
        caster: Entity,
        spell: &SpellDefinition,
        power: u32,
        item: Option<&ItemUse>,
    ) -> Vec<(Entity, Skill)> {
        let (mut character, ..) = self.members.get_mut(caster).unwrap();
        let practice: Vec<(Skill, u32)> = match item {
            None => {
                let index = spell.level + power;
                let mut practice = vec![(super::best_book(&character, spell), (index + 2) >> 2)];
                if let Some(realm) = spell.realm {
                    practice.push((realm.skill(), index));
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
                .map(|skill| vec![(skill, spell.level + power / 2)])
                .unwrap_or_default(),
        };
        practice
            .into_iter()
            .filter(|&(skill, usage)| character.practise(skill, usage, &mut self.rng))
            .map(|(skill, _)| (caster, skill))
            .collect()
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
            let worn =
                carried.active_equipment().any(|(_, equipped)| equipped.stack.id == stack.id);
            Some((stack.id, usable_stack(character, stack, items, worn)?))
        })
        .collect()
}

/// The spell a stack casts for `character`, if they can use it now: one
/// they may activate (worn, if it must be), with a charge left, whose
/// effect the game resolves.
pub fn usable_stack(
    character: &Character,
    stack: &crate::game::inventory::ItemStack,
    items: &crate::content::items::ItemCatalog,
    worn: bool,
) -> Option<SpellId> {
    let definition = &items[stack.item];
    let cast = definition.spell?;
    (can_activate(character, definition, worn)
        && stack.remaining_uses() != Some(0)
        && resolvable(cast.spell))
    .then_some(cast.spell)
}

/// Whether a learned spell can be cast in `context` with `affordable`
/// power within reach: as the cast itself checks, points for it, an effect
/// the game resolves, the moment for it and no silence.
pub fn castable(
    spell: &SpellDefinition,
    affordable: u32,
    context: UseContext,
    silenced: bool,
) -> bool {
    affordable > 0 && resolvable(spell.id) && usable_now(spell, context) && !silenced
}
