//! Casting a learned spell, using an item's spell, or a monster's innate
//! casting (retail `ExecuteCharacterSpellCast`, `UseItem`, `CastItemSpell`,
//! `CastSpellFromSource`, `MonsterCastsSpell`). A cast is an entity
//! ([`Cast`]) made [`By`] its caster from what it [`Chosen`] that goes
//! through the [`Casts`] stages: checks that refuse what cannot be cast,
//! aiming that reaches its victims, its power, its attempt, its roll and
//! payment, then a fizzle, a backfire or its missiles, until it is released
//! as the act that lands ([`crate::game::hits`]).
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
use crate::game::hits::effects::InFlight;
use crate::game::hits::reach::{Recipients, within};
use crate::game::hits::spells::{group_of, leader, reach};
use crate::game::hits::{Act, By, Deed, DeedHits, Effect, Strikes};
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

/// A cast under way: the spell and when it is cast. It is made [`By`] its
/// caster from what it [`Chosen`], and goes through the [`Casts`] stages
/// until it is an act ([`release`]) or is refused.
#[derive(Component, Clone, Copy, Debug)]
pub struct Cast {
    pub spell: SpellId,
    pub context: UseContext,
}

/// The source and aim the caster chose.
#[derive(Component, Clone, Copy, Debug)]
pub struct Chosen(pub MagicChoice);

/// The cast cannot be made; nothing is spent.
#[derive(Component, Clone, Copy, Debug)]
pub struct Refused(pub Refusal);

/// Where the cast is aimed: the point a blast lands on or a cone fans
/// toward; `None` for a cast aimed at bodies only.
#[derive(Component, Clone, Copy, Debug)]
pub struct Target(pub Option<Vec3>);

/// The power the cast goes off at.
#[derive(Component, Clone, Copy, Debug)]
pub struct Power(pub u32);

/// The attempt the cast is: how likely it fails, whether it is a quiet
/// (thrown) use, and the strength it goes off at.
#[derive(Component, Clone, Copy, Debug)]
pub struct Attempt {
    pub failure: u32,
    pub quiet: bool,
    pub strength: Strength,
}

/// How the cast's roll came out.
#[derive(Component, Clone, Copy, Debug)]
pub struct Rolled(pub Outcome);

/// An auto cast's next attempt after a fizzle: it provokes nobody again.
#[derive(Component, Debug)]
pub struct Retry;

/// The stages a cast goes through, run by [`run_casts`].
#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Casts;

/// Run the cast stages until every cast is settled: refused, fizzled, or
/// released as an act.
pub fn run_casts(world: &mut World) {
    let mut passes = 0;
    while world.query_filtered::<(), With<Cast>>().iter(world).next().is_some() {
        passes += 1;
        if passes > 64 {
            error!("casts left unsettled after {passes} passes");
            break;
        }
        world.run_schedule(Casts);
    }
}

pub(crate) fn stages(app: &mut App) {
    app.add_systems(
        crate::game::hits::Hits,
        (practise_casts, recast).chain().in_set(crate::game::hits::HitPhase::Completed),
    );
    app.add_systems(
        Casts,
        (
            (check_able, check_silence, check_learned, check_affordable, check_item, check_moment)
                .chain(),
            (aim_caster, aim_ally, aim_party, aim_enemy, aim_group, aim_all, aim_cone).chain(),
            (
                play_instruments,
                refuse,
                choose_power,
                aim_blast,
                provoke,
                hostility::take_offence,
                hostility::judge_sides,
            )
                .chain(),
            (judge_attempts, roll_casts, pay_points, pay_items, tire_casters).chain(),
            (fizzle, backfire, launch, release).chain(),
        )
            .chain(),
    );
}

/// The caster must be able to fight, and a sourced cast's in the party.
fn check_able(
    mut commands: Commands,
    casts: Query<(Entity, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Vitals, Has<PartyMember>)>,
) {
    for (cast, by, chosen) in &casts {
        let innate = matches!(chosen.0.source, Source::Innate { .. });
        let able =
            bodies.get(by.0).is_ok_and(|(vitals, member)| vitals.can_fight() && (innate || member));
        if !able {
            commands.entity(cast).insert(Refused(Refusal::CannotAct));
        }
    }
}

