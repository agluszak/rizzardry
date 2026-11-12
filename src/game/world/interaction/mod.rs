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
//! Leaving the interaction forgets those, as retail's panels do. What the
//! party does to the attempt is an event at it ([`Pick`], [`Force`],
//! [`Inspect`], [`Disarm`], [`Spring`], [`Leave`]), each with one observer
//! that validates its actor and commits its costs and consequences at once;
//! retail's progress bars and tumbler animations are presentation and do
//! not delay the outcome.
//!
//! Knock-Knock, Divine Trap and tools share source validation and payment. Traps
//! whose missile, poison, hearing or summoning prerequisites remain unported
//! are refused before spending RNG or changing the device.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::triggers::DoorState;
use crate::content::characters::{Attribute, Skill};
use crate::content::level::{Lock, TRAP_TYPES};
use crate::content::spells::Working;
use crate::game::characters::{Calling, Practise, Trained, Vitals};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::effects::{Body, BodyItem};
use crate::game::magic::Outcome;
use crate::game::magic::casting::{MagicNews, Worked};
use crate::game::magic::reach::Marked;
use crate::game::magic::workings;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

/// The parts a trap is built from; each type uses a few of them.
pub const COMPONENTS: usize = 8;

/// A kind of trap: its row in `assets/tables/traps.ron`.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrapKind {
    /// The spell it discharges at the party when it is sprung.
    pub spell: crate::content::spells::SpellId,
    /// How hard its kind is.
    pub grade: u32,
    /// The components it is built from, counted from 0.
    parts: Vec<usize>,
}

impl TrapKind {
    /// Whether it is built with a component.
    pub fn has(&self, component: usize) -> bool {
        self.parts.contains(&component)
    }
}

static TRAPS: std::sync::LazyLock<Vec<TrapKind>> = std::sync::LazyLock::new(|| {
    let kinds: Vec<TrapKind> = ron::from_str(include_str!("../../../../assets/tables/traps.ron"))
        .expect("assets/tables/traps.ron is a table of traps");
    assert_eq!(kinds.len(), TRAP_TYPES, "a row for each kind of trap a level can name");
    kinds
});

/// A kind of trap by the number a level's record gives it.
pub fn trap_kind(kind: u8) -> &'static TrapKind {
    &TRAPS[usize::from(kind)]
}

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

/// Why magic cannot be aimed at an attempt, or a trap not be worked on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unable {
    /// No such interaction, or not this kind of work.
    NotNow,
    UnsupportedTrap,
}

/// The Locks & Traps level of a member who can act, if they have the skill
/// to use (retail `GetPartySlotLocksTrapsLevel`).
pub fn lock_skill(character: Trained, skills: &crate::game::actors::Skills) -> Option<u32> {
    let level = skills[Skill::LocksAndTraps];
    (character.can_use(Skill::LocksAndTraps) || level > 0).then_some(level)
}

/// A member's chance in 100 to inspect a trap, or to disarm one of its
/// components before squaring (retail `GetTrapInteractionChance`).
pub fn trap_chance(skill: Option<u32>, difficulty: u8, setting: Difficulty) -> i32 {
    let skill = skill.map_or(-1, |s| s as i32) - GRADE_SKILL[usize::from(difficulty.min(10))];
    (skill * 3 / 2 + (11 - setting as i32) * 10).clamp(0, 99)
}

/// The chance in 100 of a member who can act to force a lock with this many
/// tumblers, if they have the stamina and strength to try (retail `W8LockInteraction::AttemptForce`).
pub fn force_chance(
    strength: u32,
    vitals: &Vitals,
    tumblers: usize,
    setting: Difficulty,
) -> Option<u32> {
    if vitals.stamina.current < FORCE_STAMINA || strength <= FORCE_STRENGTH {
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
        let trap = rng.below(TRAP_TYPES as u32) as u8;
        let grade = trap_kind(trap).grade;
        if grade <= budget && grade + 4 >= budget {
            return trap;
        }
    }
}

