//! Locks and traps on devices, and the short-lived interaction in which the
//! party works on one or looks through a container (decomp
//! `MainGameScreen.cpp` lock and trap panels, `Traps.cpp`).
//!
//! The device owns what is settled: whether its lock or trap still stands,
//! the trap's type once rolled, and its contents. The [`DeviceInteraction`]
//! owns the attempt in progress: who holds which tumbler, which trap
//! components are disarmed and what each member's inspection made of them.
//! Leaving the interaction forgets those, as retail's panels do. Each
//! attempt names its actor, validates them and commits its costs and
//! consequences at once; retail's progress bars and tumbler animations are
//! presentation and do not delay the outcome.
//!
//! Knock-Knock, Divine Trap and tools share source validation and payment. Traps
//! whose missile, poison, hearing or summoning prerequisites remain unported
//! are refused before spending RNG or changing the device.

use bevy::prelude::*;

use super::triggers::{Device, DoorState};
use crate::content::characters::{Attribute, Skill};
use crate::content::level::{Lock, TRAP_TYPES};
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Vitals};
use crate::game::difficulty::Difficulty;
use crate::game::inventory::StackId;
use crate::game::party::roster::PartyMember;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

/// The parts a trap is built from; each type uses a few of them.
pub const COMPONENTS: usize = 8;

/// Each trap type's own grade (retail `g_trap_difficulty`).
pub(crate) const TRAP_GRADE: [i32; TRAP_TYPES] = [1, 1, 2, 2, 3, 3, 3, 4, 5, 5, 5, 6, 6, 7, 7];

/// Which components each trap type is built from (retail `g_table1`).
pub const TRAP_COMPONENTS: [[bool; COMPONENTS]; TRAP_TYPES] = {
    const T: bool = true;
    const F: bool = false;
    [
        [F, T, F, F, F, T, F, F],
        [F, T, F, T, F, F, F, F],
        [F, F, T, F, F, T, F, F],
        [F, F, F, F, T, T, F, F],
        [F, F, T, T, F, F, F, T],
        [T, T, F, F, T, F, F, F],
        [F, F, T, F, T, F, T, F],
        [T, F, F, T, F, T, F, F],
        [F, F, T, T, T, F, F, F],
        [T, F, F, F, F, T, F, T],
        [F, T, T, F, F, F, F, T],
        [F, F, T, T, F, F, T, F],
        [F, T, F, F, T, F, T, F],
        [F, T, F, F, F, T, T, F],
        [T, F, F, F, F, T, T, F],
    ]
};

/// The skill a trap of each authored grade asks for (retail `g_table2`).
const GRADE_SKILL: [i32; 11] = [10, 25, 35, 40, 50, 60, 70, 80, 90, 100, 110];

/// The party remarks on a trap again once this much game time has passed
/// since it last faced it (retail: 0x3840 seconds).
const REMARK_AGAIN_MS: u64 = 4 * 3_600_000;

/// Forcing a lock costs this much stamina and needs at least as much
/// (retail `FatigueCharacter(slot, 0x50)`).
const FORCE_STAMINA: u32 = 80;
/// Strength above this adds to the chance to force a lock.
const FORCE_STRENGTH: u32 = 50;

/// The lock or trap still standing on a device; removed once it is picked,
/// forced or disarmed. Its authored kind and grade stay in the definition.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Guarded {
    /// A trap's type, authored or rolled from its grade when first faced
    /// (retail `SelectTrapType`), and then kept.
    pub trap: Option<u8>,
    /// Game time the party last faced it.
    pub faced_at: Option<u64>,
    /// Remaining practice opportunities; leaving the panel does not reset them.
    pub practice_left: u8,
}

impl Guarded {
    pub fn new(lock: Lock) -> Self {
        match lock {
            Lock::Trap { kind, .. } => Guarded { trap: kind, ..default() },
            Lock::Pickable { difficulty } => {
                Guarded { practice_left: difficulty.clamp(2, 8) * 3, ..default() }
            }
        }
    }
}