/// A silenced caster casts no spell and plays no instrument; a monster's
/// own casting was weighed against its silence as it chose.
fn check_silence(
    mut commands: Commands,
    casts: Query<(Entity, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Vitals, &Carried)>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
) {
    for (cast, by, chosen) in &casts {
        if matches!(chosen.0.source, Source::Innate { .. }) {
            continue;
        }
        let Ok((vitals, carried)) = bodies.get(by.0) else { continue };
        let item = item_use(chosen.0.source, carried, &inventory, &content.items);
        let voiced = item.is_none_or(|item| item.definition.use_type == UseType::Instrument);
        if voiced && vitals.conditions.has(Condition::Silenced) {
            commands.entity(cast).insert(Refused(Refusal::Silenced));
        }
    }
}

/// A learned spell must be learned, and cast at some power.
fn check_learned(
    mut commands: Commands,
    casts: Query<(Entity, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    learned: Query<&LearnedSpells>,
) {
    for (cast, by, chosen) in &casts {
        let Source::Spell { spell, power } = chosen.0.source else { continue };
        let refusal = if power == 0 {
            Refusal::CannotUse
        } else if !learned.get(by.0).is_ok_and(|learned| learned.0.contains(&spell)) {
            Refusal::NotLearned
        } else {
            continue;
        };
        commands.entity(cast).insert(Refused(refusal));
    }
}

/// A learned spell needs the points for its first power.
fn check_affordable(
    mut commands: Commands,
    casts: Query<(Entity, &By, &Cast, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Character, &Vitals, &LearnedSpells, &SpellPoints)>,
    content: If<Res<Catalogs>>,
) {
    for (cast, by, &Cast { spell, .. }, chosen) in &casts {
        if !matches!(chosen.0.source, Source::Spell { .. }) {
            continue;
        }
        let affordable = bodies.get(by.0).map_or(0, |(character, vitals, learned, pool)| {
            let maximum =
                character.stats(&vitals.enchantments).max_points(learned, &content.spells);
            pool.affordable_power(&content.spells[spell], &maximum)
        });
        if affordable == 0 {
            commands.entity(cast).insert(Refused(Refusal::NoSpellPoints));
        }
    }
}

/// An item must still be there, cast something, be one its user may
/// activate (worn, if it must be), and have a charge left.
fn check_item(
    mut commands: Commands,
    casts: Query<(Entity, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Character, &Carried)>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
) {
    for (cast, by, chosen) in &casts {
        let (id, from_pool) = match chosen.0.source {
            Source::Spell { .. } | Source::Innate { .. } => continue,
            Source::Item(id) => (id, false),
            Source::PoolItem(id) => (id, true),
        };
        let Ok((character, carried)) = bodies.get(by.0) else { continue };
        let stack = if from_pool {
            inventory.pool.stacks.iter().find(|s| s.id == id)
        } else {
            carried.find_stack(id)
        };
        let refusal = match stack {
            None => Some(Refusal::NoItem),
            Some(stack) => {
                let definition = &content.items[stack.item];
                let worn = !from_pool && carried.active_equipment().any(|(_, e)| e.stack.id == id);
                match definition.spell {
                    None => Some(Refusal::CannotUse),
                    Some(cast)
                        if cast.power == 0
                            && !matches!(
                                definition.use_type,
                                UseType::Instrument | UseType::Gadget
                            ) =>
                    {
                        Some(Refusal::CannotUse)
                    }
                    Some(_) if !can_activate(character, definition, worn) => {
                        Some(Refusal::CannotUse)
                    }
                    Some(_) if stack.remaining_uses() == Some(0) => Some(Refusal::NoCharges),
                    Some(_) => None,
                }
            }
        };
        if let Some(refusal) = refusal {
            commands.entity(cast).insert(Refused(refusal));
        }
    }
}

/// The spell must be one the game resolves, and a sourced cast made at a
/// time it may be; an auto recast belongs outside a fight (retail refuses
/// it in one). A monster's own casting weighed its moment as it chose.
fn check_moment(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &Chosen), (Without<Refused>, Without<Target>)>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
) {
    for (cast, &Cast { spell, context }, chosen) in &casts {
        let definition = &content.spells[spell];
        let innate = matches!(chosen.0.source, Source::Innate { .. });
        let repeats = chosen.0.source == Source::Spell { spell, power: AUTO_POWER }
            && definition.power == PowerChoice::ChosenOrRepeat;
        let refusal = if definitions.spell(spell).is_none() {
            Refusal::Unsupported
        } else if !innate
            && (!usable_now(definition, context) || (repeats && context == UseContext::Combat))
        {
            Refusal::WrongTime
        } else {
            continue;
        };
        commands.entity(cast).insert(Refused(refusal));
    }
}