/// Whether the party is too busy to work on a device: entering a level, or
/// fighting.
#[derive(SystemParam)]
struct Busy<'w, 's> {
    entering: Query<'w, 's, (), With<super::level::Destination>>,
    battle: Query<'w, 's, (), With<Battle>>,
}

impl Busy<'_, '_> {
    fn busy(&self) -> bool {
        !self.entering.is_empty() || !self.battle.is_empty()
    }
}

/// A member's Locks & Traps level if they can use it now.
fn skill(body: &BodyItem) -> Option<u32> {
    lock_skill(body.trained().filter(|_| body.can_act)?, body.skills)
}

/// A member raises a tumbler of the lock being picked.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Pick {
    #[event_target]
    pub attempt: Entity,
    pub actor: Entity,
    pub tumbler: usize,
}

/// A member tries to force the lock being picked.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Force {
    #[event_target]
    pub attempt: Entity,
    pub actor: Entity,
}

/// A member looks the trap over.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Inspect {
    #[event_target]
    pub attempt: Entity,
    pub actor: Entity,
}

/// A member disarms one component of the trap.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Disarm {
    #[event_target]
    pub attempt: Entity,
    pub actor: Entity,
    pub component: usize,
}

/// The party sets the trap off on purpose, on `actor` if anyone.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Spring {
    #[event_target]
    pub attempt: Entity,
    pub actor: Option<Entity>,
}

/// A member opens a spell or item selector on the device: the pins their
/// spell holds fall, even if the selector is then cancelled (retail
/// `ReleaseOwnedTumblers`).
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct ReleasePins {
    #[event_target]
    pub attempt: Entity,
    pub actor: Entity,
}

/// The party leaves the attempt.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Leave {
    #[event_target]
    pub attempt: Entity,
}

/// The lock or trap an attempt works on is overcome: the attempt ends and
/// the device runs (retail `CompleteItemInteraction` then `Run`).
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Overcome {
    #[event_target]
    attempt: Entity,
}

/// The trap an attempt works on goes off, on `by` if anyone set it off.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Sprung {
    #[event_target]
    attempt: Entity,
    by: Option<Entity>,
}

/// Whether a trap may be worked on: it still stands, and its discharge can
/// be resolved. One that cannot stays guarded: nothing that could set it off
/// is attempted.
fn ready(
    trap: &Disarming,
    guarded: bool,
    content: &Catalogs,
    news: &mut MessageWriter<LockNews>,
) -> bool {
    guarded
        && crate::game::magic::traps::validate(trap.trap, content)
            .inspect_err(|_| {
                news.write(LockNews::UnsupportedTrap);
            })
            .is_ok()
}

/// Raise a tumbler (retail `OnTumblerPicked`, `ResolvePick`). Every tumbler
/// the actor already holds may drop, each by the actor's skill; when all
/// are up the lock opens.
fn pick(
    pick: On<Pick>,
    mut attempts: Query<(&mut DeviceInteraction, &AttemptOn)>,
    mut guards: Query<&mut Guarded>,
    mut bodies: Query<Body>,
    busy: Busy,
    setting: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<LockNews>,
    mut commands: Commands,
) {
    let Pick { attempt, actor, tumbler } = *pick;
    let Ok((mut interaction, &AttemptOn(device))) = attempts.get_mut(attempt) else { return };
    let Work::Picking(picking) = &mut interaction.work else { return };
    let Some(skill) = bodies.get_mut(actor).ok().filter(|_| !busy.busy()).and_then(|b| skill(&b))
    else {
        return;
    };
    if picking.tumblers.get(tumbler).is_none_or(|pin| pin.raised()) {
        return;
    }
    if picking.picker != Some(actor) {
        for held in &mut picking.tumblers {
            if matches!(*held, Tumbler::Held(owner) if Some(owner) == picking.picker) {
                *held = Tumbler::Down;
            }
        }
        picking.picker = Some(actor);
    }
    let penalty = 5 * *setting as u32;
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
        && let Ok(mut guarded) = guards.get_mut(device)
        && guarded.practice_left > 0
    {
        guarded.practice_left -= 1;
        commands.trigger(Practise { member: actor, skill: Skill::LocksAndTraps, usage: 1 });
    }
    news.write(LockNews::Tumbler { dropped });
    if picking.tumblers.iter().all(|pin| pin.raised()) {
        news.write(LockNews::Picked);
        commands.trigger(Overcome { attempt });
    }
}