/// The interaction under way with one device. While it lasts the party
/// stays put; it ends when the party leaves it, the device opens or a
/// fight begins.
#[derive(Resource, Debug)]
pub struct DeviceInteraction {
    pub device: Entity,
    pub work: Work,
}

#[derive(Debug)]
pub enum Work {
    Picking(Picking),
    Disarming(Disarming),
    /// Looking through a container's contents (retail's item picker).
    Looting,
}

/// A pin has one owner and one hold policy, never parallel state arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tumbler {
    Down,
    Held(Entity),
    Sealed(Entity),
}
impl Tumbler {
    pub fn raised(self) -> bool {
        self != Self::Down
    }
}

/// A lock being picked: each tumbler stays up while the member who set it
/// keeps working. A different member's first pick drops them.
#[derive(Debug, Clone, PartialEq)]
pub struct Picking {
    pub difficulty: u8,
    pub tumblers: Vec<Tumbler>,
    picker: Option<Entity>,
}

impl Picking {
    fn new(difficulty: u8) -> Self {
        let tumblers = usize::from(difficulty).clamp(2, 8);
        Picking { difficulty, tumblers: vec![Tumbler::Down; tumblers], picker: None }
    }
}

/// A trap being disarmed.
#[derive(Debug, Clone, PartialEq)]
pub struct Disarming {
    pub difficulty: u8,
    pub trap: u8,
    pub disarmed: [bool; COMPONENTS],
    pub divine_power: u8,
    /// What each member's inspection made of each component.
    pub observations: Vec<(Entity, [Observation; COMPONENTS])>,
}

impl Disarming {
    pub fn inspection_chance(&self, skill: Option<u32>, difficulty: Difficulty) -> u32 {
        trap_chance(
            skill.map(|s| s + u32::from(self.divine_power) * 6),
            self.difficulty,
            difficulty,
        ) as u32
    }
    pub fn disarm_chance(&self, skill: Option<u32>, difficulty: Difficulty) -> u32 {
        let chance = trap_chance(
            skill.map(|s| s + u32::from(self.divine_power.div_ceil(2)) * 6),
            self.difficulty,
            difficulty,
        ) as u32;
        chance * chance / 100
    }

    pub fn observed(&self, member: Entity) -> Option<&[Observation; COMPONENTS]> {
        self.observations.iter().find(|(m, _)| *m == member).map(|(_, seen)| seen)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Absent,
    Present,
    Unknown,
}

/// What came of facing or working on a device, for presentation.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum LockNews {
    /// The device is locked: the lock panel opens.
    Locked,
    /// A trap stands in the way; `remark` when the party comments on it.
    Trapped {
        remark: bool,
    },
    /// Neither while fighting.
    NotInCombat,
    /// A tumbler went up; `dropped` when others fell meanwhile.
    Tumbler {
        dropped: bool,
    },
    Picked,
    Forced {
        member: Entity,
    },
    NotForced {
        member: Entity,
    },
    Inspected {
        member: Entity,
    },
    ComponentDisarmed {
        member: Entity,
        component: usize,
    },
    Disarmed {
        trap: u8,
    },
    /// The device stays guarded because discharge cannot yet be resolved.
    UnsupportedTrap,
    SkillImproved {
        member: Entity,
    },
    Sprung {
        trap: u8,
        destroyed: bool,
    },
}

/// Why an attempt was not made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unable {
    /// No such interaction, or not this kind of work.
    NotNow,
    NotMember,
    CannotAct,
    /// The member lacks Locks & Traps, or cannot act.
    Unskilled,
    /// Too weak or too tired to force the lock.
    TooWeak,
    /// That tumbler or component is already done.
    Done,
    UnsupportedTrap,
    MissingContent,
}

/// A member's Locks & Traps level, if they can use it now (retail
/// `GetPartySlotLocksTrapsLevel`).
pub fn lock_skill(character: &Character, vitals: &Vitals) -> Option<u32> {
    let level = character.skill(Skill::LocksAndTraps);
    (vitals.can_act() && (character.skill_available(Skill::LocksAndTraps) || level > 0))
        .then_some(level)
}

