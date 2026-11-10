//! Locks and traps on devices, and the short-lived interaction in which the
//! party works on one or looks through a container (decomp
//! `MainGameScreen.cpp` lock and trap panels, `Traps.cpp`).
//!
//! The device owns what is settled: whether its lock or trap still stands,
//! the trap's type once rolled, and its contents. A [`DeviceInteraction`]
//! entity owns the attempt in progress: who holds which tumbler, which trap
//! components are disarmed and what each member's inspection made of them.
//! It is related to its device ([`AttemptOn`]), so it ends with the device or
//! its level, and every request names the attempt it was made for, so a
//! request left over from an attempt that has ended changes nothing.
//! Leaving the interaction forgets those, as retail's panels do. Each
//! attempt names its actor, validates them and commits its costs and
//! consequences at once; retail's progress bars and tumbler animations are
//! presentation and do not delay the outcome.
//!
//! Knock-Knock, Divine Trap and tools share source validation and payment. Traps
//! whose missile, poison, hearing or summoning prerequisites remain unported
//! are refused before spending RNG or changing the device.

use bevy::prelude::*;

use super::triggers::DoorState;
use crate::content::characters::{Attribute, Skill};
use crate::content::level::{Lock, TRAP_TYPES};
use crate::content::spells::SpellId;
use crate::game::characters::{Character, Practise, Vitals};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::Release;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::{Body, BodyItem};
use crate::game::inventory::{PartyInventory, StackId};
use crate::game::magic::casting::{Aim, MagicNews, Refusal, Source, UseContext};
use crate::game::magic::{Outcome, Utility};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

/// The parts a trap is built from; each type uses a few of them.
pub const COMPONENTS: usize = 8;

/// Each trap type's own grade (retail `g_trap_difficulty`).
pub(crate) const TRAP_GRADE: [u32; TRAP_TYPES] = [1, 1, 2, 2, 3, 3, 3, 4, 5, 5, 5, 6, 6, 7, 7];

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
const REMARK_AGAIN_MS: u64 = 4 * crate::game::time::HOUR_MS;

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

/// The interaction under way with one device, on an entity of its own: the
/// attempt. While it lasts the party stays put; it ends when the party
/// leaves it, the device opens or goes, the party travels or a fight begins.
/// There is at most one.
#[derive(Component, Debug)]
#[require(Name::new("Device attempt"))]
pub struct DeviceInteraction {
    pub work: Work,
}

/// The device an attempt works on.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = DeviceAttempt)]
pub struct AttemptOn(pub Entity);

/// The attempt under way on a device, despawned with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = AttemptOn, linked_spawn)]
pub struct DeviceAttempt(Entity);

/// The attempt under way and its device, if any.
pub fn current(world: &World) -> Option<(Entity, Entity)> {
    let mut attempts =
        world.try_query_filtered::<(Entity, &AttemptOn), With<DeviceInteraction>>()?;
    attempts.iter(world).next().map(|(attempt, on)| (attempt, on.0))
}

/// Start an attempt on a device.
pub(crate) fn begin(world: &mut World, device: Entity, work: Work) -> Entity {
    world.spawn((DeviceInteraction { work }, AttemptOn(device))).id()
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
    pub divine_power: u32,
    /// What each member's inspection made of each component.
    pub observations: Vec<(Entity, [Observation; COMPONENTS])>,
}