/// Force the lock (retail `AttemptForce`): it costs stamina whatever comes
/// of it.
fn force(
    force: On<Force>,
    mut attempts: Query<&mut DeviceInteraction>,
    mut bodies: Query<Body>,
    busy: Busy,
    setting: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<LockNews>,
    mut commands: Commands,
) {
    let Force { attempt, actor: member } = *force;
    let Ok(mut interaction) = attempts.get_mut(attempt) else { return };
    let Work::Picking(picking) = &interaction.work else { return };
    let Ok(mut body) = bodies.get_mut(member) else { return };
    let strength = body.attributes[Attribute::Strength];
    let Some(chance) = force_chance(strength, &body.vitals, picking.tumblers.len(), *setting)
        .filter(|_| body.can_act && !busy.busy())
    else {
        return;
    };
    body.vitals.tire(FORCE_STAMINA);
    // The panel redraws from the attempt: what its members can do changed.
    interaction.set_changed();
    if rng.below(100) < chance {
        news.write(LockNews::Forced { member });
        commands.trigger(Overcome { attempt });
    } else {
        news.write(LockNews::NotForced { member });
    }
}

/// Look a trap over: a failed inspection sets it off; otherwise the member
/// makes out each component at the same chance.
fn inspect(
    inspect: On<Inspect>,
    mut attempts: Query<(&mut DeviceInteraction, &AttemptOn)>,
    guards: Query<(), With<Guarded>>,
    mut bodies: Query<Body>,
    busy: Busy,
    (content, setting): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<LockNews>,
    mut commands: Commands,
) {
    let Inspect { attempt, actor } = *inspect;
    let Ok((mut interaction, &AttemptOn(device))) = attempts.get_mut(attempt) else { return };
    let Work::Disarming(trap) = &mut interaction.work else { return };
    if !ready(trap, guards.contains(device), &content, &mut news) || busy.busy() {
        return;
    }
    let Some(skill) = bodies.get_mut(actor).ok().and_then(|body| skill(&body)) else { return };
    let chance = trap.inspection_chance(Some(skill), *setting);
    if rng.below(100) >= chance {
        commands.trigger(Sprung { attempt, by: Some(actor) });
        return;
    }
    commands.trigger(Practise { member: actor, skill: Skill::LocksAndTraps, usage: 1 });
    let seen = std::array::from_fn(|column| {
        if rng.below(100) >= chance {
            Observation::Unknown
        } else if trap_kind(trap.trap).has(column) {
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
}

/// Disarm one component: a part the trap lacks, or a failed attempt, sets
/// it off; with every part it has disarmed, the trap is.
fn disarm(
    disarm: On<Disarm>,
    mut attempts: Query<(&mut DeviceInteraction, &AttemptOn)>,
    guards: Query<(), With<Guarded>>,
    mut bodies: Query<Body>,
    busy: Busy,
    (content, setting): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<LockNews>,
    mut commands: Commands,
) {
    let Disarm { attempt, actor, component } = *disarm;
    let Ok((mut interaction, &AttemptOn(device))) = attempts.get_mut(attempt) else { return };
    let Work::Disarming(trap) = &mut interaction.work else { return };
    if !ready(trap, guards.contains(device), &content, &mut news) || busy.busy() {
        return;
    }
    let Some(skill) = bodies.get_mut(actor).ok().and_then(|body| skill(&body)) else { return };
    if component >= COMPONENTS || trap.disarmed[component] {
        return;
    }
    let kind = trap_kind(trap.trap);
    if !kind.has(component) || rng.below(100) >= trap.disarm_chance(Some(skill), *setting) {
        commands.trigger(Sprung { attempt, by: Some(actor) });
        return;
    }
    trap.disarmed[component] = true;
    commands.trigger(Practise { member: actor, skill: Skill::LocksAndTraps, usage: 1 });
    news.write(LockNews::ComponentDisarmed { member: actor, component });
    if (0..COMPONENTS).all(|part| !kind.has(part) || trap.disarmed[part]) {
        news.write(LockNews::Disarmed { trap: trap.trap });
        commands.trigger(Overcome { attempt });
    }
}

/// The party sets the trap off.
fn spring(
    spring: On<Spring>,
    attempts: Query<(&DeviceInteraction, &AttemptOn)>,
    guards: Query<(), With<Guarded>>,
    busy: Busy,
    content: Res<Catalogs>,
    mut news: MessageWriter<LockNews>,
    mut commands: Commands,
) {
    let Spring { attempt, actor } = *spring;
    let Ok((interaction, &AttemptOn(device))) = attempts.get(attempt) else { return };
    let Work::Disarming(trap) = &interaction.work else { return };
    if !busy.busy() && ready(trap, guards.contains(device), &content, &mut news) {
        commands.trigger(Sprung { attempt, by: actor });
    }
}

/// The pins a member's spell holds fall.
fn release_pins(
    release: On<ReleasePins>,
    mut attempts: Query<&mut DeviceInteraction>,
    members: Query<(), With<Calling>>,
    busy: Busy,
) {
    let Ok(mut interaction) = attempts.get_mut(release.attempt) else { return };
    let Work::Picking(picking) = &mut interaction.work else { return };
    if busy.busy() || !members.contains(release.actor) {
        return;
    }
    for pin in &mut picking.tumblers {
        if *pin == Tumbler::Sealed(release.actor) {
            *pin = Tumbler::Down;
        }
    }
}

/// The attempt is left: what was done to the lock or trap is forgotten.
fn leave(leave: On<Leave>, mut commands: Commands) {
    commands.entity(leave.attempt).try_despawn();
}

/// A cast aimed at the attempt under way, paid for and rolled as any other,
/// does what it is for: Knock-Knock reaches a few tumblers at random and
/// seals each it moves, or drops them on a backfire (the reaching is
/// [`workings::knock`]); Divine Trap helps those who look the trap over; any
/// other disarms the trap or sets it off on its caster. A fizzle does
/// nothing.
fn cast_at(
    worked: On<Worked>,
    mut attempts: Query<&mut DeviceInteraction>,
    mut rng: ResMut<GameRng>,
    (mut news, mut told): (MessageWriter<LockNews>, MessageWriter<MagicNews>),
    mut commands: Commands,
) {
    let Worked { caster, working, report, strength, marked } = worked.event();
    let Marked::Attempt(attempt) = *marked else { return };
    let (outcome, level) = (report.outcome, workings::level(*strength));
    told.write(MagicNews::Done(report.clone()));
    let Ok(mut interaction) = attempts.get_mut(attempt) else { return };
    if outcome == Outcome::Fizzled {
        return;
    }
    match &mut interaction.work {
        Work::Disarming(trap) if *working == Working::DivineTrap => trap.divine_power = level,
        Work::Picking(picking) => {
            let mut dropped = false;
            for reached in workings::knock(&mut rng, picking.tumblers.len(), level) {
                if reached.moves {
                    let pin = &mut picking.tumblers[reached.tumbler];
                    if outcome == Outcome::Backfired {
                        dropped |= pin.raised();
                        *pin = Tumbler::Down;
                    } else {
                        *pin = Tumbler::Sealed(*caster);
                    }
                }
            }
            news.write(LockNews::Tumbler { dropped });
            if picking.tumblers.iter().all(|pin| pin.raised()) {
                news.write(LockNews::Picked);
                commands.trigger(Overcome { attempt });
            }
        }
        Work::Disarming(trap) => {
            if workings::disarms(&mut rng, level, trap.difficulty, outcome) {
                news.write(LockNews::Disarmed { trap: trap.trap });
                commands.trigger(Overcome { attempt });
            } else {
                commands.trigger(Sprung { attempt, by: Some(*caster) });
            }
        }
        Work::Looting => {}
    }
}

/// A trap goes off: it discharges at the living of the party, and the
/// attempt ends. A discharge that destroys the trap leaves the device open.
fn sprung(
    sprung: On<Sprung>,
    attempts: Query<&DeviceInteraction>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    vitals: Query<&Vitals>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<LockNews>,
    mut commands: Commands,
) {
    let Sprung { attempt, by } = *sprung;
    let Ok(DeviceInteraction { work: Work::Disarming(trap) }) = attempts.get(attempt) else {
        return;
    };
    let living =
        roster.iter().filter(|&member| vitals.get(member).is_ok_and(|v| !v.is_dead())).collect();
    let (destroyed, discharge) = crate::game::magic::traps::discharge(
        (trap.trap, trap.difficulty),
        (living, by),
        &content,
        &mut rng,
    );
    commands.spawn((discharge, crate::game::magic::landed::Magical));
    news.write(LockNews::Sprung { trap: trap.trap, destroyed });
    if destroyed {
        commands.trigger(Overcome { attempt });
    } else {
        commands.entity(attempt).despawn();
    }
}

/// A lock or trap overcome no longer guards its device, which runs.
fn overcome(
    overcome: On<Overcome>,
    attempts: Query<&AttemptOn>,
    mut doors: Query<&mut DoorState>,
    mut activations: MessageWriter<super::ActivateDevice>,
    mut commands: Commands,
) {
    let Ok(&AttemptOn(device)) = attempts.get(overcome.attempt) else { return };
    commands.entity(overcome.attempt).despawn();
    commands.entity(device).remove::<Guarded>();
    if let Ok(mut door) = doors.get_mut(device) {
        door.locked = false;
    }
    activations.write(super::ActivateDevice { device, player: false });
}

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

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(pick)
        .add_observer(force)
        .add_observer(inspect)
        .add_observer(disarm)
        .add_observer(spring)
        .add_observer(release_pins)
        .add_observer(leave)
        .add_observer(cast_at)
        .add_observer(sprung)
        .add_observer(overcome)
        .add_observer(super::containers::take);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession, Race};
    use crate::game::characters::Character;

    /// A lock of two tumblers, picked by one skilled enough never to drop a
    /// pin: the first leaves it locked, the second opens it, and the device
    /// runs.
    #[test]
    fn a_lock_opens_once_every_tumbler_is_up_and_its_device_runs() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<GameRng>()
            .init_resource::<Difficulty>()
            .init_resource::<Catalogs>()
            .add_message::<LockNews>()
            .add_message::<MagicNews>()
            .add_message::<crate::game::world::ActivateDevice>()
            .add_plugins((plugin, crate::game::actors::plugin));
        let world = app.world_mut();
        let mut thief = Character::new("Thief", Race::Human, Gender::Male, Profession::Rogue);
        thief.training.0[Skill::LocksAndTraps] = 100;
        let member = world.spawn((Vitals::new(&thief), thief)).id();
        crate::game::party::roster::test_party(world, &[member]);
        world.run_schedule(crate::game::actors::Settle);
        let lock = Lock::Pickable { difficulty: 2 };
        let door = world.spawn(Guarded::new(lock)).id();
        let attempt = begin(world, door, Work::Picking(Picking::new(2)));
        world.flush();
        world.trigger(Pick { attempt, actor: member, tumbler: 0 });
        world.flush();
        assert!(world.get::<Guarded>(door).is_some(), "one pin up of two");
        world.trigger(Pick { attempt, actor: member, tumbler: 1 });
        world.flush();
        assert!(world.get::<Guarded>(door).is_none());
        assert!(world.get_entity(attempt).is_err(), "the attempt is over");
        let ran: Vec<_> =
            world.resource_mut::<Messages<crate::game::world::ActivateDevice>>().drain().collect();
        assert!(matches!(ran.as_slice(), [run] if run.device == door && !run.player));
    }
}
