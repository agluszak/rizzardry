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
//! Not ported yet: skill practice from picking and disarming, the Knock-Knock
//! and Divine Trap spells and tools used on a device, and what a sprung
//! trap's spell does. Attempts that could spring a trap are refused before
//! spending RNG or changing the device until those consequences are implemented.

use bevy::prelude::*;

use super::triggers::{Device, DoorState};
use crate::content::characters::{Attribute, Skill};
use crate::content::level::{Lock, TRAP_TYPES};
use crate::game::characters::{Character, Vitals};
use crate::game::difficulty::Difficulty;
use crate::game::party::roster::PartyMember;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

/// The parts a trap is built from; each type uses a few of them.
pub const COMPONENTS: usize = 8;

/// Each trap type's own grade (retail `g_trap_difficulty`).
const TRAP_GRADE: [i32; TRAP_TYPES] = [1, 1, 2, 2, 3, 3, 3, 4, 5, 5, 5, 6, 6, 7, 7];

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
}

impl Guarded {
    pub fn new(lock: Lock) -> Self {
        match lock {
            Lock::Trap { kind, .. } => Guarded { trap: kind, faced_at: None },
            Lock::Pickable { .. } => Guarded::default(),
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

/// A lock being picked: each tumbler stays up while the member who set it
/// keeps working. A different member's first pick drops them.
#[derive(Debug, Clone, PartialEq)]
pub struct Picking {
    pub difficulty: u8,
    pub tumblers: Vec<Option<Entity>>,
    picker: Option<Entity>,
}

impl Picking {
    fn new(difficulty: u8) -> Self {
        let tumblers = usize::from(difficulty).clamp(2, 8);
        Picking { difficulty, tumblers: vec![None; tumblers], picker: None }
    }
}

/// A trap being disarmed.
#[derive(Debug, Clone, PartialEq)]
pub struct Disarming {
    pub difficulty: u8,
    pub trap: u8,
    pub disarmed: [bool; COMPONENTS],
    /// What each member's inspection made of each component.
    pub observations: Vec<(Entity, [Observation; COMPONENTS])>,
}

impl Disarming {
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
}

/// Why an attempt was not made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unable {
    /// No such interaction, or not this kind of work.
    NotNow,
    NotMember,
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
    (crate::game::camp::can_act(vitals)
        && (character.skill_available(Skill::LocksAndTraps) || level > 0))
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
    if !crate::game::camp::can_act(vitals)
        || vitals.stamina < FORCE_STAMINA
        || strength <= FORCE_STRENGTH
    {
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
    if world
        .query_filtered::<Entity, With<crate::game::combat::Battle>>()
        .iter(world)
        .next()
        .is_some()
    {
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
    if !world.query::<&super::level::Destination>().is_empty(world)
        || !world.query::<&crate::game::combat::Battle>().is_empty(world)
    {
        return Err(Unable::NotNow);
    }
    if world.get::<PartyMember>(actor).is_none() {
        return Err(Unable::NotMember);
    }
    let entity = world.get_entity(actor).map_err(|_| Unable::NotMember)?;
    Ok((
        entity.get::<Character>().ok_or(Unable::NotMember)?,
        entity.get().ok_or(Unable::NotMember)?,
    ))
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
    if picking.tumblers.get(tumbler).is_none_or(Option::is_some) {
        return Err(Unable::Done);
    }
    let mut picking = picking.clone();
    if picking.picker != Some(actor) {
        for held in &mut picking.tumblers {
            if held.is_some() && *held == picking.picker {
                *held = None;
            }
        }
        picking.picker = Some(actor);
    }
    let chance = (skill as i32 + 5 - 5 * *world.resource::<Difficulty>() as i32).max(0) as u32;
    let mut dropped = false;
    for held in &mut picking.tumblers {
        if *held == Some(actor) && chance <= rng(world).below(100) {
            *held = None;
            dropped = true;
        }
    }
    picking.tumblers[tumbler] = Some(actor);
    world.write_message(LockNews::Tumbler { dropped });
    let open = picking.tumblers.iter().all(Option::is_some);
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
    let (character, vitals) = member(world, actor)?;
    let Some(DeviceInteraction { work: Work::Picking(picking), .. }) =
        world.get_resource::<DeviceInteraction>()
    else {
        return Err(Unable::NotNow);
    };
    let chance =
        force_chance(character, vitals, picking.tumblers.len(), *world.resource::<Difficulty>())
            .ok_or(Unable::TooWeak)?;
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

/// Inspecting or disarming can discharge a trap. Until that consequence is
/// implemented, none of these routes may roll an attempt or open the device.
pub fn inspect_trap(world: &mut World, actor: Entity) -> Result<(), Unable> {
    member(world, actor)?;
    disarming(world)?;
    unsupported_trap(world)
}

pub fn disarm_component(world: &mut World, actor: Entity, _component: usize) -> Result<(), Unable> {
    member(world, actor)?;
    disarming(world)?;
    unsupported_trap(world)
}

pub fn spring_trap(world: &mut World) -> Result<(), Unable> {
    disarming(world)?;
    unsupported_trap(world)
}

fn unsupported_trap(world: &mut World) -> Result<(), Unable> {
    world.write_message(LockNews::UnsupportedTrap);
    Err(Unable::UnsupportedTrap)
}

/// The lock or trap is overcome: the interaction ends and the device runs.
fn complete(world: &mut World) {
    let interaction = world.remove_resource::<DeviceInteraction>().unwrap();
    finish(world, interaction.device);
}

/// Retail `CompleteItemInteraction` then `Run`.
fn finish(world: &mut World, device: Entity) {
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

/// An interaction ends with its device, an accepted entry, or a fight.
pub(crate) fn end_lost_interaction(
    mut commands: Commands,
    interaction: Option<Res<DeviceInteraction>>,
    devices: Query<(), With<Device>>,
    entering: Query<(), With<super::level::Destination>>,
    battle: Query<(), With<crate::game::combat::Battle>>,
) {
    if interaction
        .is_some_and(|i| !devices.contains(i.device) || !battle.is_empty() || !entering.is_empty())
    {
        commands.remove_resource::<DeviceInteraction>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Gender, Profession, Race};

    #[test]
    fn difficulty_changes_both_displayed_and_authoritative_lock_chances() {
        let mut character =
            Character::new("Fighter", Race::Human, Gender::Male, Profession::Fighter);
        character.attributes[Attribute::Strength] = 85;
        let vitals = Vitals::new(&character);
        assert!(
            force_chance(&character, &vitals, 4, Difficulty::Novice)
                > force_chance(&character, &vitals, 4, Difficulty::Expert)
        );
        assert!(
            trap_chance(Some(25), 8, Difficulty::Novice)
                > trap_chance(Some(25), 8, Difficulty::Expert)
        );
    }
}