impl Disarming {
    pub fn inspection_chance(&self, skill: Option<u32>, difficulty: Difficulty) -> u32 {
        trap_chance(skill.map(|s| s + self.divine_power * 6), self.difficulty, difficulty) as u32
    }
    pub fn disarm_chance(&self, skill: Option<u32>, difficulty: Difficulty) -> u32 {
        let chance = trap_chance(
            skill.map(|s| s + self.divine_power.div_ceil(2) * 6),
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
    CannotAct,
    /// The member lacks Locks & Traps, or cannot act.
    Unskilled,
    /// Too weak or too tired to force the lock.
    TooWeak,
    /// That tumbler or component is already done.
    Done,
    UnsupportedTrap,
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
    strength: u32,
    vitals: &Vitals,
    tumblers: usize,
    setting: Difficulty,
) -> Option<u32> {
    if !vitals.can_act() || vitals.stamina.current < FORCE_STAMINA || strength <= FORCE_STRENGTH {
        return None;
    }
    let divisor = (setting as usize + tumblers - 1).clamp(2, 8);
    Some((strength - FORCE_STRENGTH) >> (divisor - 2))
}

/// A device's lock or trap stops its use: start working on it (retail
/// `OpenLockInteraction`, `OpenTrapInteraction`). Facing a trap rolls its
/// type the first time.
pub(crate) fn confront(world: &mut World, device: Entity, lock: Lock) {
    if current(world).is_some() {
        return;
    }
    if world.query_filtered::<Entity, With<Battle>>().iter(world).next().is_some() {
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
            let remark = guarded.faced_at.is_none_or(|at| now - at > REMARK_AGAIN_MS);
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
    begin(world, device, work);
}

/// Retail `SelectTrapType`: draw types until one's grade lies within four
/// below the authored grade (at least 1).
fn roll_trap(difficulty: u8, rng: &mut GameRng) -> u8 {
    let budget = u32::from(difficulty).max(1);
    loop {
        let trap = rng.below(TRAP_TYPES as u32) as usize;
        if TRAP_GRADE[trap] <= budget && TRAP_GRADE[trap] + 4 >= budget {
            return trap as u8;
        }
    }
}

/// The member acting, unless the party is entering a level or fighting.
fn member<'a, 's>(
    bodies: &'a mut Query<'_, 's, Body>,
    actor: Entity,
    occupied: bool,
) -> Result<BodyItem<'a, 's>, Unable> {
    bodies.get_mut(actor).ok().filter(|_| !occupied).ok_or(Unable::NotNow)
}

/// What an operation on the attempt came to.
enum Step {
    /// The attempt goes on.
    Stay,
    /// The lock or trap is overcome.
    Opened,
    /// The trap goes off, on this member if anyone set it off.
    Sprung(Option<Entity>),
}

/// A member's Locks & Traps level if they can use it now.
fn skill(body: &BodyItem) -> Result<u32, Unable> {
    if !body.vitals.can_act() {
        return Err(Unable::CannotAct);
    }
    let character = body.character.as_deref().ok_or(Unable::CannotAct)?;
    lock_skill(character, &body.vitals).ok_or(Unable::Unskilled)
}

/// Raise a tumbler (retail `OnTumblerPicked`, `ResolvePick`). Every tumbler
/// the actor already holds may drop, each by the actor's skill; when all
/// are up the lock opens.
fn pick_tumbler(
    picking: &mut Picking,
    tumbler: usize,
    body: &mut BodyItem,
    guarded: Option<Mut<Guarded>>,
    setting: Difficulty,
    rng: &mut GameRng,
    (news, commands): (&mut MessageWriter<LockNews>, &mut Commands),
) -> Result<Step, Unable> {
    let actor = body.entity;
    let skill = skill(body)?;
    if picking.tumblers.get(tumbler).is_none_or(|pin| pin.raised()) {
        return Err(Unable::Done);
    }
    if picking.picker != Some(actor) {
        for held in &mut picking.tumblers {
            if matches!(*held, Tumbler::Held(owner) if Some(owner) == picking.picker) {
                *held = Tumbler::Down;
            }
        }
        picking.picker = Some(actor);
    }
    let penalty = 5 * setting as u32;
    let chance = (skill + 5).max(penalty) - penalty;
    let mut dropped = false;
    for held in &mut picking.tumblers {
        if *held == Tumbler::Held(actor) && chance <= rng.below(100) {
            *held = Tumbler::Down;
            dropped = true;
        }
    }
    picking.tumblers[tumbler] = Tumbler::Held(actor);
    // The device's practice is finite; leaving the panel does not renew it.
    if rng.below(5) == 0
        && let Some(mut guarded) = guarded
        && guarded.practice_left > 0
    {
        guarded.practice_left -= 1;
        commands.trigger(Practise { member: actor, skill: Skill::LocksAndTraps, usage: 1 });
    }
    news.write(LockNews::Tumbler { dropped });
    if picking.tumblers.iter().all(|pin| pin.raised()) {
        news.write(LockNews::Picked);
        return Ok(Step::Opened);
    }
    Ok(Step::Stay)
}

/// Force the lock (retail `AttemptForce`): it costs stamina whatever comes
/// of it.
fn force_lock(
    picking: &Picking,
    body: &mut BodyItem,
    setting: Difficulty,
    rng: &mut GameRng,
    news: &mut MessageWriter<LockNews>,
) -> Result<Step, Unable> {
    let member = body.entity;
    if !body.vitals.can_act() {
        return Err(Unable::CannotAct);
    }
    let chance = force_chance(
        body.attributes[Attribute::Strength],
        &body.vitals,
        picking.tumblers.len(),
        setting,
    )
    .ok_or(Unable::TooWeak)?;
    body.vitals.tire(FORCE_STAMINA);
    if rng.below(100) < chance {
        news.write(LockNews::Forced { member });
        return Ok(Step::Opened);
    }
    news.write(LockNews::NotForced { member });
    Ok(Step::Stay)
}

/// Look a trap over: a failed inspection sets it off; otherwise the member
/// makes out each component at the same chance, practice counting at once.
fn inspect_trap(
    trap: &mut Disarming,
    body: &mut BodyItem,
    setting: Difficulty,
    rng: &mut GameRng,
    (news, commands): (&mut MessageWriter<LockNews>, &mut Commands),
) -> Result<Step, Unable> {
    let actor = body.entity;
    let chance = trap.inspection_chance(Some(skill(body)?), setting);
    if rng.below(100) >= chance {
        return Ok(Step::Sprung(Some(actor)));
    }
    // Practice may increase this very inspection's observation chance.
    commands.trigger(Practise { member: actor, skill: Skill::LocksAndTraps, usage: 1 });
    let chance = trap.inspection_chance(skill(body).ok(), setting);
    let seen = std::array::from_fn(|column| {
        if rng.below(100) >= chance {
            Observation::Unknown
        } else if TRAP_COMPONENTS[usize::from(trap.trap)][column] {
            Observation::Present
        } else {
            Observation::Absent
        }
    });
    match trap.observations.iter_mut().find(|(member, _)| *member == actor) {
        Some((_, previous)) => *previous = seen,
        None => trap.observations.push((actor, seen)),
    }
    news.write(LockNews::Inspected { member: actor });
    Ok(Step::Stay)
}

/// Disarm one component: a part the trap lacks, or a failed attempt, sets
/// it off; with every part it has disarmed, the trap is.
fn disarm_component(
    trap: &mut Disarming,
    component: usize,
    body: &mut BodyItem,
    setting: Difficulty,
    rng: &mut GameRng,
    (news, commands): (&mut MessageWriter<LockNews>, &mut Commands),
) -> Result<Step, Unable> {
    let actor = body.entity;
    if component >= COMPONENTS || trap.disarmed[component] {
        return Err(Unable::Done);
    }
    let chance = trap.disarm_chance(Some(skill(body)?), setting);
    let parts = TRAP_COMPONENTS[usize::from(trap.trap)];
    if !parts[component] || rng.below(100) >= chance {
        return Ok(Step::Sprung(Some(actor)));
    }
    trap.disarmed[component] = true;
    commands.trigger(Practise { member: actor, skill: Skill::LocksAndTraps, usage: 1 });
    news.write(LockNews::ComponentDisarmed { member: actor, component });
    if parts.iter().zip(trap.disarmed).all(|(&required, disarmed)| !required || disarmed) {
        news.write(LockNews::Disarmed { trap: trap.trap });
        return Ok(Step::Opened);
    }
    Ok(Step::Stay)
}

/// Per mille that Knock-Knock moves each tumbler it reaches, by power.
const KNOCK_CHANCE: [u32; 8] = [0, 344, 459, 516, 550, 573, 589, 602];

/// Knock-Knock reaches a few tumblers at random and seals each it moves; a
/// backfire drops them instead.
fn knock(
    picking: &mut Picking,
    actor: Entity,
    (power, backfire): (u32, bool),
    rng: &mut GameRng,
    news: &mut MessageWriter<LockNews>,
) -> Step {
    let mut order: Vec<_> = (0..picking.tumblers.len()).collect();
    for _ in 0..order.len() {
        let a = rng.below(order.len() as u32) as usize;
        let b = rng.below(order.len() as u32) as usize;
        order.swap(a, b);
    }
    let mut dropped = false;
    for index in order.into_iter().take(power as usize + 1) {
        if rng.below(1000) < KNOCK_CHANCE[power as usize] {
            let pin = &mut picking.tumblers[index];
            if backfire {
                dropped |= pin.raised();
                *pin = Tumbler::Down;
            } else {
                *pin = Tumbler::Sealed(actor);
            }
        }
    }
    news.write(LockNews::Tumbler { dropped });
    if picking.tumblers.iter().all(|pin| pin.raised()) {
        news.write(LockNews::Picked);
        return Step::Opened;
    }
    Step::Stay
}

/// Opening either selector releases this actor's spell-held pins even if
/// the uncommitted selector is cancelled, as retail ReleaseOwnedTumblers does.
/// The device attempt magic may be aimed at now: the one under way, unless
/// the party is looting what it opened. `requested` is the attempt the
/// caller means; it is refused when another, or none, is under way.
pub fn magic_attempt(world: &World, requested: Option<Entity>) -> Result<Option<Entity>, Unable> {
    match current(world) {
        Some((attempt, _)) => {
            let looting = matches!(
                world.get::<DeviceInteraction>(attempt).map(|i| &i.work),
                Some(Work::Looting)
            );
            if looting || requested.is_some_and(|requested| requested != attempt) {
                return Err(Unable::NotNow);
            }
            Ok(Some(attempt))
        }
        None if requested.is_some() => Err(Unable::NotNow),
        None => Ok(None),
    }
}

/// Travel and fights end the attempt under way (its device's own end takes
/// the attempt with it through [`DeviceAttempt`]).
pub(crate) fn end_attempts(
    _: On<Add<(super::level::Destination, Battle)>>,
    attempts: Query<Entity, With<DeviceInteraction>>,
    mut commands: Commands,
) {
    for attempt in &attempts {
        commands.entity(attempt).despawn();
    }
}

/// What the player asks of the attempt under way. It names its attempt,
/// so one left over from an attempt that has ended changes nothing.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct DeviceRequest {
    pub on: Entity,
    pub act: DeviceAct,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceAct {
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
    },
    /// A spell or an item's is cast at the lock or trap.
    Cast {
        actor: Entity,
        source: Source,
    },
    Take {
        picks: Vec<(StackId, u16)>,
        to: super::containers::Recipient,
    },
    Leave,
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<DeviceRequest>().add_systems(
        Update,
        (handle_device_requests, super::containers::take_from_containers)
            .chain()
            .after(crate::game::magic::accept_magic)
            .before(super::activate_devices),
    );
}

/// A cast aimed at the attempt under way, to be carried out with the
/// player's other requests on it.
fn device_casts(accepted: &mut MessageReader<Utility>) -> Vec<(Entity, DeviceAct)> {
    accepted
        .read()
        .filter_map(|cast| match cast.choice.aim {
            Aim::Device(on) => {
                Some((on, DeviceAct::Cast { actor: cast.caster, source: cast.choice.source }))
            }
            _ => None,
        })
        .collect()
}

/// Carry out what is asked of the attempt under way, in order: the
/// player's requests, then casts aimed at it. Each names its actor,
/// validates them and commits its costs and consequences at once. A refused
/// request changes nothing (retail offers only the controls that can work);
/// a refused cast is told of. A lock or trap overcome ends the attempt and
/// the device runs (retail `CompleteItemInteraction` then `Run`); a trap
/// sprung discharges at the party.
#[allow(clippy::type_complexity)]
pub fn handle_device_requests(
    mut commands: Commands,
    (mut requests, mut accepted): (MessageReader<DeviceRequest>, MessageReader<Utility>),
    mut attempts: Query<(&mut DeviceInteraction, &AttemptOn)>,
    mut devices: Query<(Option<&mut Guarded>, Option<&mut DoorState>)>,
    mut bodies: Query<Body>,
    party: Single<(&crate::game::party::roster::PartyRoster, &mut PartyInventory)>,
    (entering, battle): (Query<(), With<super::level::Destination>>, Query<(), With<Battle>>),
    states: Query<&super::triggers::LevelState, With<super::level::Level>>,
    (content, definitions, setting): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    (mut news, mut activations, mut released, mut magic_news): (
        MessageWriter<LockNews>,
        MessageWriter<super::ActivateDevice>,
        MessageWriter<Release>,
        MessageWriter<MagicNews>,
    ),
) {
    let (roster, mut inventory) = party.into_inner();
    let occupied = !entering.is_empty() || !battle.is_empty();
    let asked: Vec<_> = requests
        .read()
        .map(|request| (request.on, request.act.clone()))
        .chain(device_casts(&mut accepted))
        .collect();
    for (on, act) in asked {
        // A request may name an attempt that has since ended.
        let Ok((mut interaction, &AttemptOn(device))) = attempts.get_mut(on) else {
            if let DeviceAct::Cast { actor, .. } = act {
                let refusal = Refusal::NoTarget;
                magic_news.write(MagicNews::Refused { caster: actor, refusal });
            }
            continue;
        };
        let Ok((mut guarded, door)) = devices.get_mut(device) else { continue };
        let work = &mut interaction.bypass_change_detection().work;
        // A trap whose discharge cannot be resolved stays guarded: nothing
        // that could set it off is attempted.
        let ready = |work: &Work, news: &mut MessageWriter<LockNews>| match work {
            Work::Disarming(trap) if guarded.is_some() => {
                crate::game::magic::traps::validate(trap.trap, &definitions).inspect_err(|_| {
                    news.write(LockNews::UnsupportedTrap);
                })
            }
            Work::Disarming(_) => Err(Unable::NotNow),
            _ => Ok(()),
        };
        let step = match (act, &mut *work) {
            (DeviceAct::Leave, _) => {
                commands.entity(on).despawn();
                continue;
            }
            (DeviceAct::Pick { actor: by, tumbler }, Work::Picking(picking)) => {
                member(&mut bodies, by, occupied).and_then(|mut body| {
                    let guarded = guarded.as_mut().map(|guarded| guarded.reborrow());
                    pick_tumbler(
                        picking,
                        tumbler,
                        &mut body,
                        guarded,
                        *setting,
                        &mut rng,
                        (&mut news, &mut commands),
                    )
                })
            }
            (DeviceAct::Force { actor: by }, Work::Picking(picking)) => {
                member(&mut bodies, by, occupied).and_then(|mut body| {
                    force_lock(picking, &mut body, *setting, &mut rng, &mut news)
                })
            }
            (DeviceAct::Inspect { actor: by }, Work::Disarming(_)) => ready(work, &mut news)
                .and_then(|()| member(&mut bodies, by, occupied))
                .and_then(|mut body| {
                    let Work::Disarming(trap) = work else { unreachable!() };
                    inspect_trap(trap, &mut body, *setting, &mut rng, (&mut news, &mut commands))
                }),
            (DeviceAct::Disarm { actor: by, component }, Work::Disarming(_)) => {
                ready(work, &mut news).and_then(|()| member(&mut bodies, by, occupied)).and_then(
                    |mut body| {
                        let Work::Disarming(trap) = work else { unreachable!() };
                        disarm_component(
                            trap,
                            component,
                            &mut body,
                            *setting,
                            &mut rng,
                            (&mut news, &mut commands),
                        )
                    },
                )
            }
            (DeviceAct::Spring { actor }, Work::Disarming(_)) if !occupied => {
                ready(work, &mut news).map(|()| Step::Sprung(actor))
            }
            // Opening a spell or item selector releases this actor's
            // spell-held pins even if the selector is then cancelled, as
            // retail `ReleaseOwnedTumblers` does.
            (DeviceAct::PrepareMagic { actor: by }, Work::Picking(picking)) => {
                member(&mut bodies, by, occupied).map(|_| {
                    for pin in &mut picking.tumblers {
                        if *pin == Tumbler::Sealed(by) {
                            *pin = Tumbler::Down;
                        }
                    }
                    Step::Stay
                })
            }
            // Knock-Knock, Divine Trap and tools share the cast's payment;
            // the target is settled before anything is spent.
            (DeviceAct::Cast { actor: by, source }, Work::Picking(_) | Work::Disarming(_)) => {
                let context = match work {
                    Work::Picking(_) => UseContext::Lock,
                    _ => UseContext::Trap,
                };
                let paid = match (guarded.is_some(), ready(work, &mut news)) {
                    (false, _) | (_, Err(Unable::NotNow)) => Err(Refusal::NoTarget),
                    (_, Err(_)) => Err(Refusal::Unsupported),
                    (true, Ok(())) => member(&mut bodies, by, occupied)
                        .map_err(|_| Refusal::CannotAct)
                        .and_then(|mut body| {
                            if !body.vitals.can_act() {
                                return Err(Refusal::CannotAct);
                            }
                            crate::game::magic::casting::pay(
                                &mut body,
                                &mut inventory,
                                (source, context),
                                (
                                    &content,
                                    *setting,
                                    crate::game::magic::utilities::underwater(&states),
                                ),
                                &mut rng,
                                &mut commands,
                            )
                        }),
                };
                let (report, strength) = match paid {
                    Ok(paid) => paid,
                    Err(refusal) => {
                        magic_news.write(MagicNews::Refused { caster: by, refusal });
                        continue;
                    }
                };
                let (outcome, spell) = (report.outcome, report.spell);
                magic_news.write(MagicNews::Done(report));
                let power = (strength.power + strength.power * strength.boost / 100).min(7);
                Ok(match work {
                    _ if outcome == Outcome::Fizzled => Step::Stay,
                    Work::Disarming(trap) if spell == SpellId::DIVINE_TRAP => {
                        trap.divine_power = power;
                        Step::Stay
                    }
                    Work::Picking(picking) => knock(
                        picking,
                        by,
                        (power, outcome == Outcome::Backfired),
                        &mut rng,
                        &mut news,
                    ),
                    Work::Disarming(trap) => {
                        let roll = rng.below(100) + u32::from(trap.difficulty) * 6;
                        if outcome != Outcome::Backfired && roll < power * 5 + 50 {
                            news.write(LockNews::Disarmed { trap: trap.trap });
                            Step::Opened
                        } else {
                            Step::Sprung(Some(by))
                        }
                    }
                    Work::Looting => unreachable!(),
                })
            }
            _ => Err(Unable::NotNow),
        };
        // The panel redraws from the attempt: it has changed.
        interaction.set_changed();
        let opened = match step {
            Ok(Step::Stay) => continue,
            Err(why) => {
                debug!("device request refused: {why:?}");
                continue;
            }
            Ok(Step::Opened) => true,
            Ok(Step::Sprung(selected)) => {
                let Work::Disarming(trap) = &interaction.work else { unreachable!() };
                let living = roster
                    .iter()
                    .filter(|&member| bodies.get(member).is_ok_and(|body| !body.vitals.is_dead()))
                    .collect();
                let (destroyed, discharge) = crate::game::magic::traps::discharge(
                    (trap.trap, trap.difficulty),
                    (living, selected),
                    (&content, &definitions),
                    &mut rng,
                );
                released.write(discharge);
                news.write(LockNews::Sprung { trap: trap.trap, destroyed });
                destroyed
            }
        };
        commands.entity(on).despawn();
        if opened {
            commands.entity(device).remove::<Guarded>();
            if let Some(mut door) = door {
                door.locked = false;
            }
            activations.write(super::ActivateDevice { device, player: false });
        }
    }
}
