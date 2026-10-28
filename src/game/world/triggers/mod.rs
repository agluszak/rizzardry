//! Live devices and synchronous trigger operations. Definitions stay in the
//! level asset; runtime links and mutable state belong to device entities.
//!
//! An action commits its sound, ordered recipients, location variable, message
//! and fired flag synchronously. A relay also runs its recipients as its action,
//! preserving retail's double execution when it is marked to fire on commit.

mod records;
#[cfg(test)]
pub(crate) mod tests;
mod walk;

pub use records::{DeviceRecords, TriggerMemory, spawn_devices};
pub(crate) use walk::walk;

use std::collections::HashMap;

use bevy::ecs::world::EntityMut;
use bevy::prelude::*;

use super::containers::{Container, ContainerUse, use_container};
use super::interaction::{Guarded, Work};
use super::level::{Level, LevelOwned};
use super::props::{Prop, PropMotion};
use super::scripts::BellButton;
use crate::content::items::Dice;
use crate::content::level::{
    Fountain, LevelDefinition, Restores, StateChange, TriggerAction, TriggerDefinition, TriggerUse,
    VariableName,
};
use crate::game::characters::{Character, Vitals};
use crate::game::data::GameData;
use crate::game::inventory::PartyInventory;
use crate::game::party::DamageApplied;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

const MAX_DEPTH: usize = 16;

/// A device's immutable definition and links, resolved once per visit.
/// A prop-backed device lives on that prop; positioned devices stand alone.
#[derive(Component, Debug)]
pub struct Device {
    pub definition: usize,
    pub recipients: Vec<Entity>,
}

/// Authored traversal order, resolved once per visit; not an owner of device state.
#[derive(Component, Debug, Default)]
pub struct PositionedDevices(Vec<Entity>);

/// Only level-wide variables and visit state live on the level.
#[derive(Component, Debug, Default)]
pub struct LevelState {
    variables: HashMap<VariableName, bool>,
    previous_eye: Option<Vec3>,
}

impl LevelState {
    /// Rebase walk-in detection after teleporting, without firing destination
    /// triggers or treating an unobserved level as already visited.
    pub(crate) fn arrive(&mut self, eye: Vec3) {
        if self.previous_eye.is_some() {
            self.previous_eye = Some(eye);
        }
    }

    fn variable(&self, name: &VariableName) -> bool {
        self.variables.get(name).copied().unwrap_or(false)
    }

    fn set_variable(&mut self, name: &VariableName, value: bool) {
        if let Some(stored) = self.variables.get_mut(name) {
            *stored = value;
        } else {
            self.variables.insert(name.clone(), value);
        }
    }
}

#[derive(Component, Clone, Debug)]
pub struct TriggerState {
    pub on: bool,
    pub fired: bool,
    use_alternate: bool,
    awaits_animation: bool,
}

impl Default for TriggerState {
    fn default() -> Self {
        Self { on: true, fired: false, use_alternate: false, awaits_animation: false }
    }
}

/// Logical door state is independent of its prop's animation progress.
#[derive(Component, Clone, Debug)]
pub struct DoorState {
    pub open: bool,
    pub locked: bool,
}

#[derive(Component, Clone, Debug, Default)]
pub struct DialState {
    pub position: usize,
}

#[derive(Component, Clone, Debug, Default)]
pub struct FountainState {
    used: Option<FountainUses>,
}

#[derive(Clone, Copy, Debug)]
struct FountainUses {
    left: Option<u32>,
    refills_at: Option<u64>,
}

/// Hurt each living member by `dice`, in roster order.
fn damage_party(
    In(dice): In<Dice>,
    mut commands: Commands,
    roster: Single<&PartyRoster>,
    mut members: Query<&mut Vitals>,
    mut rng: ResMut<GameRng>,
    mut formation: Single<&mut crate::game::party::formation::Formation>,
    mut damage: MessageWriter<DamageApplied>,
) {
    for entity in roster.iter() {
        let Ok(mut member) = members.get_mut(entity) else { continue };
        if member.is_dead() {
            continue;
        }
        let rolled = rng.roll(dice);
        if rolled > 0 {
            crate::game::party::damage_member(
                entity,
                &mut member,
                &mut formation,
                rolled,
                &mut damage,
            );
        }
    }
    commands.run_system_cached(crate::game::combat::settle_defeat);
}