/// A member's chance in 100 to inspect a trap, or to disarm one of its
/// components before squaring (retail `GetTrapInteractionChance`).
pub fn trap_chance(skill: Option<u32>, difficulty: u8, setting: Difficulty) -> i32 {
    let skill = skill.map_or(-1, |s| s as i32) - GRADE_SKILL[usize::from(difficulty.min(10))];
    (skill * 3 / 2 + (11 - setting as i32) * 10).clamp(0, 99)
}

/// A member's chance in 100 to force a lock with this many tumblers, if
/// they may try (retail `W8LockInteraction::AttemptForce`).
pub fn force_chance(
    character: &Character,
    vitals: &Vitals,
    tumblers: usize,
    setting: Difficulty,
) -> Option<u32> {
    let strength = character.attribute(Attribute::Strength);
    if !vitals.can_act() || vitals.stamina < FORCE_STAMINA || strength <= FORCE_STRENGTH {
        return None;
    }
    let divisor = (setting as i32 - 1 + tumblers as i32).clamp(2, 8);
    Some((strength - FORCE_STRENGTH) >> (divisor - 2))
}

/// A device's lock or trap stops its use: start working on it (retail
/// `OpenLockInteraction`, `OpenTrapInteraction`). Facing a trap rolls its
/// type the first time.
pub(crate) fn confront(world: &mut World, device: Entity, lock: Lock) {
    if world.contains_resource::<DeviceInteraction>() {
        return;
    }
    if crate::game::combat::in_battle(world) {
        world.write_message(LockNews::NotInCombat);
        return;
    }
    let work = match lock {
        Lock::Pickable { difficulty } => {
            world.write_message(LockNews::Locked);
            Work::Picking(Picking::new(difficulty))
        }
        Lock::Trap { difficulty, .. } => {
            let now = world.resource::<GameClock>().ms;
            let mut guarded = world.get_mut::<Guarded>(device).unwrap();
            let remark = guarded.faced_at.is_none_or(|at| now.saturating_sub(at) > REMARK_AGAIN_MS);
            guarded.faced_at = Some(now);
            let trap = match guarded.trap {
                Some(trap) => trap,
                None => {
                    let trap = world
                        .resource_scope(|_, mut rng: Mut<GameRng>| roll_trap(difficulty, &mut rng));
                    world.get_mut::<Guarded>(device).unwrap().trap = Some(trap);
                    trap
                }
            };
            world.write_message(LockNews::Trapped { remark });
            Work::Disarming(Disarming {
                difficulty,
                trap,
                disarmed: [false; COMPONENTS],
                divine_power: 0,
                observations: vec![],
            })
        }
    };
    world.insert_resource(DeviceInteraction { device, work });
}

/// Retail `SelectTrapType`: draw types until one's grade lies within four
/// below the authored grade (at least 1).
fn roll_trap(difficulty: u8, rng: &mut GameRng) -> u8 {
    let budget = i32::from(difficulty).max(1);
    loop {
        let trap = rng.below(TRAP_TYPES as u32) as usize;
        if TRAP_GRADE[trap] <= budget && TRAP_GRADE[trap] + 4 >= budget {
            return trap as u8;
        }
    }
}

/// The member's character and vitals, if they are in the party.
fn member(world: &mut World, actor: Entity) -> Result<(&Character, &Vitals), Unable> {
    if crate::game::world::party_occupied(world) {
        return Err(Unable::NotNow);
    }
    if world.get::<PartyMember>(actor).is_none() {
        return Err(Unable::NotMember);
    }
    let entity = world.get_entity(actor).map_err(|_| Unable::NotMember)?;
    let vitals = entity.get::<Vitals>().ok_or(Unable::NotMember)?;
    if !vitals.can_act() {
        return Err(Unable::CannotAct);
    }
    Ok((entity.get::<Character>().ok_or(Unable::NotMember)?, vitals))
}