/// A potion or food used in a fight is taken by its user (retail `UseItem`
/// aims potions at their user).
fn drunk(
    chosen: &Chosen,
    carried: &Carried,
    inventory: &PartyInventory,
    items: &ItemCatalog,
) -> bool {
    item_use(chosen.0.source, carried, inventory, items)
        .is_some_and(|item| consumed_by_anyone(item.definition))
}

/// The casts whose spell has `targeting`, still to be aimed.
fn aiming<'a>(
    casts: &'a Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    spells: &'a crate::content::spells::SpellCatalog,
    targeting: Targeting,
) -> impl Iterator<Item = (Entity, Entity, &'a Chosen, &'a SpellDefinition)> {
    casts
        .iter()
        .filter(move |(_, cast, ..)| spells[cast.spell].targeting == targeting)
        .map(|(entity, cast, by, chosen)| (entity, by.0, chosen, &spells[cast.spell]))
}

/// A spell on its caster lands on them; so does anything drunk in a fight.
fn aim_caster(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    carried: Query<&Carried>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
) {
    for (cast, &Cast { spell, context }, by, chosen) in &casts {
        let forced = context == UseContext::Combat
            && carried
                .get(by.0)
                .is_ok_and(|carried| drunk(chosen, carried, &inventory, &content.items));
        if forced || content.spells[spell].targeting == Targeting::Caster {
            reach(&mut commands, cast, &[(by.0, None)]);
            commands.entity(cast).insert(Target(None));
        }
    }
}

/// An ally spell lands on the friend aimed at, if it is on the caster's
/// side and the spell reaches it.
fn aim_ally(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Footing, &Side, &Loyalty)>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    for (cast, caster, chosen, spell) in aiming(&casts, &content.spells, Targeting::Ally) {
        let friend = chosen.0.aim.body().and_then(|aimed| {
            let (from, side, loyalty) = bodies.get(caster).ok()?;
            let (to, their_side, their_loyalty) = bodies.get(aimed).ok()?;
            let friendly = aimed == caster
                || hostility::between((side.0, *loyalty), (their_side.0, *their_loyalty))
                    == Stance::Friendly;
            (friendly && reaches(from, to, spell.reach.metres(), &obstacles)).then_some(aimed)
        });
        match friend {
            Some(friend) => {
                reach(&mut commands, cast, &[(friend, None)]);
                commands.entity(cast).insert(Target(None));
            }
            None => {
                commands.entity(cast).insert(Refused(Refusal::NoTarget));
            }
        }
    }
}