fn drink_fountain(
    In((entity, fountain, at)): In<(Entity, Fountain, Option<Vec3>)>,
    mut fountains: Query<&mut FountainState>,
    clock: Res<GameClock>,
    roster: Single<&PartyRoster>,
    mut members: Query<(&Character, &mut Vitals)>,
    mut outcomes: MessageWriter<TriggerEffect>,
) -> bool {
    let mut state = fountains.get_mut(entity).unwrap();
    let now = clock.ms;
    // A dry fountain is assessed without marking it changed.
    if state.used.is_some_and(|s| s.left == Some(0) && !s.refills_at.is_some_and(|at| at <= now)) {
        return false;
    }
    let state = state.used.get_or_insert(FountainUses { left: fountain.uses, refills_at: None });
    if state.refills_at.is_some_and(|at| at <= now) {
        *state = FountainUses { left: fountain.uses, refills_at: None };
    }
    if let Some(left) = &mut state.left {
        if *left == 0 {
            return false;
        }
        *left -= 1;
        if state.refills_at.is_none() {
            state.refills_at = fountain.refills_after.map(|after| now + after);
        }
    }
    for (profile, mut member) in members.iter_many_unique_mut(roster.iter()).matched() {
        if member.is_dead()
            || match fountain.restores {
                Restores::Health => member.hp >= profile.stats(&member.enchantments).max_hp(),
                Restores::Stamina => {
                    member.stamina >= profile.stats(&member.enchantments).max_stamina()
                        && !member.conditions.contains(&(
                            crate::game::characters::Condition::Unconscious,
                            crate::game::characters::Lasting::Indefinite,
                        ))
                }
            }
        {
            continue;
        }
        match fountain.restores {
            Restores::Health => {
                let maximum = profile.stats(&member.enchantments).max_hp();
                member.heal(fountain.amount, maximum)
            }
            Restores::Stamina => {
                let maximum = profile.stats(&member.enchantments).max_stamina();
                member.restore_stamina(fountain.amount, maximum)
            }
        }
    }
    outcomes.write(TriggerEffect::FountainUsed { at });
    true
}

/// Something presentation shows after triggers ran.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TriggerEffect {
    /// A retail sound file, at a place or heard everywhere.
    Sound {
        path: String,
        volume: f32,
        at: Option<Vec3>,
    },
    Shake {
        seconds: f32,
        intensity: f32,
        fade_out: bool,
    },
    /// A message from the level's message file.
    Message {
        source: Option<Handle<crate::content::ui::TextTable>>,
        id: i32,
    },
    /// The party used something and nothing came of it.
    NothingHappened,
    Gold(u32),
    FountainUsed {
        at: Option<Vec3>,
    },
    BellRang(usize),
    Locked,
}

/// Moving the party somewhere, which the party module carries out.
#[derive(Clone, Debug)]
pub enum PartyMove {
    Teleport(crate::content::level::Arrival),
    /// Fly the party's eye along a camera path, by name (ladders, climbs).
    Fly(usize),
}

/// On a level: a move one of its devices requested, carried out after the
/// trigger step. A later request replaces it; it goes with its level.
#[derive(Component, Clone, Debug)]
pub struct PendingPartyMove(pub PartyMove);

/// Complete a device request synchronously. Player use revalidates current reach;
/// linked/script activation does not apply the player's reach/usable gate.
pub fn activate(world: &mut World, entity: Entity, player: bool) -> bool {
    if player && !world.run_system_cached(crate::game::screen::party_free).unwrap_or(false) {
        return false;
    }
    if world.get::<Device>(entity).is_none() {
        return false;
    }
    let Some(&LevelOwned(level)) = world.get::<LevelOwned>(entity) else { return false };
    let Some(level) = world.get::<Level>(level) else { return false };
    let handle = level.definition.clone();
    // Borrow immutable definitions for the entire synchronous operation. Nested
    // links use these same definitions, without cloning a graph or a definition.
    world.resource_scope(|world, geometry: Mut<Assets<LevelDefinition>>| {
        let Some(geometry) = geometry.get(&handle) else { return false };
        let definition = &geometry.triggers[world.get::<Device>(entity).unwrap().definition];
        if (player
            && (!usable(world, entity, definition)
                || player_distance(world, entity).is_none_or(|distance| {
                    !distance.is_finite()
                        || !(definition.range.0..=definition.range.1).contains(&distance)
                })))
            || !ready(world, entity)
        {
            return false;
        }
        let ran = execute(world, geometry, entity, 0);
        if player && ran == Ran::Declined {
            world.write_message(TriggerEffect::NothingHappened);
        }
        ran == Ran::Committed
    })
}