/// Whether the device is still guarded and on the loaded level.
fn guarded_on_level(world: &World, device: Entity) -> bool {
    world.get::<Guarded>(device).is_some()
        && world
            .get::<super::level::LevelOwned>(device)
            .is_some_and(|owner| world.get::<super::level::Level>(owner.0).is_some())
}

fn rng(world: &mut World) -> Mut<'_, GameRng> {
    world.resource_mut::<GameRng>()
}

/// Raise a tumbler (retail `OnTumblerPicked`, `ResolvePick`). Every tumbler
/// the actor already holds may drop, each by the actor's skill; when all
/// are up the lock opens.
pub fn pick_tumbler(world: &mut World, actor: Entity, tumbler: usize) -> Result<(), Unable> {
    let (character, vitals) = member(world, actor)?;
    let skill = lock_skill(character, vitals).ok_or(Unable::Unskilled)?;
    let Some(DeviceInteraction { work: Work::Picking(picking), .. }) =
        world.get_resource::<DeviceInteraction>()
    else {
        return Err(Unable::NotNow);
    };
    if picking.tumblers.get(tumbler).is_none_or(|pin| pin.raised()) {
        return Err(Unable::Done);
    }
    let mut picking = picking.clone();
    if picking.picker != Some(actor) {
        for held in &mut picking.tumblers {
            if matches!(*held, Tumbler::Held(owner) if Some(owner) == picking.picker) {
                *held = Tumbler::Down;
            }
        }
        picking.picker = Some(actor);
    }
    let chance = (skill as i32 + 5 - 5 * *world.resource::<Difficulty>() as i32).max(0) as u32;
    let mut dropped = false;
    for held in &mut picking.tumblers {
        if *held == Tumbler::Held(actor) && chance <= rng(world).below(100) {
            *held = Tumbler::Down;
            dropped = true;
        }
    }
    picking.tumblers[tumbler] = Tumbler::Held(actor);
    // Retail rolls even when the device's finite practice budget is exhausted.
    if rng(world).below(5) == 0 {
        let device = world.resource::<DeviceInteraction>().device;
        if let Some(mut guarded) = world.get_mut::<Guarded>(device)
            && guarded.practice_left > 0
        {
            guarded.practice_left -= 1;
            practise_locks(world, actor);
        }
    }
    world.write_message(LockNews::Tumbler { dropped });
    let open = picking.tumblers.iter().all(|pin| pin.raised());
    let mut interaction = world.resource_mut::<DeviceInteraction>();
    interaction.work = Work::Picking(picking);
    if open {
        world.write_message(LockNews::Picked);
        complete(world);
    }
    Ok(())
}

/// Force the lock (retail `AttemptForce`): it costs stamina whatever comes
/// of it.
pub fn force_lock(world: &mut World, actor: Entity) -> Result<(), Unable> {
    let Some(DeviceInteraction { work: Work::Picking(picking), .. }) =
        world.get_resource::<DeviceInteraction>()
    else {
        return Err(Unable::NotNow);
    };
    let tumblers = picking.tumblers.len();
    let setting = *world.resource::<Difficulty>();
    let (character, vitals) = member(world, actor)?;
    let chance = force_chance(character, vitals, tumblers, setting).ok_or(Unable::TooWeak)?;
    world.get_mut::<Vitals>(actor).unwrap().spend_stamina(FORCE_STAMINA);
    if rng(world).below(100) < chance {
        world.write_message(LockNews::Forced { member: actor });
        complete(world);
    } else {
        world.write_message(LockNews::NotForced { member: actor });
    }
    Ok(())
}

fn disarming(world: &World) -> Result<&Disarming, Unable> {
    match world.get_resource::<DeviceInteraction>() {
        Some(DeviceInteraction { work: Work::Disarming(disarming), .. }) => Ok(disarming),
        _ => Err(Unable::NotNow),
    }
}

