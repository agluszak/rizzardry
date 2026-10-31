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

use crate::content::characters::{Attribute, Skill};
use crate::content::level::{Lock, TRAP_TYPES};
use crate::content::spells::SpellId;
use crate::game::GameplaySet;
use crate::game::characters::{Character, Vitals};
use crate::game::combat::Battle;
use crate::game::difficulty::Difficulty;
use crate::game::rng::GameRng;

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

/// The named attempt's work and device, if it is still the one under way and
/// its device still stands.
fn attempt(world: &World, attempt: Entity) -> Result<(&Work, Entity), Unable> {
    // A request may name an attempt that has since ended.
    let entity = world.get_entity(attempt).map_err(|_| Unable::NotNow)?;
    let interaction = entity.get::<DeviceInteraction>().ok_or(Unable::NotNow)?;
    Ok((&interaction.work, entity.get::<AttemptOn>().unwrap().0))
}

fn work_mut(world: &mut World, attempt: Entity) -> Mut<'_, Work> {
    world.get_mut::<DeviceInteraction>(attempt).unwrap().map_unchanged(|i| &mut i.work)
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
    character: &Character,
    vitals: &Vitals,
    tumblers: usize,
    setting: Difficulty,
) -> Option<u32> {
    let strength = character.stats(&vitals.enchantments).attribute(Attribute::Strength);
    if !vitals.can_act() || vitals.stamina < FORCE_STAMINA || strength <= FORCE_STRENGTH {
        return None;
    }
    let divisor = (setting as i32 - 1 + tumblers as i32).clamp(2, 8);
    Some((strength - FORCE_STRENGTH) >> (divisor - 2))
}

/// Face a device's lock or trap (retail `OpenLockInteraction`,
/// `OpenTrapInteraction`): the work an attempt on it starts with. Facing a
/// trap rolls its type the first time.
pub(crate) fn face(
    lock: Lock,
    guarded: &mut Guarded,
    now: u64,
    rng: &mut GameRng,
    news: &mut MessageWriter<LockNews>,
) -> Work {
    match lock {
        Lock::Pickable { difficulty } => {
            news.write(LockNews::Locked);
            Work::Picking(Picking::new(difficulty))
        }
        Lock::Trap { difficulty, .. } => {
            let remark = guarded.faced_at.is_none_or(|at| now.saturating_sub(at) > REMARK_AGAIN_MS);
            guarded.faced_at = Some(now);
            let trap = *guarded.trap.get_or_insert_with(|| roll_trap(difficulty, rng));
            news.write(LockNews::Trapped { remark });
            Work::Disarming(Disarming {
                difficulty,
                trap,
                disarmed: [false; COMPONENTS],
                divine_power: 0,
                observations: vec![],
            })
        }
    }
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

/// The party's work on device attempts: who works, the attempt, the
/// device's guard, the setting and the dice.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DeviceWork<'w, 's> {
    attempts: Query<'w, 's, (&'static mut DeviceInteraction, &'static AttemptOn)>,
    guards: Query<'w, 's, &'static mut Guarded>,
    members: Query<'w, 's, (&'static mut Character, &'static mut Vitals)>,
    difficulty: Res<'w, Difficulty>,
    rng: ResMut<'w, GameRng>,
    news: MessageWriter<'w, LockNews>,
    sprung: MessageWriter<'w, TrapSprung>,
    opened: MessageWriter<'w, DeviceOpened>,
    entering: Query<'w, 's, (), With<super::level::Destination>>,
    battles: Query<'w, 's, (), With<Battle>>,
}

impl DeviceWork<'_, '_> {
    /// The member's Locks & Traps level, when the party is free to work and
    /// the member able.
    fn worker(&self, actor: Entity) -> Result<Option<u32>, Unable> {
        if !self.entering.is_empty() || !self.battles.is_empty() {
            return Err(Unable::NotNow);
        }
        let (character, vitals) = self.members.get(actor).unwrap();
        if !vitals.can_act() {
            return Err(Unable::CannotAct);
        }
        Ok(lock_skill(character, vitals))
    }

    /// The attempt's work, if it is still under way.
    fn work(&mut self, on: Entity) -> Result<(Mut<'_, Work>, Entity), Unable> {
        let (interaction, device) = self.attempts.get_mut(on).map_err(|_| Unable::NotNow)?;
        Ok((interaction.map_unchanged(|i| &mut i.work), device.0))
    }