/// A party spell lands on every member, the dead included.
fn aim_party(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    roster: Single<&PartyRoster>,
    content: If<Res<Catalogs>>,
) {
    for (cast, ..) in aiming(&casts, &content.spells, Targeting::Party) {
        let victims: Vec<_> = roster.iter().map(|member| (member, None)).collect();
        reach(&mut commands, cast, &victims);
        commands.entity(cast).insert(Target(None));
    }
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

/// An enemy spell lands on the foe aimed at; a blast is aimed at where it
/// stands, its victims taken once its power is known ([`aim_blast`]).
fn aim_enemy(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    footings: Query<&Footing>,
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    for (cast, &Cast { spell, .. }, by, chosen) in &casts {
        let spell = &content.spells[spell];
        if !matches!(spell.targeting, Targeting::Enemy | Targeting::Radius) {
            continue;
        }
        let foes =
            foes_reached(by.0, spell.reach.metres(), &roster, combatants, &bodies, &obstacles);
        let target = footings
            .get(by.0)
            .ok()
            .and_then(|from| aimed_foe(chosen.0.aim, &foes, from, &footings));
        match target {
            Some(target) => {
                if spell.targeting == Targeting::Enemy {
                    reach(&mut commands, cast, &[(target, None)]);
                }
                let at = footings.get(target).map_or(Vec3::ZERO, |at| at.feet);
                commands.entity(cast).insert(Target(Some(at)));
            }
            None => {
                commands.entity(cast).insert(Refused(Refusal::NoTarget));
            }
        }
    }
}

/// A group spell lands on the foe aimed at and those of its group the
/// spell reaches.
fn aim_group(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    footings: Query<&Footing>,
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    memberships: Query<&GroupMember>,
    groups: Query<&GroupMembers>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    for (cast, caster, chosen, spell) in aiming(&casts, &content.spells, Targeting::EnemyGroup) {
        let foes =
            foes_reached(caster, spell.reach.metres(), &roster, combatants, &bodies, &obstacles);
        let Some(first) = footings
            .get(caster)
            .ok()
            .and_then(|from| aimed_foe(chosen.0.aim, &foes, from, &footings))
        else {
            commands.entity(cast).insert(Refused(Refusal::NoTarget));
            continue;
        };
        let mut victims = vec![(first, None)];
        for mate in group_of(first, &roster, &memberships, &groups) {
            if mate != first && foes.contains(&mate) {
                victims.push((mate, None));
            }
        }
        reach(&mut commands, cast, &victims);
        let at = footings.get(first).map_or(Vec3::ZERO, |at| at.feet);
        commands.entity(cast).insert(Target(Some(at)));
    }
}

/// An all-enemies spell lands on every foe it reaches.
fn aim_all(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    for (cast, caster, _, spell) in aiming(&casts, &content.spells, Targeting::AllEnemies) {
        let victims: Vec<_> =
            foes_reached(caster, spell.reach.metres(), &roster, combatants, &bodies, &obstacles)
                .into_iter()
                .map(|foe| (foe, None))
                .collect();
        if victims.is_empty() {
            commands.entity(cast).insert(Refused(Refusal::NoTarget));
            continue;
        }
        reach(&mut commands, cast, &victims);
        commands.entity(cast).insert(Target(None));
    }
}

/// A cone fans out from the caster toward the foe aimed at, over everyone
/// else within its reach and arcs, whatever their side (retail
/// `PopulateSpellTargetMarkers`).
fn aim_cone(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (Without<Refused>, Without<Target>)>,
    bodies: Query<(&Vitals, &Footing, &Side, &Loyalty), Without<Untargetable>>,
    footings: Query<&Footing>,
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    for (cast, caster, chosen, spell) in aiming(&casts, &content.spells, Targeting::Cone) {
        let reach_of = spell.reach.metres();
        let foes = foes_reached(caster, reach_of, &roster, combatants, &bodies, &obstacles);
        let Ok(from) = footings.get(caster) else { continue };
        let Some(target) = aimed_foe(chosen.0.aim, &foes, from, &footings) else {
            commands.entity(cast).insert(Refused(Refusal::NoTarget));
            continue;
        };
        let toward = footings.get(target).map_or(from.eye, |at| at.feet) - from.eye;
        let victims: Vec<_> = crate::game::combat::participants(&roster, combatants)
            .filter(|&other| other != caster)
            .filter(|&other| {
                bodies.get(other).is_ok_and(|(vitals, at, ..)| {
                    !vitals.is_dead()
                        && reaches(from, at, reach_of, &obstacles)
                        && in_cone(toward, at.feet - from.eye)
                })
            })
            .map(|victim| (victim, None))
            .collect();
        reach(&mut commands, cast, &victims);
        commands.entity(cast).insert(Target(Some(from.eye + toward)));
    }
}

/// An instrument or gadget finds the power its player can manage, tiring
/// them whether or not they can (retail `UseItem`); played badly, nothing
/// else is spent, and the attempt is practice.
fn play_instruments(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (With<Target>, Without<Power>, Without<Refused>)>,
    mut bodies: Query<(&mut Character, &mut Vitals, &Carried)>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (cast, &Cast { spell, .. }, by, chosen) in &casts {
        let Ok((mut profile, mut vitals, carried)) = bodies.get_mut(by.0) else { continue };
        let Some(item) = item_use(chosen.0.source, carried, &inventory, &content.items) else {
            continue;
        };
        let definition = item.definition;
        if !matches!(definition.use_type, UseType::Instrument | UseType::Gadget) {
            continue;
        }
        let spell = &content.spells[spell];
        let mut power = MAX_POWER;
        while power > 0
            && item_failure(&profile, definition, spell, power, true, Difficulty::Normal).0 > 15
        {
            power -= 1;
        }
        while power > 0 && power * 7 > rng.below(100) {
            power -= 1;
        }
        let improved = (power == 0)
            .then(|| {
                definition.use_type.skill().filter(|&skill| profile.practise(skill, 1, &mut rng))
            })
            .flatten();
        vitals.tire(spell.level * 8 + 20);
        if power == 0 {
            commands.entity(cast).insert(Refused(Refusal::PlayedBadly { improved }));
        } else {
            commands.entity(cast).insert(Power(power));
        }
    }
}

/// Refused casts go, spending nothing, with a word to the player.
fn refuse(
    mut commands: Commands,
    casts: Query<(Entity, &Refused, &By)>,
    mut news: MessageWriter<MagicNews>,
) {
    for (cast, refused, by) in &casts {
        news.write(MagicNews::Refused { caster: by.0, refusal: refused.0 });
        commands.entity(cast).despawn();
    }
}

/// The power a cast goes off at: an item's own, an innate cast's chosen
/// one; a learned spell's chosen power, or for an auto cast what its victim
/// needs, within what the caster can afford now.
fn choose_power(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen), (With<Target>, Without<Power>)>,
    casters: Query<(&Character, &Vitals, &LearnedSpells, &SpellPoints, &Carried)>,
    victims: Query<&Vitals>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
) {
    for (cast, &Cast { spell, .. }, by, chosen) in &casts {
        let spell = &content.spells[spell];
        let power = match chosen.0.source {
            Source::Innate { power, .. } => power,
            Source::Spell { power, .. } => {
                let Ok((character, vitals, learned, pool, _)) = casters.get(by.0) else { continue };
                let maximum =
                    character.stats(&vitals.enchantments).max_points(learned, &content.spells);
                let affordable = pool.affordable_power(spell, &maximum);
                if power != AUTO_POWER {
                    power.min(affordable)
                } else {
                    let restores = definitions.spell(spell.id).is_some_and(|id| {
                        definitions
                            .get(id)
                            .effects
                            .iter()
                            .any(|e| matches!(e.kind, EffectKind::Heal | EffectKind::Refresh))
                    });
                    let victim = chosen.0.aim.body().and_then(|body| victims.get(body).ok());
                    auto_power(character, spell, victim, affordable, restores)
                }
            }
            Source::Item(_) | Source::PoolItem(_) => {
                let Ok((.., carried)) = casters.get(by.0) else { continue };
                let Some(item) = item_use(chosen.0.source, carried, &inventory, &content.items)
                else {
                    continue;
                };
                item.definition.spell.map_or(0, |cast| cast.power)
            }
        };
        commands.entity(cast).insert(Power(power));
    }
}