/// Check the source, level and dependencies before any failure can discharge.
fn ready_trap(world: &mut World) -> Result<Disarming, Unable> {
    if crate::game::world::party_occupied(world) {
        return Err(Unable::NotNow);
    }
    let interaction = world.get_resource::<DeviceInteraction>().ok_or(Unable::NotNow)?;
    let device = interaction.device;
    let trap = disarming(world)?.clone();
    if !guarded_on_level(world, device) {
        return Err(Unable::NotNow);
    }
    if let Err(error) = crate::game::magic::traps::validate(world, trap.trap) {
        if error == Unable::UnsupportedTrap {
            world.write_message(LockNews::UnsupportedTrap);
        }
        return Err(error);
    }
    Ok(trap)
}

fn practise_locks(world: &mut World, actor: Entity) {
    let improved = world.resource_scope(|world, mut rng: Mut<GameRng>| {
        world.get_mut::<Character>(actor).unwrap().practise(Skill::LocksAndTraps, 1, &mut rng)
    });
    if improved {
        world.write_message(LockNews::SkillImproved { member: actor });
    }
}

pub fn inspect_trap(world: &mut World, actor: Entity) -> Result<(), Unable> {
    let trap = ready_trap(world)?;
    let (character, vitals) = member(world, actor)?;
    let skill = lock_skill(character, vitals).ok_or(Unable::Unskilled)?;
    let setting = *world.resource::<Difficulty>();
    let chance = trap.inspection_chance(Some(skill), setting);
    if rng(world).below(100) >= chance {
        return spring_trap(world, Some(actor));
    }
    // Practice may increase this very inspection's observation chance.
    practise_locks(world, actor);
    let (character, vitals) = member(world, actor)?;
    let chance = trap.inspection_chance(lock_skill(character, vitals), setting);
    let seen = std::array::from_fn(|column| {
        if rng(world).below(100) >= chance {
            Observation::Unknown
        } else if TRAP_COMPONENTS[usize::from(trap.trap)][column] {
            Observation::Present
        } else {
            Observation::Absent
        }
    });
    let mut interaction = world.resource_mut::<DeviceInteraction>();
    let Work::Disarming(trap) = &mut interaction.work else { unreachable!() };
    if let Some((_, previous)) = trap.observations.iter_mut().find(|(member, _)| *member == actor) {
        *previous = seen;
    } else {
        trap.observations.push((actor, seen));
    }
    world.write_message(LockNews::Inspected { member: actor });
    Ok(())
}

pub fn disarm_component(world: &mut World, actor: Entity, component: usize) -> Result<(), Unable> {
    let trap = ready_trap(world)?;
    if component >= COMPONENTS || trap.disarmed[component] {
        return Err(Unable::Done);
    }
    let (character, vitals) = member(world, actor)?;
    let skill = lock_skill(character, vitals).ok_or(Unable::Unskilled)?;
    let chance = trap.disarm_chance(Some(skill), *world.resource::<Difficulty>());
    if !TRAP_COMPONENTS[usize::from(trap.trap)][component] || rng(world).below(100) >= chance {
        return spring_trap(world, Some(actor));
    }
    let mut interaction = world.resource_mut::<DeviceInteraction>();
    let Work::Disarming(work) = &mut interaction.work else { unreachable!() };
    work.disarmed[component] = true;
    let done = TRAP_COMPONENTS[usize::from(trap.trap)]
        .iter()
        .zip(work.disarmed)
        .all(|(&required, disarmed)| !required || disarmed);
    practise_locks(world, actor);
    world.write_message(LockNews::ComponentDisarmed { member: actor, component });
    if done {
        world.write_message(LockNews::Disarmed { trap: trap.trap });
        complete(world);
    }
    Ok(())
}

pub fn spring_trap(world: &mut World, selected: Option<Entity>) -> Result<(), Unable> {
    let trap = ready_trap(world)?;
    let destroyed =
        crate::game::magic::traps::discharge(world, trap.trap, trap.difficulty, selected)?;
    world.write_message(LockNews::Sprung { trap: trap.trap, destroyed });
    if destroyed {
        complete(world);
    } else {
        leave(world);
    }
    Ok(())
}

