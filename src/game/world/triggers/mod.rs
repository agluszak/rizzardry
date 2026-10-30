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
    Fountain, LevelDefinition, PropModel, Restores, TriggerAction, TriggerDefinition, TriggerUse,
    VariableName,
};
use crate::game::characters::{Character, Vitals};
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
    pub camera_sway: bool,
    variables: HashMap<VariableName, bool>,
    ambient_sounds: HashMap<String, bool>,
    previous_eye: Option<Vec3>,
}

/// Current named light activation, with its own rendering change boundary.
#[derive(Component, Clone, Debug, Default)]
pub struct LightActivation {
    lights: HashMap<String, bool>,
}
impl LightActivation {
    pub fn light_enabled(&self, name: &str, initial: bool) -> bool {
        self.lights.get(&name.to_uppercase()).copied().unwrap_or(initial)
    }
}

impl LevelState {
    pub fn ambient_sound_enabled(&self, name: &str) -> bool {
        self.ambient_sounds.get(&name.to_uppercase()).copied().unwrap_or(true)
    }
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
    mut members: Query<(&Character, &mut Vitals)>,
    mut rng: ResMut<GameRng>,
    mut formation: Single<&mut crate::game::party::formation::Formation>,
    mut damage: MessageWriter<DamageApplied>,
    fight: Option<Single<Entity, With<crate::game::combat::Battle>>>,
) {
    let fight = fight.map(|fight| *fight);
    for entity in roster.iter() {
        let (profile, mut member) = members.get_mut(entity).unwrap();
        if member.is_dead() {
            continue;
        }
        let rolled = rng.roll(dice);
        if rolled > 0 {
            let mut fate = crate::game::characters::Fate { fight, rng: &mut rng };
            crate::game::party::damage_member(
                entity,
                (profile, &mut member),
                &mut formation,
                (rolled, &mut fate),
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
    mut members: Query<(
        &Character,
        &mut Vitals,
        Option<(&crate::game::magic::LearnedSpells, &mut crate::game::magic::SpellPoints)>,
    )>,
    content: If<Res<crate::game::data::Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut outcomes: MessageWriter<TriggerEffect>,
) -> bool {
    let restoration = (fountain.restores == Restores::Restoration)
        .then(|| &content.spells[crate::content::spells::SpellId::HEAL_ALL]);
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
    for (profile, mut member, points) in members.iter_many_unique_mut(roster.iter()).matched() {
        if member.is_dead()
            || match fountain.restores {
                Restores::Restoration => false,
                Restores::Magic => points.is_none(),
                Restores::Health => member.hp >= profile.stats(&member.enchantments).max_hp(),
                Restores::Stamina => {
                    member.stamina >= profile.stats(&member.enchantments).max_stamina()
                        && !member.conditions.exhausted()
                }
            }
        {
            continue;
        }
        match fountain.restores {
            Restores::Restoration => {
                // PointCastSpell: zero does not cast, values above seven use
                // power one (the retail temple fountain authors fifteen).
                let power = if fountain.amount > 7 { 1 } else { fountain.amount as u8 };
                if power > 0 {
                    let amount = crate::game::magic::effects::magnitude(
                        &mut rng,
                        restoration.unwrap().dice,
                        crate::game::magic::Strength { power, ..default() },
                    );
                    let maximum = profile.stats(&member.enchantments).max_hp();
                    member.heal(amount, maximum);
                }
                member.conditions.lift_all();
            }
            Restores::Magic => {
                let (_, mut points) = points.unwrap();
                points.restore_evenly(fountain.amount);
            }
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
    /// A mandatory authored action has no implementation.
    Unsupported {
        device: String,
        code: u32,
    },
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
    if player && !crate::game::freedom::free_in(world) {
        return false;
    }
    let level = world.get::<LevelOwned>(entity).unwrap().0;
    let handle = world.get::<Level>(level).unwrap().definition.clone();
    // Borrow immutable definitions for the entire synchronous operation. Nested
    // links use these same definitions, without cloning a graph or a definition.
    world.resource_scope(|world, geometry: Mut<Assets<LevelDefinition>>| {
        let geometry = geometry.get(&handle).unwrap();
        let definition = &geometry.triggers[world.get::<Device>(entity).unwrap().definition];
        if player
            && (!usable(world, entity, definition)
                || player_distance(world, geometry, entity).is_none_or(|distance| {
                    !distance.is_finite()
                        || !(definition.range.0..=definition.range.1).contains(&distance)
                }))
        {
            return false;
        }
        let ran = execute(world, geometry, entity, 0, -1);
        if player && ran == Ran::Declined {
            world.write_message(TriggerEffect::NothingHappened);
        }
        ran == Ran::Committed
    })
}

/// Reach uses the current gameplay geometry, independent of the last pointer hit.
/// Colliderless props fall back to their authored centre.
fn player_distance(world: &mut World, geometry: &LevelDefinition, entity: Entity) -> Option<f32> {
    use avian3d::prelude::Collider;
    let eye = crate::game::party::body::PartyBody::eye_in(world);
    let centre = geometry.props.get(world.get::<Prop>(entity)?.definition)?.centre();
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
    Some(distance.unwrap_or_else(|| eye.distance(centre)))
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
    Use { chosen: &'a TriggerUse, keyed: bool, next: Option<bool> },
    /// A lock or trap must be overcome first.
    Guarded,
}

fn usable(world: &World, entity: Entity, definition: &TriggerDefinition) -> bool {
    world
        .get::<TriggerState>(entity)
        .is_some_and(|state| definition.usable && state.on && !(definition.once && state.fired))
}

/// Choose the use a run performs (retail `Trigger::SelectAction`). As in
/// retail, executing a supported selection commits before its action: a key unlocks the door
/// and overcomes its lock, and the initial/alternate sequence advances, even
/// if the chosen action then declines (retail `Run` returns without undoing
/// either). Fired flags, location variables, sounds, links and messages
/// wait for the action's commit. Selection is pure so an unsupported action
/// can be reported without any of those mutations.
fn select_use<'a>(
    actor: &bevy::ecs::world::EntityRef,
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
        return definition.fallback.as_ref().map(|chosen| Selected::Use {
            chosen,
            keyed: false,
            next: None,
        });
    }
    let mut keyed = false;
    if let Some(door) = actor.get::<DoorState>()
        && door.locked
        && let Some(key) = definition.door.and_then(|d| d.key)
    {
        if offered == Some(key) {
            keyed = true;
        } else {
            return definition.fallback.as_ref().map(|chosen| Selected::Use {
                chosen,
                keyed: false,
                next: None,
            });
        }
    }
    if !keyed && actor.contains::<Guarded>() {
        return Some(Selected::Guarded);
    }
    let state = actor.get::<TriggerState>()?;
    let (chosen, next) = match &definition.alternate {
        Some(alternate) if state.use_alternate => (alternate, !definition.alternate_toggles),
        alternate => (&definition.initial, alternate.is_some()),
    };
    Some(Selected::Use { chosen, keyed, next: Some(next) })
}

fn operate_door(
    mut device: EntityMut,
    model: &PropModel,
    open_only: bool,
) -> Result<bool, TriggerEffect> {
    if device.get::<DoorState>().is_some_and(|door| door.locked) {
        return Err(TriggerEffect::Locked);
    }
    let Ok((mut door, mut motion, mut trigger)) =
        device.get_components_mut::<(&mut DoorState, &mut PropMotion, &mut TriggerState)>()
    else {
        return Ok(false);
    };
    let open = open_only || !door.open;
    if motion.frame != motion.target || door.open == open {
        return Ok(false);
    }
    door.open = open;
    motion.target = if open { model.last_frame() as f32 } else { 0.0 };
    trigger.awaits_animation = true;
    Ok(true)
}

fn animate_device(
    mut device: EntityMut,
    model: &PropModel,
    animation: crate::content::level::Animation,
) {
    PropMotion::activate(device.get_mut::<PropMotion>().unwrap(), model, animation);
    if let Some(mut door) = device.get_mut::<DoorState>() {
        door.open = !door.open;
    }
    device.get_mut::<TriggerState>().unwrap().awaits_animation = true;
}

fn turn_dial(
    mut actor: EntityMut,
    model: &PropModel,
    mut level: Mut<LevelState>,
    variable: Option<&VariableName>,
) -> bool {
    let Ok((mut dial, mut motion)) =
        actor.get_components_mut::<(&mut DialState, &mut PropMotion)>()
    else {
        return false;
    };
    let old = dial.position;
    let slots = &model.slots;
    let next = if slots.len() < 3 {
        0
    } else {
        let next = (old + 1) % (slots.len() - 1);
        motion.frame = f32::from(slots[next]);
        motion.target = f32::from(slots[next + 1]);
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

fn sound_position(
    world: &World,
    geometry: &LevelDefinition,
    entity: Entity,
    definition: &TriggerDefinition,
) -> Option<Vec3> {
    definition.place.or_else(|| {
        let prop = world.get::<Prop>(entity)?;
        geometry.props.get(prop.definition).map(PropModel::centre)
    })
}

fn play_authored_sound(
    world: &mut World,
    geometry: &LevelDefinition,
    entity: Entity,
    definition: &TriggerDefinition,
) {
    if let Some(sound) = &definition.sound {
        world.write_message(TriggerEffect::Sound {
            path: sound.path.clone(),
            volume: sound.volume,
            at: sound_position(world, geometry, entity, definition),
        });
    }
}

fn execute_links(
    world: &mut World,
    geometry: &LevelDefinition,
    entity: Entity,
    depth: usize,
    source: i32,
) {
    let count = world.get::<Device>(entity).unwrap().recipients.len();
    for index in 0..count {
        let recipient = world.get::<Device>(entity).unwrap().recipients[index];
        execute(world, geometry, recipient, depth + 1, source);
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

fn execute(
    world: &mut World,
    geometry: &LevelDefinition,
    entity: Entity,
    depth: usize,
    source: i32,
) -> Ran {
    let device = world.get::<Device>(entity).unwrap();
    let level = world.get::<LevelOwned>(entity).unwrap().0;
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
    let offered = world.get::<PartyInventory>(party).unwrap().hand.held.as_ref().map(|h| h.item);
    let chosen = select_use(
        &world.entity(entity),
        world.get::<LevelState>(level).unwrap(),
        definition,
        offered,
    );
    let chosen = match chosen {
        None => return Ran::Declined,
        Some(Selected::Guarded) => {
            if let Some(lock) = definition.lock {
                super::interaction::confront(world, entity, lock);
            }
            return Ran::Answered;
        }
        Some(Selected::Use { chosen, keyed, next }) => {
            // Missing implementation is not a retail action failure. Preserve
            // selection and locks so installing support cannot skip this action.
            if let TriggerAction::Unsupported(code) = chosen.action {
                world.write_message(TriggerEffect::Unsupported {
                    device: definition.name.clone(),
                    code,
                });
                return Ran::Answered;
            }
            if keyed {
                world.get_mut::<DoorState>(entity).unwrap().locked = false;
                world.entity_mut(entity).remove::<Guarded>();
            }
            if let Some(next) = next {
                let mut state = world.get_mut::<TriggerState>(entity).unwrap();
                if state.use_alternate != next {
                    state.use_alternate = next;
                }
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
    // The prop the device moves, as its level authored it.
    let model = world.get::<Prop>(entity).and_then(|prop| geometry.props.get(prop.definition));
    match &chosen.action {
        TriggerAction::Nothing => {}
        TriggerAction::NoRuntimeAction => return Ran::Declined,
        action @ (TriggerAction::OpenDoor | TriggerAction::ToggleDoor) => {
            let Some(model) = model else { return Ran::Declined };
            match operate_door(
                world.entity_mut(entity).into_mutable(),
                model,
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
            let Some(model) = model.filter(|_| world.get::<PropMotion>(entity).is_some()) else {
                return Ran::Declined;
            };
            if world.get::<Container>(entity).is_some() {
                match world.run_system_cached_with(use_container, entity).unwrap() {
                    ContainerUse::Busy => return Ran::Declined,
                    ContainerUse::Deposited => return Ran::Answered,
                    ContainerUse::Toggle => {}
                    ContainerUse::Look { open } => {
                        super::interaction::begin(world, entity, Work::Looting);
                        if open {
                            return Ran::Answered;
                        }
                    }
                }
            }
            animate_device(world.entity_mut(entity).into_mutable(), model, *animation);
        }
        TriggerAction::Sound => {
            play_authored_sound(world, geometry, entity, definition);
            return Ran::Declined;
        }
        TriggerAction::CameraSway => {
            let mut state = world.get_mut::<LevelState>(level).unwrap();
            state.camera_sway = source > 0;
            return Ran::Answered;
        }
        TriggerAction::Relay(source) => {
            if world.get::<Device>(entity).unwrap().recipients.is_empty() {
                return Ran::Declined;
            }
            execute_links(world, geometry, entity, depth, *source);
        }
        TriggerAction::Lights { sources, change } => {
            if sources.is_empty() {
                return Ran::Declined;
            }
            let mut state = world.get_mut::<LightActivation>(level).unwrap();
            for (name, initial) in sources {
                let enabled = state.light_enabled(name, *initial);
                let next = change.apply(enabled);
                state.lights.insert(name.to_uppercase(), next);
            }
        }
        TriggerAction::Particles { sources, change } => {
            use super::particles::Emitter;
            let mut query = world.query::<(Entity, &Emitter, &LevelOwned)>();
            let recipients: Vec<_> = query
                .iter(world)
                .filter(|(_, emitter, owner)| {
                    owner.0 == level && sources.contains(&emitter.definition())
                })
                .map(|(entity, ..)| entity)
                .collect();
            if recipients.is_empty() {
                return Ran::Declined;
            }
            for recipient in recipients {
                let mut emitter = world.get_mut::<Emitter>(recipient).unwrap();
                let enabled = change.apply(emitter.emitting());
                emitter.set_active(enabled);
            }
        }
        TriggerAction::AmbientSounds { names, change } => {
            if names.is_empty() {
                return Ran::Declined;
            }
            let mut state = world.get_mut::<LevelState>(level).unwrap();
            for name in names {
                let enabled = state.ambient_sound_enabled(name);
                let next = change.apply(enabled);
                state.ambient_sounds.insert(name.to_uppercase(), next);
            }
        }
        TriggerAction::SetDevices(change) => {
            let count = world.get::<Device>(entity).unwrap().recipients.len();
            if count == 0 {
                return Ran::Declined;
            }
            for index in 0..count {
                let recipient = world.get::<Device>(entity).unwrap().recipients[index];
                let mut state = world.get_mut::<TriggerState>(recipient).unwrap();
                let on = change.apply(state.on);
                if state.on != on {
                    state.on = on;
                }
            }
        }
        TriggerAction::RandomTeleport(arrivals) => {
            if arrivals.is_empty() {
                return Ran::Declined;
            }
            // Retail uses wall-clock modulo. The simulation RNG makes replay
            // and continuation independent of rendering and machine timing.
            let index = world.resource_mut::<GameRng>().below(arrivals.len() as u32) as usize;
            world
                .entity_mut(level)
                .insert(PendingPartyMove(PartyMove::Teleport(arrivals[index].clone())));
        }
        TriggerAction::Teleport(arrival) => {
            world.entity_mut(level).insert(PendingPartyMove(PartyMove::Teleport(arrival.clone())));
        }
        TriggerAction::CameraPath(path) => {
            world.entity_mut(level).insert(PendingPartyMove(PartyMove::Fly(*path)));
        }
        // Refused with its level at preparation.
        TriggerAction::HurtParty(dice) => {
            if world.run_system_cached_with(damage_party, *dice).is_err() {
                return Ran::Declined;
            }
        }
        TriggerAction::Fountain(fountain) => {
            let at = sound_position(world, geometry, entity, definition);
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
            let Some(model) = model else { return Ran::Declined };
            let [actor, mut owner] = world.get_entity_mut([entity, level]).unwrap();
            if !turn_dial(
                actor,
                model,
                owner.get_mut::<LevelState>().unwrap(),
                definition.state_change.as_ref().map(|(name, _)| name),
            ) {
                return Ran::Declined;
            }
        }
        TriggerAction::Unsupported(_) => unreachable!("checked before selection commits"),
    }
    if definition.sound_on_commit {
        play_authored_sound(world, geometry, entity, definition);
    }
    if definition.fire_linked {
        execute_links(world, geometry, entity, depth, -1);
    }
    if changes_state && let Some((name, change)) = &definition.state_change {
        let mut state = world.get_mut::<LevelState>(level).unwrap();
        let value = change.apply(state.variable(name));
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