/// A blast takes whoever stands within its radius of where it lands, but
/// those on its caster's side.
fn aim_blast(
    mut commands: Commands,
    casts: Query<
        (Entity, &Cast, &By, &Target, &Power),
        (With<Chosen>, Without<Rolled>, Without<DeedHits>),
    >,
    bodies: Query<(Entity, &Vitals, &Side, &Footing), Without<Untargetable>>,
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    for (cast, &Cast { spell, .. }, by, target, power) in &casts {
        let spell = &content.spells[spell];
        let (Targeting::Radius, Some(centre)) = (spell.targeting, target.0) else { continue };
        let side = bodies.get(by.0).map_or(Stance::Friendly, |(_, _, side, _)| side.0);
        let radius = spell.radius + spell.radius_per_power * power.0 as f32;
        let victims: Vec<_> = within(
            centre,
            centre,
            radius,
            Recipients::Opponents(side),
            &bodies,
            (&roster, combatants),
            &obstacles,
        )
        .into_iter()
        .map(|victim| (victim, None))
        .collect();
        reach(&mut commands, cast, &victims);
    }
}

/// A party member's spell aimed at enemies provokes those it is cast at,
/// whatever the roll (retail `MakeTargetGroupHostile` as the action
/// starts); a retry provokes nobody again.
fn provoke(
    casts: Query<(&Cast, &By, &DeedHits), (With<Power>, Without<Rolled>, Without<Retry>)>,
    members: Query<(), With<PartyMember>>,
    reaches: Query<&Strikes>,
    content: If<Res<Catalogs>>,
    mut provocations: MessageWriter<Provocation>,
) {
    for (&Cast { spell, .. }, by, hits) in &casts {
        if !members.contains(by.0) || !content.spells[spell].aimed_at_enemies() {
            continue;
        }
        for strikes in reaches.iter_many(hits.iter()).matched() {
            provocations.write(Provocation(strikes.0));
        }
    }
}

/// What each cast is as an attempt (retail `ExecuteCharacterSpellCast`,
/// `CastItemSpell`, `MonsterCastsSpell`): a learned spell by its caster's
/// skill and level, an item by its user and itself, an innate cast by the
/// monster's budget and level.
fn judge_attempts(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen, &Power), (Without<Attempt>, Without<Refused>)>,
    casters: Query<(&Character, &Carried)>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    difficulty: Res<Difficulty>,
) {
    for (cast, &Cast { spell, .. }, by, chosen, power) in &casts {
        let spell = &content.spells[spell];
        let attempt = match chosen.0.source {
            Source::Innate { budget, level, .. } => Attempt {
                failure: super::spell_failure(budget, spell, power.0),
                quiet: false,
                strength: Strength::at(u32::from(level), spell, power.0, 0),
            },
            source => {
                let Ok((character, carried)) = casters.get(by.0) else { continue };
                let item = item_use(source, carried, &inventory, &content.items);
                attempt(character, item, spell, power.0, *difficulty)
            }
        };
        commands.entity(cast).insert(attempt);
    }
}