/// Opening either selector releases this actor's spell-held pins even if
/// the uncommitted selector is cancelled, as retail ReleaseOwnedTumblers does.
pub fn prepare_device_magic(
    world: &mut World,
    actor: Entity,
    device: Entity,
) -> Result<(), Unable> {
    member(world, actor)?;
    let mut interaction = world.get_resource_mut::<DeviceInteraction>().ok_or(Unable::NotNow)?;
    if interaction.device != device {
        return Err(Unable::NotNow);
    }
    if let Work::Picking(picking) = &mut interaction.work {
        for pin in &mut picking.tumblers {
            if *pin == Tumbler::Sealed(actor) {
                *pin = Tumbler::Down;
            }
        }
    }
    Ok(())
}

/// Per mille that Knock-Knock moves each tumbler it reaches, by power.
const KNOCK_CHANCE: [u32; 8] = [0, 344, 459, 516, 550, 573, 589, 602];

fn knock(world: &mut World, actor: Entity, power: u8, backfire: bool) {
    let (done, dropped) = world.resource_scope(|world, mut rng: Mut<GameRng>| {
        let mut interaction = world.resource_mut::<DeviceInteraction>();
        let Work::Picking(picking) = &mut interaction.work else { unreachable!() };
        let mut order: Vec<_> = (0..picking.tumblers.len()).collect();
        for _ in 0..order.len() {
            let a = rng.below(order.len() as u32) as usize;
            let b = rng.below(order.len() as u32) as usize;
            order.swap(a, b);
        }
        let mut dropped = false;
        for index in order.into_iter().take(usize::from(power) + 1) {
            if rng.below(1000) < KNOCK_CHANCE[usize::from(power)] {
                let pin = &mut picking.tumblers[index];
                if backfire {
                    dropped |= pin.raised();
                    *pin = Tumbler::Down;
                } else {
                    *pin = Tumbler::Sealed(actor);
                }
            }
        }
        (picking.tumblers.iter().all(|pin| pin.raised()), dropped)
    });
    world.write_message(LockNews::Tumbler { dropped });
    if done {
        world.write_message(LockNews::Picked);
        complete(world);
    }
}

/// One exclusive operation: target preflight, shared cast/payment/practice,
/// and the device effect. No observer or unconsumed command owns consequences.
pub fn cast_device(
    world: &mut World,
    actor: Entity,
    device: Entity,
    source: crate::game::magic::casting::Source,
) -> Result<(), crate::game::magic::casting::Refusal> {
    use crate::game::magic::{
        Outcome,
        casting::{MagicNews, Refusal, UseContext},
    };
    member(world, actor).map_err(|_| Refusal::CannotAct)?;
    let interaction = world.get_resource::<DeviceInteraction>().ok_or(Refusal::NoTarget)?;
    if interaction.device != device || !guarded_on_level(world, device) {
        return Err(Refusal::NoTarget);
    }
    let context = match interaction.work {
        Work::Picking(_) => UseContext::Lock,
        Work::Disarming(_) => UseContext::Trap,
        Work::Looting => return Err(Refusal::NoTarget),
    };
    if context == UseContext::Trap {
        ready_trap(world).map_err(|why| match why {
            Unable::UnsupportedTrap => Refusal::Unsupported,
            Unable::MissingContent => Refusal::MissingContent,
            _ => Refusal::NoTarget,
        })?;
    }
    let (report, strength) =
        crate::game::magic::devices::cast_source(world, actor, source, context)?;
    let outcome = report.outcome;
    let spell = report.spell;
    world.write_message(MagicNews::Done(report));
    if outcome == Outcome::Fizzled {
        return Ok(());
    }
    let power =
        (u32::from(strength.power) + u32::from(strength.power) * strength.boost / 100).min(7) as u8;
    if spell == SpellId::DIVINE_TRAP {
        let mut interaction = world.resource_mut::<DeviceInteraction>();
        let Work::Disarming(trap) = &mut interaction.work else { unreachable!() };
        trap.divine_power = power;
    } else if context == UseContext::Lock {
        knock(world, actor, power, outcome == Outcome::Backfired);
    } else {
        let trap = disarming(world).unwrap();
        let (kind, difficulty) = (trap.trap, trap.difficulty);
        let chance = if outcome == Outcome::Backfired {
            0
        } else {
            i32::from(power) * 5 + 50 - i32::from(difficulty) * 6
        };
        if (rng(world).below(100) as i32) < chance {
            world.write_message(LockNews::Disarmed { trap: kind });
            complete(world);
        } else {
            spring_trap(world, Some(actor)).map_err(|_| Refusal::MissingContent)?;
        }
    }
    Ok(())
}

