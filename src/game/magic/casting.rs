//! Casting a learned spell, using an item's spell, or a monster's innate
//! casting (retail `ExecuteCharacterSpellCast`, `UseItem`, `CastItemSpell`,
//! `CastSpellFromSource`, `MonsterCastsSpell`). An accepted cast is an
//! entity ([`Cast`]) made [`By`] its caster; [`resolve_casts`] aims it and
//! judges it without spending, then pays, rolls and releases it as the act
//! that lands ([`crate::game::hits`]): a fizzle, a backfire, or its victims
//! and missiles.
//!
//! Choosing never spends anything. A refused cast spends nothing; one that
//! is rolled pays even when it fizzles or backfires: spell points for a
//! learned spell, one use for an item, nothing for an innate cast. Its act
//! keeps its own copy of the spell and victims, so using the last potion
//! still heals, and a missile in flight still lands after its caster falls.

use bevy::prelude::*;

use super::{LearnedSpells, Outcome, SpellPoints, Strength};
use crate::content::characters::Skill;
use crate::content::items::{ItemCatalog, ItemDefinition, UseType};
use crate::content::spells::{
    MAX_POWER, PowerChoice, SpellDefinition, SpellId, Targeting, Usable, min_caster_level,
};
use crate::game::characters::{Character, Condition, Lasting, Profession, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::{Definitions, EffectKind, Uses};
use crate::game::hits::reach::{Recipients, within};
use crate::game::hits::spells::{group_of, leader, reach};
use crate::game::hits::{Act, By, Deed, DeedHits, Effect};
use crate::game::inventory::{Carried, PartyInventory, StackId};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::{self, Loyalty, Provocation, Side, Stance};
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::level::Level;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Untargetable;

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
    /// A monster's own casting, which costs nothing: its spell budget
    /// stands in for a caster's skill, its level sets the strength.
    Innate { spell: SpellId, power: u32, budget: u32, level: u8 },
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
            Self::Spell { spell, .. } | Self::Innate { spell, .. } => Some(spell),
            Self::Item(id) => items[carried.find_stack(id)?.item].spell.map(|s| s.spell),
            Self::PoolItem(id) => items[inventory.pool.stacks.iter().find(|s| s.id == id)?.item]
                .spell
                .map(|s| s.spell),
        }
    }
}

/// Whom the caster aimed at; group, area, party and self spells widen or
/// replace it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Point(Vec3),
    /// A body, a member's or a monster's.
    Body(Entity),
    /// Whoever stands nearest the caster's heading within the spell's
    /// reach when it is cast (retail's automatic targeting), the nearer of
    /// equals.
    Ahead,
    /// The device attempt under way (its lock or trap), by the attempt.
    Device(Entity),
    /// The selected stack, retained across any delayed request.
    Item(StackId),
}

impl Aim {
    /// The body aimed at, if one was.
    pub fn body(self) -> Option<Entity> {
        match self {
            Aim::Body(entity) => Some(entity),
            _ => None,
        }
    }
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
    pub power: u32,
    /// The item it came from, already spent.
    pub item: Option<String>,
    pub outcome: Outcome,
    /// What landed at once; missiles land later as [`MagicNews::Landed`].
    pub effects: Vec<Effect>,
}

/// A cast's practice, owed once its deed has landed: its caster practises
/// these skills if the cast affected anyone (retail `SpellAffectedTarget`),
/// or whatever came of it once a missile is on its way.
#[derive(Component, Debug)]
pub struct Practice {
    pub caster: Entity,
    pub skills: Vec<(Skill, u32)>,
    pub sure: bool,
}