/// Each attempt is rolled (retail `CastSpellFromSource`), and the cast's
/// record begun; a sourced cast's practice is owed once it has landed.
fn roll_casts(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen, &Power, &Attempt), Without<Rolled>>,
    casters: Query<(&Character, &Carried)>,
    inventory: Single<&PartyInventory>,
    states: Query<&crate::game::world::triggers::LevelState, With<Level>>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    let underwater = states.single().is_ok_and(|state| state.camera_sway);
    for (cast, &Cast { spell, .. }, by, chosen, power, attempt) in &casts {
        let spell = &content.spells[spell];
        let outcome = super::roll(&mut rng, attempt.failure, spell, attempt.quiet, underwater);
        let item = casters.get(by.0).ok().and_then(|(_, carried)| {
            item_use(chosen.0.source, carried, &inventory, &content.items)
        });
        let report = Report {
            caster: by.0,
            spell: spell.id,
            power: power.0,
            item: item.map(|item| item.definition.name.clone()),
            outcome,
            effects: Vec::new(),
        };
        let mut cast = commands.entity(cast);
        cast.insert((Rolled(outcome), attempt.strength, Deed::new(Act::Cast(report))));
        if !matches!(chosen.0.source, Source::Innate { .. })
            && let Ok((character, _)) = casters.get(by.0)
        {
            let skills = practice(character, spell, power.0, item.map(|item| item.definition));
            cast.insert(Practice { caster: by.0, skills, sure: false });
        }
    }
}

/// Each sourced attempt tires its caster, more under load (retail
/// `FatigueCharacter`): a learned spell by its level and power, a thrown
/// item ten, a drink a little; an instrument or gadget already paid.
fn tire_casters(
    casts: Query<(&Cast, &By, &Chosen, &Power), Added<Rolled>>,
    mut casters: Query<(&mut Vitals, &Carried, &crate::game::inventory::Burden)>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
) {
    for (&Cast { spell, .. }, by, chosen, power) in &casts {
        if matches!(chosen.0.source, Source::Innate { .. }) {
            continue;
        }
        let Ok((mut vitals, carried, burden)) = casters.get_mut(by.0) else { continue };
        let fatigue = match item_use(chosen.0.source, carried, &inventory, &content.items) {
            None => super::cast_fatigue(&content.spells[spell], power.0),
            Some(item) => match item.definition.use_type {
                UseType::Thrown => 10,
                UseType::Instrument | UseType::Gadget => 0,
                _ => {
                    let base = rng.below(2) + 1;
                    if vitals.conditions.has(Condition::Diseased) { base * 2 } else { base }
                }
            },
        };
        let slowed = vitals.conditions.has(Condition::Slowed);
        let hasted = vitals.enchantments.has(SpellId::HASTE);
        vitals.tire(burden.load.scale_fatigue(fatigue, slowed, hasted));
    }
}

/// A learned spell's attempt costs its points, whatever the roll.
fn pay_points(
    casts: Query<(&Cast, &By, &Chosen, &Power), Added<Rolled>>,
    mut points: Query<&mut SpellPoints>,
    content: If<Res<Catalogs>>,
) {
    for (&Cast { spell, .. }, by, chosen, power) in &casts {
        if !matches!(chosen.0.source, Source::Spell { .. }) {
            continue;
        }
        let spell = &content.spells[spell];
        let realm = spell.realm.expect("a cast spell has a realm");
        points.get_mut(by.0).unwrap().spend(realm, spell.cost_at(power.0));
    }
}

/// An item's use costs one use, whatever the roll; the last potion still
/// lands.
fn pay_items(
    casts: Query<(&By, &Chosen), Added<Rolled>>,
    mut carried: Query<&mut Carried>,
    mut inventory: Single<&mut PartyInventory>,
    content: If<Res<Catalogs>>,
) {
    for (by, chosen) in &casts {
        let Ok(mut carried) = carried.get_mut(by.0) else { continue };
        let Some(item) = item_use(chosen.0.source, &carried, &inventory, &content.items) else {
            continue;
        };
        let definition = item.definition;
        if item.from_pool {
            inventory.pool.consume_use(item.id, definition);
        } else {
            carried.consume_use(item.id, definition, definition.keeps_empty());
        }
    }
}