    /// The trap under way, if the device still stands guarded and its
    /// discharge can be resolved.
    fn trap(&mut self, on: Entity) -> Result<Disarming, Unable> {
        if !self.entering.is_empty() || !self.battles.is_empty() {
            return Err(Unable::NotNow);
        }
        let (work, device) = self.work(on)?;
        let Work::Disarming(trap) = &*work else { return Err(Unable::NotNow) };
        let trap = trap.clone();
        if !self.guards.contains(device) {
            return Err(Unable::NotNow);
        }
        if let Err(error) = crate::game::magic::traps::validate(trap.trap) {
            self.news.write(LockNews::UnsupportedTrap);
            return Err(error);
        }
        Ok(trap)
    }

    fn practise(&mut self, actor: Entity) {
        let (mut character, _) = self.members.get_mut(actor).unwrap();
        if character.practise(Skill::LocksAndTraps, 1, &mut self.rng) {
            self.news.write(LockNews::SkillImproved { member: actor });
        }
    }

    /// Raise a tumbler (retail `OnTumblerPicked`, `ResolvePick`). Every
    /// tumbler the actor already holds may drop, each by the actor's skill;
    /// when all are up the lock opens.
    fn pick(&mut self, on: Entity, actor: Entity, tumbler: usize) -> Result<(), Unable> {
        let skill = self.worker(actor)?.ok_or(Unable::Unskilled)?;
        let chance = (skill as i32 + 5 - 5 * *self.difficulty as i32).max(0) as u32;
        let (mut work, device) = self.work(on)?;
        let Work::Picking(picking) = &mut *work else { return Err(Unable::NotNow) };
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
        let mut dropped = false;
        for held in &mut picking.tumblers {
            if *held == Tumbler::Held(actor) && chance <= self.rng.below(100) {
                *held = Tumbler::Down;
                dropped = true;
            }
        }
        picking.tumblers[tumbler] = Tumbler::Held(actor);
        // Retail rolls even when the device's finite practice budget is
        // exhausted.
        if self.rng.below(5) == 0 {
            let mut guarded = self.guards.get_mut(device).unwrap();
            if guarded.practice_left > 0 {
                guarded.practice_left -= 1;
                self.practise(actor);
            }
        }
        self.news.write(LockNews::Tumbler { dropped });
        let open = picking.tumblers.iter().all(|pin| pin.raised());
        *self.work(on)?.0 = Work::Picking(picking);
        if open {
            self.news.write(LockNews::Picked);
            self.opened.write(DeviceOpened { on });
        }
        Ok(())
    }

    /// Force the lock (retail `AttemptForce`): it costs stamina whatever
    /// comes of it.
    fn force(&mut self, on: Entity, actor: Entity) -> Result<(), Unable> {
        self.worker(actor)?;
        let Work::Picking(picking) = &*self.work(on)?.0 else { return Err(Unable::NotNow) };
        let tumblers = picking.tumblers.len();
        let (character, mut vitals) = self.members.get_mut(actor).unwrap();
        let chance =
            force_chance(&character, &vitals, tumblers, *self.difficulty).ok_or(Unable::TooWeak)?;
        vitals.spend_stamina(FORCE_STAMINA);
        if self.rng.below(100) < chance {
            self.news.write(LockNews::Forced { member: actor });
            self.opened.write(DeviceOpened { on });
        } else {
            self.news.write(LockNews::NotForced { member: actor });
        }
        Ok(())
    }

    /// Inspect the trap: a failed inspection springs it; otherwise the
    /// member learns what each component is, by a chance practice may
    /// raise for this very inspection.
    fn inspect(&mut self, on: Entity, actor: Entity) -> Result<(), Unable> {
        let trap = self.trap(on)?;
        let skill = self.worker(actor)?.ok_or(Unable::Unskilled)?;
        let setting = *self.difficulty;
        if self.rng.below(100) >= trap.inspection_chance(Some(skill), setting) {
            self.sprung.write(TrapSprung { on, selected: Some(actor) });
            return Ok(());
        }
        self.practise(actor);
        let chance = trap.inspection_chance(self.worker(actor)?, setting);
        let seen = std::array::from_fn(|column| {
            if self.rng.below(100) >= chance {
                Observation::Unknown
            } else if TRAP_COMPONENTS[usize::from(trap.trap)][column] {
                Observation::Present
            } else {
                Observation::Absent
            }
        });
        let (mut work, _) = self.work(on)?;
        let Work::Disarming(trap) = &mut *work else { unreachable!("checked by trap") };
        match trap.observations.iter_mut().find(|(member, _)| *member == actor) {
            Some((_, previous)) => *previous = seen,
            None => trap.observations.push((actor, seen)),
        }
        self.news.write(LockNews::Inspected { member: actor });
        Ok(())
    }