/// Reach uses the current gameplay geometry, independent of the last pointer hit.
/// Colliderless props fall back to their authored centre.
fn player_distance(world: &mut World, entity: Entity) -> Option<f32> {
    use crate::game::party::body::PartyBody;
    use avian3d::prelude::{Collider, Position};
    let eye = world
        .query_filtered::<&Position, With<PartyBody>>()
        .single(world)
        .ok()
        .map(PartyBody::eye)?;
    let prop = world.get::<Prop>(entity)?;
    let distance = world
        .get::<Children>(entity)
        .into_iter()
        .flatten()
        .filter_map(|child| {
            world.get::<super::props::PropPart>(*child)?;
            let at = world.get::<Transform>(*child)?;
            let collider = world.get::<Collider>(*child)?;
            Some(collider.distance_to_point(at.translation, at.rotation, eye, true))
        })
        .min_by(f32::total_cmp);
    Some(distance.unwrap_or_else(|| eye.distance(prop.centre)))
}

/// How running a device went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ran {
    Committed,
    /// It did not run, and nothing came of it.
    Declined,
    /// It did not run, but something answered: its lock or trap, or an item
    /// put into it (retail `g_trigger_feedback`).
    Answered,
}

/// The use a device's state selects.
enum Selected<'a> {
    /// `keyed` when a key just unlocked it, which overcomes its lock too
    /// (retail marks the lock completed).
    Use { chosen: &'a TriggerUse, keyed: bool },
    /// A lock or trap must be overcome first.
    Guarded,
}

fn usable(world: &World, entity: Entity, definition: &TriggerDefinition) -> bool {
    world
        .get::<TriggerState>(entity)
        .is_some_and(|state| definition.usable && !(definition.once && state.fired))
}

fn ready(world: &World, entity: Entity) -> bool {
    if world.get::<Container>(entity).is_none() {
        return true;
    }
    GameData::items(world).is_some() && GameData::loot(world).is_some()
}

/// Choose the use a run performs (retail `Trigger::SelectAction`). As in
/// retail, choosing is itself a commitment: a matching key unlocks the door
/// and overcomes its lock, and the initial/alternate sequence advances, even
/// if the chosen action then declines (retail `Run` returns without undoing
/// either). Fired flags, location variables, sounds, links and messages
/// wait for the action's commit.
fn select_use<'a>(
    mut actor: EntityMut,
    variables: &LevelState,
    definition: &'a TriggerDefinition,
    offered: Option<crate::content::items::ItemId>,
) -> Option<Selected<'a>> {
    let state = actor.get::<TriggerState>()?;
    let motion = actor.get::<PropMotion>();
    let running = state.awaits_animation && motion.is_some_and(|m| m.frame != m.target);
    if !state.on
        || (definition.once && state.fired)
        || (running && !matches!(definition.initial.action, TriggerAction::Shake { .. }))
    {
        return None;
    }
    if !definition.required_states.iter().all(|name| variables.variable(name)) {
        return definition.fallback.as_ref().map(|chosen| Selected::Use { chosen, keyed: false });
    }
    let mut keyed = false;
    if let Some(mut door) = actor.get_mut::<DoorState>()
        && door.locked
        && let Some(key) = definition.door.and_then(|d| d.key)
    {
        if offered == Some(key) {
            door.locked = false;
            keyed = true;
        } else {
            return definition
                .fallback
                .as_ref()
                .map(|chosen| Selected::Use { chosen, keyed: false });
        }
    }
    if !keyed && actor.contains::<Guarded>() {
        return Some(Selected::Guarded);
    }
    let mut state = actor.get_mut::<TriggerState>()?;
    let (chosen, next) = match &definition.alternate {
        Some(alternate) if state.use_alternate => (alternate, !definition.alternate_toggles),
        alternate => (&definition.initial, alternate.is_some()),
    };
    if state.use_alternate != next {
        state.use_alternate = next;
    }
    Some(Selected::Use { chosen, keyed })
}

fn operate_door(mut device: EntityMut, open_only: bool) -> Result<bool, TriggerEffect> {
    if device.get::<DoorState>().is_some_and(|door| door.locked) {
        return Err(TriggerEffect::Locked);
    }
    let Ok((mut door, prop, mut motion, mut trigger)) =
        device.get_components_mut::<(&mut DoorState, &Prop, &mut PropMotion, &mut TriggerState)>()
    else {
        return Ok(false);
    };
    let open = open_only || !door.open;
    if motion.frame != motion.target || door.open == open {
        return Ok(false);
    }
    door.open = open;
    motion.target = if open { prop.last_frame as f32 } else { 0.0 };
    trigger.awaits_animation = true;
    Ok(true)
}