/// An auto recast (retail's "repeat" power): once it has landed, its caster
/// casts again while its victims are not whole and stamina and points last.
#[derive(Component, Debug)]
pub struct Recast {
    pub caster: Entity,
    pub choice: MagicChoice,
    pub victims: Vec<Entity>,
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

/// Whether the game can resolve a spell: its effects are defined, or it is
/// one of the device spells the device interaction resolves.
pub fn resolvable(spell: SpellId, definitions: &Definitions) -> bool {
    definitions.spell(spell).is_some()
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

/// Whether a body at `from` reaches one at `to` with a spell of `reach`:
/// within it, body to body, along a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `CanMonsterSeeMonster`'s line).
pub fn reaches(from: &Footing, to: &Footing, reach: f32, obstacles: &WorldObstacles) -> bool {
    from.gap(to) <= reach && obstacles.clear(from.eye, to.eye)
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
    victim: Option<&Vitals>,
    affordable: u32,
    restores: bool,
) -> u32 {
    let fail = |p| super::fail_chance(character, spell, p);
    let Some(vitals) = victim else { return 1 };
    let power = if restores {
        let pool = if spell.id == SpellId::STAMINA { vitals.stamina } else { vitals.health };
        let missing = (pool.max as f32 - pool.current as f32).max(0.0);
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

/// Shared menu/commit activation rule; alternate hands are not worn.
pub fn can_activate(character: &Character, item: &ItemDefinition, worn: bool) -> bool {
    !matches!(item.use_type, UseType::None | UseType::Spellbook)
        && (!item.requires_worn() || worn)
        && (character.can_use(item) || consumed_by_anyone(item))
}

/// An item about to be used: its stack, what it is, and where it is kept.
#[derive(Clone, Copy)]
struct ItemUse<'d> {
    id: StackId,
    definition: &'d ItemDefinition,
    identified: bool,
    from_pool: bool,
}

/// The item a source uses, if it is one and is still there.
fn item_use<'d>(
    source: Source,
    carried: &Carried,
    inventory: &PartyInventory,
    items: &'d ItemCatalog,
) -> Option<ItemUse<'d>> {
    let (id, from_pool) = match source {
        Source::Spell { .. } | Source::Innate { .. } => return None,
        Source::Item(id) => (id, false),
        Source::PoolItem(id) => (id, true),
    };
    let stack = if from_pool {
        inventory.pool.stacks.iter().find(|s| s.id == id)
    } else {
        carried.find_stack(id)
    }?;
    Some(ItemUse { id, definition: &items[stack.item], identified: stack.identified, from_pool })
}

/// An accepted cast waiting for the resolution boundary: the spell, when
/// it is cast, and what the caster chose. It is made [`By`] its caster;
/// [`resolve_casts`] turns it into an act ([`crate::game::hits`]) or
/// refuses it.
#[derive(Component, Clone, Copy, Debug)]
pub struct Cast {
    pub spell: SpellId,
    pub context: UseContext,
    pub choice: MagicChoice,
    /// An auto cast's next attempt after a fizzle: it provokes nobody again.
    retry: bool,
}

impl Cast {
    pub fn new(spell: SpellId, context: UseContext, choice: MagicChoice) -> Self {
        Self { spell, context, choice, retry: false }
    }
}

/// What an attempt is: how likely it fails, whether it is a quiet (thrown)
/// use, and the strength it goes off at.
#[derive(Clone, Copy, Debug)]
struct Attempt {
    failure: u32,
    quiet: bool,
    strength: Strength,
}

/// What a backfire does instead (retail `PrepareSpellTarget`): a spell
/// aimed at one body turns on the caster, a wider one on everyone of the
/// caster's side in the fight within its reach, a blast on those within
/// its radius of the caster. It comes from whom it was aimed at, or where
/// it was to land, and is as strong as they are.
#[derive(Debug)]
struct Backfire {
    owner: Entity,
    from: Vec3,
    aim: Aim,
    victims: Vec<Entity>,
}

/// A cast aimed and judged, nothing spent yet.
#[derive(Debug)]
struct Aimed {
    victims: Vec<Entity>,
    /// The point a blast lands on or a cone fans toward; `None` for a cast
    /// aimed at bodies only.
    target: Option<Vec3>,
    power: u32,
    attempt: Attempt,
    /// An instrument's or gadget's toll on its player, paid however it
    /// played.
    instrument: Option<u32>,
    backfire: Backfire,
}

/// How far a cast got before anything is spent.
#[derive(Debug)]
enum Plan {
    Refused(Refusal),
    /// An instrument or gadget that could not be brought to play: the
    /// player is tired by `fatigue` and practises `skill`.
    PlayedBadly {
        fatigue: u32,
        skill: Option<Skill>,
    },
    Aimed(Aimed),
}

/// Resolve every accepted cast, in one pass each: aimed and judged without
/// spending, then paid, rolled and released as an act, or refused. An auto
/// cast that fizzles is cast again here while its caster can.
pub fn resolve_casts(world: &mut World) {
    while let Some(cast) = world.query_filtered::<Entity, With<Cast>>().iter(world).next() {
        let plan = world.run_system_cached_with(aim, cast).unwrap();
        world.run_system_cached_with(commit, (cast, plan)).unwrap();
    }
}

/// Whether the cast can be made at all (retail `ExecuteCharacterSpellCast`,
/// `UseItem`): by someone able to fight, in the party unless innate, not
/// silenced for a voiced cast, of a learned spell with points for it or an
/// item still there that its user may activate with a charge left, of a
/// spell the game resolves, at a moment it may be cast.
fn check(
    cast: &Cast,
    fighter: Option<(&Vitals, bool)>,
    character: Option<(&Character, &Carried)>,
    spellbook: Option<(&LearnedSpells, &SpellPoints)>,
    inventory: &PartyInventory,
    content: &Catalogs,
    definitions: &Definitions,
) -> Result<(), Refusal> {
    let Cast { spell, context, choice, .. } = *cast;
    let definition = &content.spells[spell];
    let innate = matches!(choice.source, Source::Innate { .. });
    let Some((vitals, _)) =
        fighter.filter(|(vitals, member)| vitals.can_fight() && (innate || *member))
    else {
        return Err(Refusal::CannotAct);
    };
    if !innate && let Some((_, carried)) = character {
        let item = item_use(choice.source, carried, inventory, &content.items);
        let voiced = item.is_none_or(|item| item.definition.use_type == UseType::Instrument);
        if voiced && vitals.conditions.has(Condition::Silenced) {
            return Err(Refusal::Silenced);
        }
    }
    if let Source::Spell { spell, power } = choice.source {
        if power == 0 {
            return Err(Refusal::CannotUse);
        }
        if !spellbook.is_some_and(|(learned, _)| learned.0.contains(&spell)) {
            return Err(Refusal::NotLearned);
        }
        let affordable = character.zip(spellbook).map_or(0, |((character, _), (learned, pool))| {
            let maximum =
                character.stats(&vitals.enchantments).max_points(learned, &content.spells);
            pool.affordable_power(definition, &maximum)
        });
        if affordable == 0 {
            return Err(Refusal::NoSpellPoints);
        }
    }
    let (id, from_pool) = match choice.source {
        Source::Spell { .. } | Source::Innate { .. } => (None, false),
        Source::Item(id) => (Some(id), false),
        Source::PoolItem(id) => (Some(id), true),
    };
    if let Some(id) = id
        && let Some((character, carried)) = character
    {
        let stack = if from_pool {
            inventory.pool.stacks.iter().find(|s| s.id == id)
        } else {
            carried.find_stack(id)
        };
        let Some(stack) = stack else { return Err(Refusal::NoItem) };
        let item = &content.items[stack.item];
        let worn = !from_pool && carried.active_equipment().any(|(_, e)| e.stack.id == id);
        let Some(uses) = item.spell else { return Err(Refusal::CannotUse) };
        let powerless =
            uses.power == 0 && !matches!(item.use_type, UseType::Instrument | UseType::Gadget);
        if powerless || !can_activate(character, item, worn) {
            return Err(Refusal::CannotUse);
        }
        if stack.remaining_uses() == Some(0) {
            return Err(Refusal::NoCharges);
        }
    }
    if definitions.spell(spell).is_none() {
        return Err(Refusal::Unsupported);
    }
    let repeats = choice.source == Source::Spell { spell, power: AUTO_POWER }
        && definition.power == PowerChoice::ChosenOrRepeat;
    if !innate && (!usable_now(definition, context) || (repeats && context == UseContext::Combat)) {
        return Err(Refusal::WrongTime);
    }
    Ok(())
}

/// The living foes a caster's spell reaches, in the order the fight takes
/// bodies: the party's members, then the fight's monsters.
fn foes_reached(
    caster: Entity,
    reach: f32,
    roster: &PartyRoster,
    combatants: &[Entity],
    bodies: &Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    obstacles: &WorldObstacles,
) -> Vec<Entity> {
    let Ok((_, from, side, loyalty)) = bodies.get(caster) else { return vec![] };
    let mine = (side.0, *loyalty);
    crate::game::combat::participants(roster, combatants)
        .filter(|&other| {
            other != caster
                && bodies.get(other).is_ok_and(|(vitals, to, their_side, their_loyalty)| {
                    !vitals.is_dead()
                        && hostility::between(mine, (their_side.0, *their_loyalty))
                            == Stance::Hostile
                        && reaches(from, to, reach, obstacles)
                })
        })
        .collect()
}

/// The foe an aim takes among those reached: the one aimed at, or for an
/// automatic aim the one nearest the caster's heading, the nearer of
/// equals.
fn aimed_foe(
    aim: Aim,
    foes: &[Entity],
    from: &Footing,
    footings: &Query<&Footing>,
) -> Option<Entity> {
    match aim {
        Aim::Ahead => super::target_ahead(
            from.eye,
            from.forward(),
            f32::INFINITY,
            foes.iter().filter_map(|&foe| {
                let at = footings.get(foe).ok()?;
                Some((foe, at.feet, from.gap(at)))
            }),
        ),
        aim => aim.body().filter(|aimed| foes.contains(aimed)),
    }
}

/// Whom the cast reaches and where it is aimed, by the spell's targeting
/// (retail `PopulateSpellTargetMarkers`): its caster, the friend aimed at
/// if on the caster's side and in reach, every member, the foe aimed at or
/// nearest ahead, that foe's group, every foe in reach, or everyone in a
/// cone toward the foe. A blast's victims wait for its power.
#[allow(clippy::too_many_arguments)]
fn aim_at(
    caster: Entity,
    aim: Aim,
    targeting: Targeting,
    spell: &SpellDefinition,
    bodies: &Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    footings: &Query<&Footing>,
    (roster, combatants): (&PartyRoster, &[Entity]),
    (memberships, groups): (&Query<&GroupMember>, &Query<&GroupMembers>),
    obstacles: &WorldObstacles,
) -> Result<(Vec<Entity>, Option<Vec3>), Refusal> {
    let reach_of = spell.reach.metres();
    let foes = || foes_reached(caster, reach_of, roster, combatants, bodies, obstacles);
    let feet = |body: Entity| footings.get(body).map_or(Vec3::ZERO, |at| at.feet);
    match targeting {
        Targeting::Caster => Ok((vec![caster], None)),
        Targeting::Ally => {
            let friend = aim.body().and_then(|aimed| {
                let (_, from, side, loyalty) = bodies.get(caster).ok()?;
                let (_, to, their_side, their_loyalty) = bodies.get(aimed).ok()?;
                let friendly = aimed == caster
                    || hostility::between((side.0, *loyalty), (their_side.0, *their_loyalty))
                        == Stance::Friendly;
                (friendly && reaches(from, to, reach_of, obstacles)).then_some(aimed)
            });
            friend.map(|friend| (vec![friend], None)).ok_or(Refusal::NoTarget)
        }
        Targeting::Party => Ok((roster.iter().collect(), None)),
        Targeting::Enemy | Targeting::Radius => {
            let foes = foes();
            let from = footings.get(caster).map_err(|_| Refusal::NoTarget)?;
            let target = aimed_foe(aim, &foes, from, footings).ok_or(Refusal::NoTarget)?;
            let victims = if targeting == Targeting::Enemy { vec![target] } else { vec![] };
            Ok((victims, Some(feet(target))))
        }
        Targeting::EnemyGroup => {
            let foes = foes();
            let from = footings.get(caster).map_err(|_| Refusal::NoTarget)?;
            let first = aimed_foe(aim, &foes, from, footings).ok_or(Refusal::NoTarget)?;
            let mut victims = vec![first];
            for mate in group_of(first, roster, memberships, groups) {
                if mate != first && foes.contains(&mate) {
                    victims.push(mate);
                }
            }
            Ok((victims, Some(feet(first))))
        }
        Targeting::AllEnemies => {
            let victims = foes();
            if victims.is_empty() { Err(Refusal::NoTarget) } else { Ok((victims, None)) }
        }
        Targeting::Cone => {
            let foes = foes();
            let from = footings.get(caster).map_err(|_| Refusal::NoTarget)?;
            let target = aimed_foe(aim, &foes, from, footings).ok_or(Refusal::NoTarget)?;
            let toward = footings.get(target).map_or(from.eye, |at| at.feet) - from.eye;
            let victims = crate::game::combat::participants(roster, combatants)
                .filter(|&other| other != caster)
                .filter(|&other| {
                    bodies.get(other).is_ok_and(|(vitals, at, ..)| {
                        !vitals.is_dead()
                            && reaches(from, at, reach_of, obstacles)
                            && in_cone(toward, at.feet - from.eye)
                    })
                })
                .collect();
            Ok((victims, Some(from.eye + toward)))
        }
        Targeting::Point | Targeting::Item | Targeting::LockOrTrap => Err(Refusal::Unsupported),
    }
}

/// The power an instrument or gadget is played at: as high as its player
/// manages, luck allowing (retail `UseItem`); 0 when played badly.
fn played_power(
    character: &Character,
    item: &ItemDefinition,
    spell: &SpellDefinition,
    rng: &mut GameRng,
) -> u32 {
    let mut power = MAX_POWER;
    while power > 0 && item_failure(character, item, spell, power, true, Difficulty::Normal).0 > 15
    {
        power -= 1;
    }
    while power > 0 && power * 7 > rng.below(100) {
        power -= 1;
    }
    power
}

/// The power a learned spell goes off at: the chosen one, or for an auto
/// cast what its victim needs, within what the caster can afford now.
fn spell_power(
    power: u32,
    spell: &SpellDefinition,
    (character, vitals, learned, pool): (&Character, &Vitals, &LearnedSpells, &SpellPoints),
    victim: Option<&Vitals>,
    content: &Catalogs,
    definitions: &Definitions,
) -> u32 {
    let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
    let affordable = pool.affordable_power(spell, &maximum);
    if power != AUTO_POWER {
        return power.min(affordable);
    }
    let restores = definitions.spell(spell.id).is_some_and(|id| {
        definitions
            .get(id)
            .effects
            .iter()
            .any(|e| matches!(e.kind, EffectKind::Heal | EffectKind::Refresh))
    });
    auto_power(character, spell, victim, affordable, restores)
}

/// Aim and judge the cast without spending anything: the checks that
/// refuse it, whom it reaches, the power it goes off at, its blast's
/// victims, what it is as an attempt, and what its backfire would do.
#[allow(clippy::too_many_arguments)]
fn aim(
    In(cast): In<Entity>,
    casts: Query<(&Cast, &By)>,
    fighters: Query<(&Vitals, Has<PartyMember>)>,
    characters: Query<(&Character, &Carried)>,
    spellbooks: Query<(&LearnedSpells, &SpellPoints)>,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    blast_bodies: Query<(Entity, &Vitals, &Side, &Footing), Without<Untargetable>>,
    footings: Query<&Footing>,
    (memberships, groups): (Query<&GroupMember>, Query<&GroupMembers>),
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    inventory: Single<&PartyInventory>,
    (content, definitions, difficulty): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
) -> Plan {
    let (cast, by) = casts.get(cast).unwrap();
    let caster = by.0;
    let MagicChoice { source, aim } = cast.choice;
    let spell = &content.spells[cast.spell];
    let fighter = fighters.get(caster).ok();
    let character = characters.get(caster).ok();
    let spellbook = spellbooks.get(caster).ok();
    if let Err(refusal) =
        check(cast, fighter, character, spellbook, &inventory, &content, &definitions)
    {
        return Plan::Refused(refusal);
    }
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    let item =
        character.and_then(|(_, carried)| item_use(source, carried, &inventory, &content.items));
    // A potion or food used in a fight is taken by its user (retail
    // `UseItem` aims potions at their user).
    let forced = cast.context == UseContext::Combat
        && item.is_some_and(|item| consumed_by_anyone(item.definition));
    let targeting = if forced { Targeting::Caster } else { spell.targeting };
    let aimed = aim_at(
        caster,
        aim,
        targeting,
        spell,
        &bodies,
        &footings,
        (&roster, combatants),
        (&memberships, &groups),
        &obstacles,
    );
    let (mut victims, target) = match aimed {
        Ok(aimed) => aimed,
        Err(refusal) => return Plan::Refused(refusal),
    };
    let instrument = item
        .filter(|item| matches!(item.definition.use_type, UseType::Instrument | UseType::Gadget));
    let mut toll = None;
    let power = match (source, instrument) {
        (_, Some(item)) => {
            let (character, _) = character.unwrap();
            toll = Some(spell.level * 8 + 20);
            let power = played_power(character, item.definition, spell, &mut rng);
            if power == 0 {
                return Plan::PlayedBadly {
                    fatigue: toll.unwrap(),
                    skill: item.definition.use_type.skill(),
                };
            }
            power
        }
        (Source::Innate { power, .. }, _) => power,
        (Source::Spell { power, .. }, _) => {
            let ((character, _), (learned, pool)) = character.zip(spellbook).unwrap();
            let (vitals, _) = fighter.unwrap();
            let victim =
                aim.body().and_then(|body| fighters.get(body).ok()).map(|(vitals, _)| vitals);
            spell_power(
                power,
                spell,
                (character, vitals, learned, pool),
                victim,
                &content,
                &definitions,
            )
        }
        (Source::Item(_) | Source::PoolItem(_), None) => {
            item.and_then(|item| item.definition.spell).map_or(0, |uses| uses.power)
        }
    };
    // A blast takes whoever stands within its radius of where it lands,
    // but those on its caster's side.
    let radius = spell.radius + spell.radius_per_power * power as f32;
    let side = blast_bodies.get(caster).map_or(Stance::Friendly, |(_, _, side, _)| side.0);
    if let (Targeting::Radius, Some(centre)) = (targeting, target) {
        victims = within(
            centre,
            centre,
            radius,
            Recipients::Opponents(side),
            &blast_bodies,
            (&roster, combatants),
            &obstacles,
        );
    }
    let attempt = match source {
        Source::Innate { budget, level, .. } => Attempt {
            failure: super::spell_failure(budget, spell, power),
            quiet: false,
            strength: Strength::at(u32::from(level), spell, power, 0),
        },
        _ => attempt(character.unwrap().0, item, spell, power, *difficulty),
    };
    let stand = footings.get(caster).copied().unwrap_or_default();
    let eye_of = |body: Entity| footings.get(body).map_or(stand.eye, |at| at.eye);
    let (owner, from) = match (targeting, aim) {
        (Targeting::Enemy | Targeting::Ally, Aim::Body(entity)) => (entity, eye_of(entity)),
        (Targeting::EnemyGroup, Aim::Body(entity)) => {
            let leader = leader(entity, combatants, &memberships, &groups);
            (leader, eye_of(leader))
        }
        (Targeting::Radius | Targeting::Cone, _) => (caster, target.unwrap_or(stand.eye)),
        _ => (caster, stand.eye),
    };
    let turned_on = match targeting {
        Targeting::Caster | Targeting::Ally | Targeting::Enemy => vec![caster],
        Targeting::Radius => within(
            stand.feet,
            stand.feet,
            radius,
            Recipients::Opponents(side).reversed(),
            &blast_bodies,
            (&roster, combatants),
            &obstacles,
        ),
        _ => crate::game::combat::participants(&roster, combatants)
            .filter(|&other| {
                blast_bodies.get(other).is_ok_and(|(_, vitals, their_side, at)| {
                    !vitals.is_dead()
                        && their_side.0 == side
                        && (other == caster
                            || reaches(&stand, at, spell.reach.metres(), &obstacles))
                })
            })
            .collect(),
    };
    let backfire_aim = match (targeting, target) {
        (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
        _ => aim,
    };
    Plan::Aimed(Aimed {
        victims,
        target,
        power,
        attempt,
        instrument: toll,
        backfire: Backfire { owner, from, aim: backfire_aim, victims: turned_on },
    })
}

/// Carry the cast out (retail `CastSpellFromSource`): refuse it with a
/// word to the player, or provoke whom a member aims at, roll it, pay
/// for it whatever the roll, tire the caster, and release it as an act:
/// a fizzle leaves only its record, and an auto cast tries again while
/// stamina and points last; a backfire turns on the caster's side; a cast
/// that went off takes its victims, its missiles landing as they arrive.
/// In a fight it counts as an attack.
#[allow(clippy::too_many_arguments)]
fn commit(
    In((cast, plan)): In<(Entity, Plan)>,
    mut commands: Commands,
    casts: Query<(&Cast, &By)>,
    mut characters: Query<&mut Character>,
    mut vitals: Query<&mut Vitals>,
    mut points: Query<&mut SpellPoints>,
    mut carried: Query<&mut Carried>,
    burdens: Query<&crate::game::inventory::Burden>,
    spellbooks: Query<&LearnedSpells>,
    members: Query<(), With<PartyMember>>,
    footings: Query<&Footing>,
    mut inventory: Single<&mut PartyInventory>,
    states: Query<&crate::game::world::triggers::LevelState, With<Level>>,
    (content, definitions): (Res<Catalogs>, Res<Definitions>),
    mut round: Option<Single<&mut crate::game::combat::Round>>,
    mut rng: ResMut<GameRng>,
    (mut news, mut provocations): (MessageWriter<MagicNews>, MessageWriter<Provocation>),
) {
    let (&Cast { spell, context, choice, retry }, by) = casts.get(cast).unwrap();
    let caster = by.0;
    let source = choice.source;
    let definition = &content.spells[spell];
    let refuse = |commands: &mut Commands, news: &mut MessageWriter<MagicNews>, refusal| {
        news.write(MagicNews::Refused { caster, refusal });
        commands.entity(cast).despawn();
    };
    let aimed = match plan {
        Plan::Refused(refusal) => return refuse(&mut commands, &mut news, refusal),
        Plan::PlayedBadly { fatigue, skill } => {
            vitals.get_mut(caster).unwrap().tire(fatigue);
            let improved = skill
                .filter(|&skill| characters.get_mut(caster).unwrap().practise(skill, 1, &mut rng));
            return refuse(&mut commands, &mut news, Refusal::PlayedBadly { improved });
        }
        Plan::Aimed(aimed) => aimed,
    };
    let Aimed { victims, target, power, attempt, instrument, backfire } = aimed;
    if let Some(fatigue) = instrument {
        vitals.get_mut(caster).unwrap().tire(fatigue);
    }
    // A party member's spell aimed at enemies provokes those it is cast
    // at, whatever the roll (retail `MakeTargetGroupHostile` as the action
    // starts); a retry provokes nobody again.
    if !retry && members.contains(caster) && definition.aimed_at_enemies() {
        for &victim in &victims {
            provocations.write(Provocation(victim));
        }
    }
    let underwater = states.single().is_ok_and(|state| state.camera_sway);
    let outcome = super::roll(&mut rng, attempt.failure, definition, attempt.quiet, underwater);
    let innate = matches!(source, Source::Innate { .. });
    let item = carried
        .get(caster)
        .ok()
        .and_then(|carried| item_use(source, carried, &inventory, &content.items));
    let report = Report {
        caster,
        spell,
        power,
        item: item.map(|item| item.definition.name.clone()),
        outcome,
        effects: Vec::new(),
    };
    // Paid whatever the roll: points for a learned spell, a use for an
    // item, nothing for an innate cast; then the caster is tired, more
    // under load (retail `FatigueCharacter`).
    match (source, item) {
        (Source::Spell { .. }, _) => {
            let realm = definition.realm.expect("a cast spell has a realm");
            points.get_mut(caster).unwrap().spend(realm, definition.cost_at(power));
        }
        (_, Some(item)) if item.from_pool => inventory.pool.consume_use(item.id, item.definition),
        (_, Some(item)) => {
            let keeps = item.definition.keeps_empty();
            carried.get_mut(caster).unwrap().consume_use(item.id, item.definition, keeps);
        }
        _ => {}
    }
    if !innate && let Ok(mut vitals) = vitals.get_mut(caster) {
        let fatigue = match item.map(|item| item.definition.use_type) {
            None => super::cast_fatigue(definition, power),
            Some(UseType::Thrown) => 10,
            Some(UseType::Instrument | UseType::Gadget) => 0,
            Some(_) => {
                let base = rng.below(2) + 1;
                if vitals.conditions.has(Condition::Diseased) { base * 2 } else { base }
            }
        };
        let slowed = vitals.conditions.has(Condition::Slowed);
        let hasted = vitals.enchantments.has(SpellId::HASTE);
        let load = burdens.get(caster).map(|burden| burden.load).unwrap_or_default();
        vitals.tire(load.scale_fatigue(fatigue, slowed, hasted));
    }
    let mut act = commands.entity(cast);
    act.remove::<Cast>().insert(attempt.strength);
    if outcome == Outcome::Fizzled {
        act.insert(Deed::new(Act::Cast(report)));
        if source == (Source::Spell { spell, power: AUTO_POWER })
            && let (Ok(character), Ok(vitals), Ok(learned), Ok(pool)) = (
                characters.get(caster),
                vitals.get(caster),
                spellbooks.get(caster),
                points.get(caster),
            )
        {
            let maximum =
                character.stats(&vitals.enchantments).max_points(learned, &content.spells);
            if vitals.stamina.current > definition.level
                && pool.available(definition.realm.unwrap(), &maximum) >= definition.cost
            {
                commands.spawn((Cast { spell, context, choice, retry: true }, By(caster)));
            }
        }
        return;
    }
    let backfired = outcome == Outcome::Backfired;
    let stand = footings.get(caster).copied().unwrap_or_default();
    let (owner, spell_of, victims) = if backfired {
        let Backfire { owner, from, aim, victims } = backfire;
        let spell_of = crate::game::hits::spells::Spell {
            id: spell,
            caster,
            backfired: true,
            reflected: false,
            owner: Some(owner),
            from,
            aim,
            victims: victims.clone(),
        };
        act.insert(crate::game::hits::spells::OwnersStrength { owner: Some(owner), spell });
        (owner, spell_of, victims)
    } else {
        let aim = match (definition.targeting, target) {
            (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
            _ => choice.aim,
        };
        let spell_of = crate::game::hits::spells::Spell {
            id: spell,
            caster,
            backfired: false,
            reflected: false,
            owner: Some(caster),
            from: stand.eye,
            aim,
            victims: victims.clone(),
        };
        (caster, spell_of, victims)
    };
    act.insert(spell_of);
    let uses = if backfired { definitions.backfire(spell) } else { definitions.spell(spell) };
    if let Some(uses) = uses {
        act.insert(Uses(uses));
        if let Some(lifesteal) = definitions.get(uses).leeches {
            act.insert(crate::game::hits::spells::Drains { caster: owner, lifesteal, drained: 0 });
        }
    }
    if context == UseContext::Combat
        && let Some(round) = round.as_mut()
    {
        round.attacked(true);
    }
    let flies = definition.missile.is_some() && !backfired && !victims.is_empty();
    if !innate && let Ok(character) = characters.get(caster) {
        let skills = practice(character, definition, power, item.map(|item| item.definition));
        act.insert(Practice { caster, skills, sure: flies });
    }
    // A missile cast is reported as it leaves; its impacts when they land.
    if flies {
        news.write(MagicNews::Done(report));
        act.insert(Deed::new(Act::Missile { caster, spell }));
    } else {
        act.insert(Deed::new(Act::Cast(report)));
    }
    if source == (Source::Spell { spell, power: AUTO_POWER })
        && definition.power == PowerChoice::ChosenOrRepeat
    {
        act.insert(Recast { caster, choice, victims: victims.clone() });
    }
    let flight = |victim: Entity| {
        if !flies {
            return None;
        }
        let to = match (definition.targeting, target) {
            (Targeting::Radius, Some(at)) => at,
            _ => footings.get(victim).map_or(stand.eye, |at| at.feet),
        };
        Some(to.distance(stand.eye) / MISSILE_SPEED)
    };
    let victims: Vec<_> = victims.into_iter().map(|victim| (victim, flight(victim))).collect();
    reach(&mut commands, cast, &victims);
}

/// Execute the source side of a utility or device cast. The exclusive caller
/// validates its target first and applies the consequence immediately.
pub(crate) fn cast_source(
    world: &mut World,
    caster: Entity,
    source: Source,
    context: UseContext,
) -> Result<(Report, Strength), Refusal> {
    world.run_system_cached_with(perform_utility, (caster, source, context)).unwrap()
}

/// A utility or device cast's source, revalidated, rolled and paid at once:
/// retail counts these as having done something even when they fizzle.
#[allow(clippy::type_complexity)]
fn perform_utility(
    In((caster, source, context)): In<(Entity, Source, UseContext)>,
    mut casters: Query<(
        &mut Character,
        &mut Vitals,
        &mut Carried,
        &LearnedSpells,
        &mut SpellPoints,
    )>,
    mut inventory: Single<&mut PartyInventory>,
    roster: Single<&PartyRoster>,
    content: Res<Catalogs>,
    difficulty: Res<Difficulty>,
    states: Query<&crate::game::world::triggers::LevelState, With<Level>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<crate::game::combat::CombatNews>,
) -> Result<(Report, Strength), Refusal> {
    if !roster.contains(&caster) {
        return Err(Refusal::CannotAct);
    }
    let (spells, items) = (&content.spells, &content.items);
    let underwater = states.single().is_ok_and(|state| state.camera_sway);
    let Ok((mut character, mut vitals, mut carried, learned, mut pool)) = casters.get_mut(caster)
    else {
        return Err(Refusal::CannotAct);
    };
    if !vitals.can_fight() {
        return Err(Refusal::CannotAct);
    }
    let spell = source.spell_id(&carried, &inventory, items).ok_or(Refusal::NoItem)?;
    let spell = &spells[spell];
    if !usable_now(spell, context) {
        return Err(Refusal::WrongTime);
    }
    let item = item_use(source, &carried, &inventory, items);
    let power = match (source, item) {
        (Source::Spell { power, .. }, _) => {
            if vitals.conditions.has(Condition::Silenced) {
                return Err(Refusal::Silenced);
            }
            let maximum = character.stats(&vitals.enchantments).max_points(learned, spells);
            if !learned.0.contains(&spell.id) {
                return Err(Refusal::NotLearned);
            }
            power.min(MAX_POWER).min(pool.affordable_power(spell, &maximum))
        }
        (Source::Innate { .. }, _) | (_, None) => return Err(Refusal::NoItem),
        (_, Some(item)) => item.definition.spell.map_or(0, |cast| cast.power),
    };
    if power == 0 {
        return Err(Refusal::NoSpellPoints);
    }
    let Attempt { failure, quiet, strength } = attempt(&character, item, spell, power, *difficulty);
    let outcome = super::roll(&mut rng, failure, spell, quiet, underwater);
    let report = Report {
        caster,
        spell: spell.id,
        power,
        item: item.map(|item| item.definition.name.clone()),
        outcome,
        effects: Vec::new(),
    };
    match item {
        None => pool.spend(spell.realm.expect("a cast spell has a realm"), spell.cost_at(power)),
        Some(item) if item.from_pool => inventory.pool.consume_use(item.id, item.definition),
        Some(item) => {
            let definition = item.definition;
            carried.consume_use(item.id, definition, definition.keeps_empty());
        }
    }
    vitals.tire(super::cast_fatigue(spell, power));
    let skills = practice(&character, spell, power, item.map(|item| item.definition));
    practise(caster, &mut character, &skills, &mut rng, &mut news);
    Ok((report, strength))
}

/// A sourced attempt at `power`: its failure chance, whether it is quiet (a
/// thrown item), and the strength it casts with, from a learned spell or an
/// item.
fn attempt(
    character: &Character,
    item: Option<ItemUse>,
    spell: &SpellDefinition,
    power: u32,
    difficulty: Difficulty,
) -> Attempt {
    match item {
        Some(item) => {
            let (failure, quiet) =
                item_failure(character, item.definition, spell, power, item.identified, difficulty);
            Attempt {
                failure,
                quiet,
                strength: item_strength(character, item.definition, spell, power),
            }
        }
        None => Attempt {
            failure: difficulty.scale_by_pace(super::fail_chance(character, spell, power), 0),
            quiet: false,
            strength: Strength::of(character, spell, power),
        },
    }
}

/// Practice from a cast that did something (retail `Magic.cpp:2682-2692`,
/// `PC Item.cpp:3792-3796`): a learned spell trains its book, its realm and
/// Power Cast; an item its use's skill.
fn practice(
    character: &Character,
    spell: &SpellDefinition,
    power: u32,
    item: Option<&ItemDefinition>,
) -> Vec<(Skill, u32)> {
    match item {
        None => {
            let index = spell.level + power;
            let mut practice = vec![(super::best_book(character, spell), (index + 2) >> 2)];
            if let Some(realm) = spell.realm {
                practice.push((realm.skill(), index));
            }
            if character.skill_available(Skill::PowerCast) {
                practice.push((Skill::PowerCast, (index + 2) >> 2));
            }
            practice
        }
        Some(definition) => definition
            .use_type
            .skill()
            .map(|skill| vec![(skill, spell.level + power / 2)])
            .unwrap_or_default(),
    }
}

/// Practise a cast's skills, announcing those that improved.
fn practise(
    caster: Entity,
    character: &mut Character,
    skills: &[(Skill, u32)],
    rng: &mut GameRng,
    news: &mut MessageWriter<crate::game::combat::CombatNews>,
) {
    for &(skill, usage) in skills {
        if character.practise(skill, usage, rng) {
            news.write(crate::game::combat::CombatNews::Improved { member: caster, skill });
        }
    }
}

/// Settle each landed cast's practice ([`Practice`]).
pub fn practise_casts(
    mut commands: Commands,
    deeds: Query<(Entity, &Deed, &Practice), Without<DeedHits>>,
    mut characters: Query<&mut Character>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<crate::game::combat::CombatNews>,
) {
    for (entity, deed, practice) in &deeds {
        commands.entity(entity).remove::<Practice>();
        if !practice.sure && !crate::game::hits::affected(&deed.effects) {
            continue;
        }
        let Ok(mut character) = characters.get_mut(practice.caster) else { continue };
        practise(practice.caster, &mut character, &practice.skills, &mut rng, &mut news);
    }
}

/// Recast each landed auto recast whose victims are not yet whole, while
/// its caster's stamina and points last.
pub fn recast(
    mut commands: Commands,
    deeds: Query<(Entity, &Recast), Without<DeedHits>>,
    bodies: Query<&Vitals>,
    casters: Query<(&Character, &Vitals, &LearnedSpells, &SpellPoints)>,
    content: If<Res<Catalogs>>,
    mut requests: MessageWriter<super::MagicRequest>,
) {
    for (entity, recast) in &deeds {
        commands.entity(entity).remove::<Recast>();
        let Source::Spell { spell, .. } = recast.choice.source else { continue };
        let spell = &content.spells[spell];
        // Every victim whole in what the spell restores, so it can stop.
        let restored = recast.victims.iter().all(|&victim| {
            let Ok(vitals) = bodies.get(victim) else { return true };
            vitals.is_dead()
                || match spell.id {
                    SpellId::STAMINA | SpellId::REST_ALL => {
                        vitals.stamina.current >= vitals.stamina.max
                    }
                    _ => vitals.health.current >= vitals.health.max,
                }
        });
        let Ok((character, vitals, learned, pool)) = casters.get(recast.caster) else { continue };
        let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
        if vitals.stamina.current > spell.level
            && pool.available(spell.realm.unwrap(), &maximum) >= spell.cost
            && !restored
        {
            requests.write(super::MagicRequest { caster: recast.caster, choice: recast.choice });
        }
    }
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    character: &Character,
    carried: &Carried,
    items: &crate::content::items::ItemCatalog,
    definitions: &Definitions,
) -> Vec<(StackId, SpellId)> {
    carried
        .stacks()
        .filter_map(|stack| {
            let worn =
                carried.active_equipment().any(|(_, equipped)| equipped.stack.id == stack.id);
            Some((stack.id, usable_stack(character, stack, items, definitions, worn)?))
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
    definitions: &Definitions,
    worn: bool,
) -> Option<SpellId> {
    let definition = &items[stack.item];
    let cast = definition.spell?;
    (can_activate(character, definition, worn)
        && stack.remaining_uses() != Some(0)
        && resolvable(cast.spell, definitions))
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
    definitions: &Definitions,
) -> bool {
    affordable > 0 && resolvable(spell.id, definitions) && usable_now(spell, context) && !silenced
}