    /// Disarm one component: one the trap lacks, or a failed try, springs
    /// it; the last of its components disarms the trap.
    fn disarm(&mut self, on: Entity, actor: Entity, component: usize) -> Result<(), Unable> {
        let trap = self.trap(on)?;
        if component >= COMPONENTS || trap.disarmed[component] {
            return Err(Unable::Done);
        }
        let skill = self.worker(actor)?.ok_or(Unable::Unskilled)?;
        let chance = trap.disarm_chance(Some(skill), *self.difficulty);
        let components = TRAP_COMPONENTS[usize::from(trap.trap)];
        if !components[component] || self.rng.below(100) >= chance {
            self.sprung.write(TrapSprung { on, selected: Some(actor) });
            return Ok(());
        }
        let (mut work, _) = self.work(on)?;
        let Work::Disarming(trap) = &mut *work else { unreachable!("checked by trap") };
        trap.disarmed[component] = true;
        let done = components.iter().zip(trap.disarmed).all(|(&needed, done)| !needed || done);
        let kind = trap.trap;
        self.practise(actor);
        self.news.write(LockNews::ComponentDisarmed { member: actor, component });
        if done {
            self.news.write(LockNews::Disarmed { trap: kind });
            self.opened.write(DeviceOpened { on });
        }
        Ok(())
    }

    /// Opening either selector releases this actor's spell-held pins even
    /// if the uncommitted selector is cancelled, as retail
    /// ReleaseOwnedTumblers does.
    fn prepare_magic(&mut self, on: Entity, actor: Entity) -> Result<(), Unable> {
        self.worker(actor)?;
        let (mut work, _) = self.work(on)?;
        match &mut *work {
            Work::Picking(picking) => {
                for pin in &mut picking.tumblers {
                    if *pin == Tumbler::Sealed(actor) {
                        *pin = Tumbler::Down;
                    }
                }
            }
            Work::Disarming(_) => {}
            Work::Looting => return Err(Unable::NotNow),
        }
        Ok(())
    }
}

/// A trap goes off on the attempt under way, at the member who set it off,
/// if any.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrapSprung {
    pub on: Entity,
    pub selected: Option<Entity>,
}

/// The lock or trap is overcome.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceOpened {
    pub on: Entity,
}

/// Carry out the player's device requests in order. A refused request
/// changes nothing: retail offers only the controls that can work.
pub fn work_devices(
    mut requests: MessageReader<DeviceRequest>,
    mut work: DeviceWork,
    mut commands: Commands,
) {
    for request in requests.read() {
        let on = request.on;
        let result = match request.act {
            DeviceAct::Pick { actor, tumbler } => work.pick(on, actor, tumbler),
            DeviceAct::Force { actor } => work.force(on, actor),
            DeviceAct::Inspect { actor } => work.inspect(on, actor),
            DeviceAct::Disarm { actor, component } => work.disarm(on, actor, component),
            DeviceAct::Spring { actor } => work.trap(on).map(|_| {
                work.sprung.write(TrapSprung { on, selected: actor });
            }),
            DeviceAct::PrepareMagic { actor } => work.prepare_magic(on, actor),
            DeviceAct::Leave => {
                if work.attempts.contains(on) {
                    commands.entity(on).despawn();
                }
                Ok(())
            }
        };
        if let Err(why) = result {
            debug!("{request:?} refused: {why:?}");
        }
    }
}

/// A sprung trap discharges its spell at the party (retail picks victims
/// at random, keeping the member who set it off). One it destroys opens the
/// device; otherwise the party leaves it as it is.
pub fn spring_traps(
    mut sprung: MessageReader<TrapSprung>,
    attempts: Query<&DeviceInteraction>,
    mut discharge: crate::game::magic::traps::Discharge,
    mut news: MessageWriter<LockNews>,
    mut opened: MessageWriter<DeviceOpened>,
    mut commands: Commands,
) {
    for &TrapSprung { on, selected } in sprung.read() {
        let Ok(DeviceInteraction { work: Work::Disarming(trap) }) = attempts.get(on) else {
            continue;
        };
        let destroyed = discharge.fire(trap.trap, trap.difficulty, selected);
        news.write(LockNews::Sprung { trap: trap.trap, destroyed });
        if destroyed {
            opened.write(DeviceOpened { on });
        } else {
            commands.entity(on).despawn();
        }
    }
}