fn animate_device(mut device: EntityMut, animation: crate::content::level::Animation) {
    let (prop, motion) = device.get_components_mut::<(&Prop, &mut PropMotion)>().unwrap();
    PropMotion::activate(motion, prop, animation);
    if let Some(mut door) = device.get_mut::<DoorState>() {
        door.open = !door.open;
    }
    device.get_mut::<TriggerState>().unwrap().awaits_animation = true;
}

fn turn_dial(
    mut actor: EntityMut,
    mut level: Mut<LevelState>,
    variable: Option<&VariableName>,
) -> bool {
    let Ok((mut dial, prop, mut motion)) =
        actor.get_components_mut::<(&mut DialState, &Prop, &mut PropMotion)>()
    else {
        return false;
    };
    let old = dial.position;
    let next = if prop.slots.len() < 3 {
        0
    } else {
        let next = (old + 1) % (prop.slots.len() - 1);
        motion.frame = f32::from(prop.slots[next]);
        motion.target = f32::from(prop.slots[next + 1]);
        next
    };
    if old != next {
        dial.position = next;
    }
    if let Some(name) = variable {
        level.set_variable(&name.numbered(old), false);
        level.set_variable(&name.numbered(next), true);
    }
    true
}

fn sound_position(world: &World, entity: Entity, definition: &TriggerDefinition) -> Option<Vec3> {
    definition.place.or_else(|| world.get::<Prop>(entity).map(|prop| prop.centre))
}

fn play_authored_sound(world: &mut World, entity: Entity, definition: &TriggerDefinition) {
    if let Some(sound) = &definition.sound {
        world.write_message(TriggerEffect::Sound {
            path: sound.path.clone(),
            volume: sound.volume,
            at: sound_position(world, entity, definition),
        });
    }
}

fn execute_links(world: &mut World, geometry: &LevelDefinition, entity: Entity, depth: usize) {
    let count = world.get::<Device>(entity).unwrap().recipients.len();
    for index in 0..count {
        let recipient = world.get::<Device>(entity).unwrap().recipients[index];
        execute(world, geometry, recipient, depth + 1);
    }
}

/// Whether a device this one runs, or one those run, is mid-animation.
fn links_moving(world: &World, entity: Entity, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && world.get::<Device>(entity).is_some_and(|device| {
            device.recipients.iter().any(|&recipient| {
                world.get::<PropMotion>(recipient).is_some_and(|m| m.frame != m.target)
                    || links_moving(world, recipient, depth + 1)
            })
        })
}