/// The lock or trap is overcome: the interaction ends and the device runs
/// (retail `CompleteItemInteraction` then `Run`).
fn complete(world: &mut World) {
    let Some(DeviceInteraction { device, .. }) = world.remove_resource::<DeviceInteraction>()
    else {
        return;
    };
    let Ok(mut entity) = world.get_entity_mut(device) else { return };
    entity.remove::<Guarded>();
    if let Some(mut door) = entity.get_mut::<DoorState>() {
        door.locked = false;
    }
    super::triggers::activate(world, device, None);
}

/// Leave the device as it is, forgetting the attempt in progress.
pub fn leave(world: &mut World) {
    world.remove_resource::<DeviceInteraction>();
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<DeviceRequest>().add_systems(
        Update,
        handle_device_requests
            .after(crate::game::GameplaySet::LevelActivation)
            .before(crate::game::GameplaySet::Combat),
    );
}

/// What the player asks of the device being worked on.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum DeviceRequest {
    Pick {
        actor: Entity,
        tumbler: usize,
    },
    Force {
        actor: Entity,
    },
    Inspect {
        actor: Entity,
    },
    Disarm {
        actor: Entity,
        component: usize,
    },
    Spring {
        actor: Option<Entity>,
    },
    /// A spell or item selector opens on the device.
    PrepareMagic {
        actor: Entity,
        device: Entity,
    },
    Take {
        picks: Vec<(StackId, u16)>,
        to: super::containers::Recipient,
    },
    Leave,
}

/// Carry out the player's device requests in order. A refused request
/// changes nothing: retail offers only the controls that can work.
pub fn handle_device_requests(world: &mut World) {
    let requests: Vec<_> = world.resource_mut::<Messages<DeviceRequest>>().drain().collect();
    for request in requests {
        match &request {
            &DeviceRequest::Pick { actor, tumbler } => {
                report(&request, pick_tumbler(world, actor, tumbler))
            }
            &DeviceRequest::Force { actor } => report(&request, force_lock(world, actor)),
            &DeviceRequest::Inspect { actor } => report(&request, inspect_trap(world, actor)),
            &DeviceRequest::Disarm { actor, component } => {
                report(&request, disarm_component(world, actor, component))
            }
            &DeviceRequest::Spring { actor } => report(&request, spring_trap(world, actor)),
            &DeviceRequest::PrepareMagic { actor, device } => {
                report(&request, prepare_device_magic(world, actor, device))
            }
            DeviceRequest::Take { picks, to } => {
                report(&request, super::containers::take_from_container(world, picks, *to))
            }
            DeviceRequest::Leave => leave(world),
        }
    }
}

fn report<E: std::fmt::Debug>(request: &DeviceRequest, result: Result<(), E>) {
    if let Err(why) = result {
        debug!("{request:?} refused: {why:?}");
    }
}

/// An interaction ends with its device, an accepted entry, or a fight.
/// A fight or an entry ends work on a device.
pub(crate) fn interrupt_interaction(
    _: On<Add<(crate::game::combat::Battle, super::level::Destination)>>,
    mut commands: Commands,
) {
    commands.remove_resource::<DeviceInteraction>();
}

/// Work on a device ends with the device.
pub(crate) fn end_lost_interaction(
    lost: On<Remove<Device>>,
    mut commands: Commands,
    interaction: Option<Res<DeviceInteraction>>,
) {
    if interaction.is_some_and(|i| i.device == lost.entity) {
        commands.remove_resource::<DeviceInteraction>();
    }
}

#[cfg(test)]
mod tests;