/// The lock or trap is overcome: the interaction ends and the device runs
/// (retail `CompleteItemInteraction` then `Run`).
pub fn open_devices(
    mut opened: MessageReader<DeviceOpened>,
    attempts: Query<&AttemptOn>,
    mut devices: super::triggers::Devices,
    geometry: Res<Assets<crate::content::level::LevelDefinition>>,
    mut commands: Commands,
) {
    for &DeviceOpened { on } in opened.read() {
        let Ok(&AttemptOn(device)) = attempts.get(on) else { continue };
        commands.entity(on).despawn();
        devices.open(&geometry, device);
    }
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

/// Per mille that Knock-Knock moves each tumbler it reaches, by power.
const KNOCK_CHANCE: [u32; 8] = [0, 344, 459, 516, 550, 573, 589, 602];

fn knock(world: &mut World, on: Entity, actor: Entity, power: u8, backfire: bool) {
    let (done, dropped) = world.resource_scope(|world, mut rng: Mut<GameRng>| {
        let mut work = work_mut(world, on);
        let Work::Picking(picking) = &mut *work else { unreachable!("a lock's spell") };
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
        world.write_message(DeviceOpened { on });
    }
}

/// One exclusive operation: target preflight, shared cast/payment/practice,
/// and the device effect, which springs or opens the device by the same
/// messages as the party's own work.
pub fn cast_device(
    world: &mut World,
    actor: Entity,
    on: Entity,
    source: crate::game::magic::casting::Source,
) -> Result<(), crate::game::magic::casting::Refusal> {
    use crate::game::magic::{
        Outcome,
        casting::{MagicNews, Refusal, UseContext},
    };
    if crate::game::world::party_occupied(world) || !world.get::<Vitals>(actor).unwrap().can_act() {
        return Err(Refusal::CannotAct);
    }
    let (work, device) = attempt(world, on).map_err(|_| Refusal::NoTarget)?;
    if world.get::<Guarded>(device).is_none() {
        return Err(Refusal::NoTarget);
    }
    let (context, trap) = match work {
        Work::Picking(_) => (UseContext::Lock, None),
        Work::Disarming(trap) => (UseContext::Trap, Some((trap.trap, trap.difficulty))),
        Work::Looting => return Err(Refusal::NoTarget),
    };
    if let Some((kind, _)) = trap
        && crate::game::magic::traps::validate(kind).is_err()
    {
        world.write_message(LockNews::UnsupportedTrap);
        return Err(Refusal::Unsupported);
    }
    let (report, strength) =
        crate::game::magic::casting::cast_source(world, actor, source, context)?;
    let outcome = report.outcome;
    let spell = report.spell;
    world.write_message(MagicNews::Done(report));
    if outcome == Outcome::Fizzled {
        return Ok(());
    }
    let power =
        (u32::from(strength.power) + u32::from(strength.power) * strength.boost / 100).min(7) as u8;
    match trap {
        None => knock(world, on, actor, power, outcome == Outcome::Backfired),
        Some(_) if spell == SpellId::DIVINE_TRAP => {
            let Work::Disarming(trap) = &mut *work_mut(world, on) else { unreachable!() };
            trap.divine_power = power;
        }
        Some((kind, difficulty)) => {
            let chance = if outcome == Outcome::Backfired {
                0
            } else {
                i32::from(power) * 5 + 50 - i32::from(difficulty) * 6
            };
            if (world.resource_mut::<GameRng>().below(100) as i32) < chance {
                world.write_message(LockNews::Disarmed { trap: kind });
                world.write_message(DeviceOpened { on });
            } else {
                world.write_message(TrapSprung { on, selected: Some(actor) });
            }
        }
    }
    Ok(())
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
    Leave,
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<DeviceRequest>()
        .add_message::<TrapSprung>()
        .add_message::<DeviceOpened>()
        .add_message::<super::containers::TakeFromContainer>()
        .add_systems(
            Update,
            (work_devices, spring_traps, open_devices, super::containers::take_from_containers)
                .chain()
                .after(GameplaySet::Burden)
                .before(GameplaySet::Combat),
        );
}