fn execute(world: &mut World, geometry: &LevelDefinition, entity: Entity, depth: usize) -> Ran {
    if !ready(world, entity) {
        return Ran::Declined;
    }
    let Some(device) = world.get::<Device>(entity) else { return Ran::Declined };
    let Some(&LevelOwned(level)) = world.get::<LevelOwned>(entity) else { return Ran::Declined };
    let definition = &geometry.triggers[device.definition];
    if depth > MAX_DEPTH {
        warn!("trigger {} recurses too deeply", definition.name);
        return Ran::Declined;
    }
    // A door waits while any device it runs is still moving (retail
    // `CanRunLinkedTriggers`), before a key is tried.
    if definition.door.is_some() && links_moving(world, entity, 0) {
        return Ran::Declined;
    }
    let party = crate::game::party::roster::the_party(world);
    let offered = party
        .and_then(|party| world.get::<PartyInventory>(party))
        .and_then(|p| p.hand.held.as_ref())
        .map(|h| h.item);
    let chosen = {
        let [actor, owner] = world.get_entity_mut([entity, level]).unwrap();
        select_use(actor, owner.get::<LevelState>().unwrap(), definition, offered)
    };
    let chosen = match chosen {
        None => return Ran::Declined,
        Some(Selected::Guarded) => {
            if let Some(lock) = definition.lock {
                super::interaction::confront(world, entity, lock);
            }
            return Ran::Answered;
        }
        Some(Selected::Use { chosen, keyed }) => {
            if keyed {
                world.entity_mut(entity).remove::<Guarded>();
            }
            chosen
        }
    };
    // A level callback runs once the action is selected, before it executes
    // (retail `Trigger::Run`).
    if let Some(bell) = world.get::<BellButton>(entity).copied() {
        let [button, dial] = world.get_entity_mut([entity, bell.dial]).unwrap();
        let outcome = super::scripts::ring_bell(button, dial);
        world.write_message(outcome);
    }
    let changes_state = !matches!(chosen.action, TriggerAction::Dial);
    match &chosen.action {
        TriggerAction::Nothing => {}
        action @ (TriggerAction::OpenDoor | TriggerAction::ToggleDoor) => {
            match operate_door(
                world.entity_mut(entity).into_mutable(),
                *action == TriggerAction::OpenDoor,
            ) {
                Ok(true) => {}
                Ok(false) => return Ran::Declined,
                Err(outcome) => {
                    world.write_message(outcome);
                    return Ran::Declined;
                }
            }
        }
        TriggerAction::Animate(animation) => {
            if world.get::<Prop>(entity).is_none() || world.get::<PropMotion>(entity).is_none() {
                return Ran::Declined;
            }
            if world.get::<Container>(entity).is_some() {
                // Without a party there is nobody to use it.
                match world.run_system_cached_with(use_container, entity) {
                    Err(_) | Ok(ContainerUse::Busy) => return Ran::Declined,
                    Ok(ContainerUse::Deposited) => return Ran::Answered,
                    Ok(ContainerUse::Toggle) => {}
                    Ok(ContainerUse::Look { open }) => {
                        super::interaction::begin(world, entity, Work::Looting);
                        if open {
                            return Ran::Answered;
                        }
                    }
                }
            }
            animate_device(world.entity_mut(entity).into_mutable(), *animation);
        }
        TriggerAction::Sound => {
            play_authored_sound(world, entity, definition);
            return Ran::Declined;
        }
        TriggerAction::Relay => {
            if world.get::<Device>(entity).unwrap().recipients.is_empty() {
                return Ran::Declined;
            }
            execute_links(world, geometry, entity, depth);
        }
        TriggerAction::Teleport(arrival) => {
            world.entity_mut(level).insert(PendingPartyMove(PartyMove::Teleport(arrival.clone())));
        }
        TriggerAction::CameraPath(path) => {
            world.entity_mut(level).insert(PendingPartyMove(PartyMove::Fly(*path)));
        }
        TriggerAction::HurtParty(dice) => {
            if world.run_system_cached_with(damage_party, *dice).is_err() {
                return Ran::Declined;
            }
        }
        TriggerAction::Fountain(fountain) => {
            let at = sound_position(world, entity, definition);
            if !world
                .run_system_cached_with(drink_fountain, (entity, *fountain, at))
                .unwrap_or(false)
            {
                return Ran::Declined;
            }
        }
        TriggerAction::DamageZone => {
            if definition.place.is_some() {
                world.entity_mut(entity).insert(walk::ActiveDamageZone::new());
            }
        }
        TriggerAction::Shake { seconds, intensity, fade_out } => {
            world.write_message(TriggerEffect::Shake {
                seconds: *seconds,
                intensity: *intensity,
                fade_out: *fade_out,
            });
        }
        TriggerAction::Dial => {
            let [actor, mut owner] = world.get_entity_mut([entity, level]).unwrap();
            if !turn_dial(
                actor,
                owner.get_mut::<LevelState>().unwrap(),
                definition.state_change.as_ref().map(|(name, _)| name),
            ) {
                return Ran::Declined;
            }
        }
        TriggerAction::Unsupported(code) => {
            debug!("trigger {}: action {code:#x} is not implemented", definition.name);
            return Ran::Declined;
        }
    }
    if definition.sound_on_commit {
        play_authored_sound(world, entity, definition);
    }
    if definition.fire_linked {
        execute_links(world, geometry, entity, depth);
    }
    if changes_state && let Some((name, change)) = &definition.state_change {
        let mut state = world.get_mut::<LevelState>(level).unwrap();
        let value = match change {
            StateChange::Set => true,
            StateChange::Clear => false,
            StateChange::Toggle => !state.variable(name),
        };
        if state.variable(name) != value {
            state.set_variable(name, value);
        }
    }
    if let Some(message) = chosen.message {
        let source = world.get::<Level>(level).unwrap().messages.clone();
        world.write_message(TriggerEffect::Message { source, id: message });
    }
    let mut state = world.get_mut::<TriggerState>(entity).unwrap();
    if !state.fired {
        state.fired = true;
    }
    Ran::Committed
}