/// A fizzled cast does nothing; its record stays to be reported. An auto
/// cast tries again while stamina and points last.
fn fizzle(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen, &Rolled, Option<&DeedHits>)>,
    casters: Query<(&Character, &Vitals, &LearnedSpells, &SpellPoints)>,
    content: If<Res<Catalogs>>,
) {
    for (cast, &cast_of, by, chosen, rolled, hits) in &casts {
        if rolled.0 != Outcome::Fizzled {
            continue;
        }
        for reach in hits.into_iter().flat_map(|hits| hits.iter()) {
            commands.entity(reach).despawn();
        }
        settle(&mut commands, cast);
        commands.entity(cast).remove::<Practice>();
        if !matches!(chosen.0.source, Source::Spell { power: AUTO_POWER, .. }) {
            continue;
        }
        let spell = &content.spells[cast_of.spell];
        let Ok((character, vitals, learned, pool)) = casters.get(by.0) else { continue };
        let maximum = character.stats(&vitals.enchantments).max_points(learned, &content.spells);
        if vitals.stamina.current > spell.level
            && pool.available(spell.realm.unwrap(), &maximum) >= spell.cost
        {
            commands.spawn((cast_of, *chosen, By(by.0), Retry));
        }
    }
}

/// The cast is settled: it stays only as its record.
fn settle(commands: &mut Commands, cast: Entity) {
    commands.entity(cast).remove::<(Cast, Chosen, By, Target, Power, Attempt, Rolled, Retry)>();
}

/// A backfire turns on the caster's side (retail `PrepareSpellTarget`): a
/// spell aimed at one body on the caster, a wider one on everyone of the
/// caster's side in the fight within its reach, a blast on those within
/// its radius of the caster. It comes from whom it was aimed at, or where
/// it was to land, and is as strong as they are, settled as it lands.
#[allow(clippy::type_complexity)]
fn backfire(
    mut commands: Commands,
    casts: Query<(Entity, &Cast, &By, &Chosen, &Rolled, &Target, &Power, Option<&DeedHits>)>,
    bodies: Query<(Entity, &Vitals, &Side, &Footing), Without<Untargetable>>,
    memberships: Query<&GroupMember>,
    groups: Query<&GroupMembers>,
    fight: Option<Single<&crate::game::combat::Combatants>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
) {
    let combatants = fight.as_deref().map_or(&[][..], |fight| fight.entities());
    for (cast, &Cast { spell, .. }, by, chosen, rolled, target, power, hits) in &casts {
        if rolled.0 != Outcome::Backfired {
            continue;
        }
        for reach in hits.into_iter().flat_map(|hits| hits.iter()) {
            commands.entity(reach).despawn();
        }
        let spell = &content.spells[spell];
        let caster = by.0;
        let Ok((_, _, side, stand)) = bodies.get(caster) else { continue };
        let (side, stand) = (side.0, *stand);
        let eye = stand.eye;
        let eye_of = |body: Entity| bodies.get(body).map_or(eye, |(.., at)| at.eye);
        let (owner, from) = match (spell.targeting, chosen.0.aim) {
            (Targeting::Enemy | Targeting::Ally, Aim::Body(entity)) => (entity, eye_of(entity)),
            (Targeting::EnemyGroup, Aim::Body(entity)) => {
                let leader = leader(entity, combatants, &memberships, &groups);
                (leader, eye_of(leader))
            }
            (Targeting::Radius | Targeting::Cone, _) => (caster, target.0.unwrap_or(eye)),
            _ => (caster, eye),
        };
        let victims: Vec<Entity> = match spell.targeting {
            Targeting::Caster | Targeting::Ally | Targeting::Enemy => vec![caster],
            Targeting::Radius => within(
                stand.feet,
                stand.feet,
                spell.radius + spell.radius_per_power * power.0 as f32,
                Recipients::Opponents(side).reversed(),
                &bodies,
                (&roster, combatants),
                &obstacles,
            ),
            _ => crate::game::combat::participants(&roster, combatants)
                .filter(|&other| {
                    bodies.get(other).is_ok_and(|(_, vitals, their_side, at)| {
                        !vitals.is_dead()
                            && their_side.0 == side
                            && (other == caster
                                || reaches(&stand, at, spell.reach.metres(), &obstacles))
                    })
                })
                .collect(),
        };
        let aim = match (spell.targeting, target.0) {
            (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
            _ => chosen.0.aim,
        };
        commands.entity(cast).insert((
            crate::game::hits::spells::Spell {
                id: spell.id,
                caster,
                backfired: true,
                reflected: false,
                owner: Some(owner),
                from,
                aim,
                victims: victims.clone(),
            },
            crate::game::hits::spells::OwnersStrength { owner: Some(owner), spell: spell.id },
        ));
        let victims: Vec<_> = victims.into_iter().map(|victim| (victim, None)).collect();
        reach(&mut commands, cast, &victims);
    }
}

/// A missile spell that went off flies to each victim, or to where a blast
/// lands; it lands when it arrives.
fn launch(
    mut commands: Commands,
    casts: Query<(&Cast, &By, &Rolled, &Target, &DeedHits)>,
    reaches: Query<(Entity, &Strikes), With<crate::game::hits::HitOf>>,
    footings: Query<&Footing>,
    content: If<Res<Catalogs>>,
) {
    for (&Cast { spell, .. }, by, rolled, target, hits) in &casts {
        let spell = &content.spells[spell];
        if rolled.0 != Outcome::Cast || spell.missile.is_none() {
            continue;
        }
        let Ok(from) = footings.get(by.0) else { continue };
        for (reach, strikes) in reaches.iter_many(hits.iter()).matched() {
            let to = match (spell.targeting, target.0) {
                (Targeting::Radius, Some(at)) => at,
                _ => footings.get(strikes.0).map_or(from.eye, |at| at.feet),
            };
            let seconds = to.distance(from.eye) / MISSILE_SPEED;
            commands.entity(reach).insert(InFlight(Timer::from_seconds(seconds, TimerMode::Once)));
        }
    }
}

/// A cast that went off becomes its act: what it does ([`Uses`]), its
/// spell for what follows its landing, a drain's share for its owner, and,
/// for an auto recast, the next cast once it has landed. A missile cast is
/// reported as it leaves; its impacts when they land. In a fight it counts
/// as an attack.
#[allow(clippy::type_complexity)]
fn release(
    mut commands: Commands,
    mut casts: Query<(
        Entity,
        (&Cast, &By, &Rolled, &Target, &Chosen),
        Option<&crate::game::hits::spells::Spell>,
        (Option<&DeedHits>, &mut Deed, Option<&mut Practice>),
    )>,
    reaches: Query<&Strikes, With<crate::game::hits::HitOf>>,
    footings: Query<&Footing>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut round: Option<Single<&mut crate::game::combat::Round>>,
    mut news: MessageWriter<MagicNews>,
) {
    for (cast, (cast_of, by, rolled, target, chosen), spell_of, (hits, mut deed, practice)) in
        &mut casts
    {
        let Cast { spell, context } = *cast_of;
        let spell = &content.spells[spell];
        let caster = by.0;
        let backfired = rolled.0 == Outcome::Backfired;
        let uses =
            if backfired { definitions.backfire(spell.id) } else { definitions.spell(spell.id) };
        let victims: Vec<Entity> = reaches
            .iter_many(hits.into_iter().flat_map(|hits| hits.iter()))
            .matched()
            .map(|s| s.0)
            .collect();
        let owner = spell_of.and_then(|spell| spell.owner).unwrap_or(caster);
        if spell_of.is_none() {
            let aim = match (spell.targeting, target.0) {
                (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
                _ => chosen.0.aim,
            };
            commands.entity(cast).insert(crate::game::hits::spells::Spell {
                id: spell.id,
                caster,
                backfired,
                reflected: false,
                owner: Some(owner),
                from: footings.get(caster).map_or(Vec3::ZERO, |at| at.eye),
                aim,
                victims: victims.clone(),
            });
        }
        if let Some(uses) = uses {
            commands.entity(cast).insert(Uses(uses));
            if let Some(lifesteal) = definitions.get(uses).leeches {
                commands.entity(cast).insert(crate::game::hits::spells::Drains {
                    caster: owner,
                    lifesteal,
                    drained: 0,
                });
            }
        }
        if context == UseContext::Combat
            && let Some(round) = round.as_mut()
        {
            round.attacked(true);
        }
        let flies = spell.missile.is_some() && !backfired && !victims.is_empty();
        if let Some(mut practice) = practice {
            practice.sure = flies;
        }
        if flies && let Act::Cast(report) = &deed.act {
            news.write(MagicNews::Done(report.clone()));
            deed.act = Act::Missile { caster, spell: spell.id };
        }
        if chosen.0.source == (Source::Spell { spell: spell.id, power: AUTO_POWER })
            && spell.power == PowerChoice::ChosenOrRepeat
        {
            commands.entity(cast).insert(Recast { caster, choice: chosen.0, victims });
        }
        settle(&mut commands, cast);
    }
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
