//! Live devices and synchronous trigger operations. Definitions stay in the
//! level asset; runtime links and mutable state belong to device entities.
//!
//! An action commits its sound, ordered recipients, location variable, message
//! and fired flag synchronously. A relay also runs its recipients as its action,
//! preserving retail's double execution when it is marked to fire on commit.

mod records;
mod walk;

pub use records::{DeviceRecords, TriggerMemory, spawn_devices};
pub(crate) use walk::walk;

use std::collections::HashMap;

use bevy::prelude::*;

use super::containers::{Container, ContainerUse};
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

/// Everything running a device may touch: the devices' own state, their
/// level's variables, lights and emitters, the party, the dice and the
/// clock. The trigger interpreter runs over it; the level's geometry is
/// passed beside it, borrowed for the whole run.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Devices<'w, 's> {
    devices: Query<
        'w,
        's,
        (
            &'static Device,
            &'static LevelOwned,
            &'static mut TriggerState,
            Option<&'static mut DoorState>,
            Option<&'static mut DialState>,
            Option<&'static mut PropMotion>,
            Option<&'static Prop>,
            Option<&'static BellButton>,
            Has<super::interaction::DeviceAttempt>,
        ),
    >,
    guards: Query<'w, 's, &'static mut Guarded>,
    containers: Query<'w, 's, &'static mut Container>,
    fountains: Query<'w, 's, &'static mut FountainState>,
    levels: Query<'w, 's, (&'static Level, &'static mut LevelState, &'static mut LightActivation)>,
    emitters: Query<'w, 's, (&'static mut super::particles::Emitter, &'static LevelOwned)>,
    attempts: Query<'w, 's, (), With<super::interaction::DeviceInteraction>>,
    parts: Query<
        'w,
        's,
        (&'static Transform, &'static avian3d::prelude::Collider),
        With<super::props::PropPart>,
    >,
    children: Query<'w, 's, &'static Children>,
    party: Single<
        'w,
        's,
        (
            &'static avian3d::prelude::Position,
            &'static mut PartyInventory,
            &'static PartyRoster,
            &'static mut crate::game::party::formation::Formation,
        ),
    >,
    members: Query<
        'w,
        's,
        (
            &'static Character,
            &'static mut Vitals,
            Option<(
                &'static crate::game::magic::LearnedSpells,
                &'static mut crate::game::magic::SpellPoints,
            )>,
        ),
    >,
    battles: Query<'w, 's, Entity, With<crate::game::combat::Battle>>,
    freedom: crate::game::freedom::Freedom<'w, 's>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    rng: ResMut<'w, GameRng>,
    ids: ResMut<'w, crate::game::inventory::ItemIds>,
    clock: Res<'w, GameClock>,
    effects: MessageWriter<'w, TriggerEffect>,
    locks: MessageWriter<'w, super::interaction::LockNews>,
    damage: MessageWriter<'w, DamageApplied>,
    commands: Commands<'w, 's>,
    /// A device has asked to move the party in this run; the move itself
    /// waits for the trigger step to end.
    moved: Local<'s, bool>,
    /// Devices whose lock or trap was overcome in this run; their guard
    /// goes when the run's commands apply.
    overcome: Local<'s, Vec<Entity>>,
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

impl Devices<'_, '_> {
    /// A level's geometry.
    fn geometry<'g>(
        &self,
        assets: &'g Assets<LevelDefinition>,
        level: Entity,
    ) -> &'g LevelDefinition {
        assets.get(&self.levels.get(level).unwrap().0.definition).unwrap()
    }

    fn definition<'g>(
        &self,
        geometry: &'g LevelDefinition,
        entity: Entity,
    ) -> &'g TriggerDefinition {
        &geometry.triggers[self.devices.get(entity).unwrap().0.definition]
    }

    fn request_move(&mut self, level: Entity, wanted: PartyMove) {
        self.commands.entity(level).insert(PendingPartyMove(wanted));
        *self.moved = true;
    }

    /// Use a device. Player use checks the party is free, the device usable
    /// and within its reach now; linked and script activation do not.
    /// Returns whether it committed.
    pub fn activate(
        &mut self,
        geometry: &Assets<LevelDefinition>,
        entity: Entity,
        player: bool,
    ) -> bool {
        if player && !self.freedom.free() {
            return false;
        }
        let level = self.devices.get(entity).unwrap().1.0;
        let geometry = self.geometry(geometry, level);
        let definition = self.definition(geometry, entity);
        if player
            && (!self.usable(entity, definition)
                || !(definition.range.0..=definition.range.1)
                    .contains(&self.player_distance(geometry, entity)))
        {
            return false;
        }
        let ran = self.execute(geometry, entity, 0, -1);
        if player && ran == Ran::Declined {
            self.effects.write(TriggerEffect::NothingHappened);
        }
        ran == Ran::Committed
    }

    /// The device's lock or trap is overcome: it is unlocked and runs
    /// (retail `CompleteItemInteraction` then `Run`).
    pub fn open(&mut self, geometry: &Assets<LevelDefinition>, device: Entity) {
        self.commands.entity(device).remove::<Guarded>();
        self.overcome.push(device);
        if let Some(mut door) = self.devices.get_mut(device).unwrap().3 {
            door.locked = false;
        }
        self.activate(geometry, device, false);
        self.overcome.clear();
    }

    /// Reach uses the current gameplay geometry, independent of the last
    /// pointer hit. Colliderless props fall back to their authored centre.
    /// The player uses only props.
    fn player_distance(&self, geometry: &LevelDefinition, entity: Entity) -> f32 {
        let eye = crate::game::party::body::PartyBody::eye(self.party.0);
        let prop = self.devices.get(entity).unwrap().6.unwrap();
        let centre = geometry.props[prop.definition].centre();
        self.children
            .get(entity)
            .into_iter()
            .flatten()
            .filter_map(|&child| self.parts.get(child).ok())
            .map(|(at, collider)| {
                collider.distance_to_point(at.translation, at.rotation, eye, true)
            })
            .min_by(f32::total_cmp)
            .unwrap_or_else(|| eye.distance(centre))
    }

    fn usable(&self, entity: Entity, definition: &TriggerDefinition) -> bool {
        let state = &self.devices.get(entity).unwrap().2;
        definition.usable && state.on && !(definition.once && state.fired)
    }

    /// Choose the use a run performs (retail `Trigger::SelectAction`). As in
    /// retail, executing a supported selection commits before its action: a
    /// key unlocks the door and overcomes its lock, and the initial/alternate
    /// sequence advances, even if the chosen action then declines (retail
    /// `Run` returns without undoing either). Fired flags, location
    /// variables, sounds, links and messages wait for the action's commit.
    /// Selection is pure so an unsupported action can be reported without
    /// any of those mutations.
    fn select_use<'a>(
        &self,
        entity: Entity,
        level: Entity,
        definition: &'a TriggerDefinition,
    ) -> Option<Selected<'a>> {
        let (_, _, state, door, _, motion, ..) = self.devices.get(entity).unwrap();
        let variables = &self.levels.get(level).unwrap().1;
        let running = state.awaits_animation && motion.is_some_and(|m| m.frame != m.target);
        if !state.on
            || (definition.once && state.fired)
            || (running && !matches!(definition.initial.action, TriggerAction::Shake { .. }))
        {
            return None;
        }
        let fallback = || {
            definition.fallback.as_ref().map(|chosen| Selected::Use {
                chosen,
                keyed: false,
                next: None,
            })
        };
        if !definition.required_states.iter().all(|name| variables.variable(name)) {
            return fallback();
        }
        let offered = self.party.1.hand.held.as_ref().map(|h| h.item);
        let mut keyed = false;
        if door.is_some_and(|door| door.locked)
            && let Some(key) = definition.door.and_then(|d| d.key)
        {
            if offered != Some(key) {
                return fallback();
            }
            keyed = true;
        }
        if !keyed && self.guards.contains(entity) && !self.overcome.contains(&entity) {
            return Some(Selected::Guarded);
        }
        let (chosen, next) = match &definition.alternate {
            Some(alternate) if state.use_alternate => (alternate, !definition.alternate_toggles),
            alternate => (&definition.initial, alternate.is_some()),
        };
        Some(Selected::Use { chosen, keyed, next: Some(next) })
    }

    /// Whether a device this one runs, or one those run, is mid-animation.
    fn links_moving(&self, entity: Entity, depth: usize) -> bool {
        depth <= MAX_DEPTH
            && self.devices.get(entity).unwrap().0.recipients.iter().any(|&recipient| {
                self.devices.get(recipient).unwrap().5.is_some_and(|m| m.frame != m.target)
                    || self.links_moving(recipient, depth + 1)
            })
    }

    fn sound_position(
        &self,
        geometry: &LevelDefinition,
        entity: Entity,
        definition: &TriggerDefinition,
    ) -> Option<Vec3> {
        definition.place.or_else(|| {
            let prop = self.devices.get(entity).unwrap().6?;
            geometry.props.get(prop.definition).map(PropModel::centre)
        })
    }

    fn play_authored_sound(
        &mut self,
        geometry: &LevelDefinition,
        entity: Entity,
        definition: &TriggerDefinition,
    ) {
        if let Some(sound) = &definition.sound {
            let at = self.sound_position(geometry, entity, definition);
            self.effects.write(TriggerEffect::Sound {
                path: sound.path.clone(),
                volume: sound.volume,
                at,
            });
        }
    }

    fn execute_links(
        &mut self,
        geometry: &LevelDefinition,
        entity: Entity,
        depth: usize,
        source: i32,
    ) {
        let recipients = self.devices.get(entity).unwrap().0.recipients.clone();
        for recipient in recipients {
            self.execute(geometry, recipient, depth + 1, source);
        }
    }

    /// Hurt each living member by `dice`, in roster order.
    fn damage_party(&mut self, dice: Dice) {
        let fight = self.battles.single().ok();
        let (_, _, roster, formation) = &mut *self.party;
        for entity in roster.iter() {
            let (profile, mut member, _) = self.members.get_mut(entity).unwrap();
            if member.is_dead() {
                continue;
            }
            let rolled = self.rng.roll(dice);
            if rolled > 0 {
                let mut fate = crate::game::characters::Fate { fight, rng: &mut self.rng };
                crate::game::party::damage_member(
                    entity,
                    (profile, &mut member),
                    formation,
                    (rolled, &mut fate),
                    &mut self.damage,
                );
            }
        }
        self.commands.run_system_cached(crate::game::combat::settle_defeat);
    }

    fn drink_fountain(&mut self, entity: Entity, fountain: Fountain, at: Option<Vec3>) -> bool {
        let restoration = (fountain.restores == Restores::Restoration)
            .then(|| &self.content.spells[crate::content::spells::SpellId::HEAL_ALL]);
        let mut state = self.fountains.get_mut(entity).unwrap();
        let now = self.clock.ms;
        // A dry fountain is assessed without marking it changed.
        if state
            .used
            .is_some_and(|s| s.left == Some(0) && !s.refills_at.is_some_and(|at| at <= now))
        {
            return false;
        }
        let state =
            state.used.get_or_insert(FountainUses { left: fountain.uses, refills_at: None });
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
        let roster = self.party.2;
        for (profile, mut member, points) in
            self.members.iter_many_unique_mut(roster.iter()).matched()
        {
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
                    // PointCastSpell: zero does not cast, values above seven
                    // use power one (the retail temple fountain authors
                    // fifteen).
                    let power = if fountain.amount > 7 { 1 } else { fountain.amount as u8 };
                    if power > 0 {
                        let amount = crate::game::magic::effects::magnitude(
                            &mut self.rng,
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
        self.effects.write(TriggerEffect::FountainUsed { at });
        true
    }

    /// A device's lock or trap stops its use: start working on it (retail
    /// `OpenLockInteraction`, `OpenTrapInteraction`).
    fn confront(&mut self, device: Entity, lock: crate::content::level::Lock) {
        if !self.attempts.is_empty() {
            return;
        }
        if !self.battles.is_empty() {
            self.locks.write(super::interaction::LockNews::NotInCombat);
            return;
        }
        let mut guarded = self.guards.get_mut(device).unwrap();
        let work = super::interaction::face(
            lock,
            &mut guarded,
            self.clock.ms,
            &mut self.rng,
            &mut self.locks,
        );
        self.commands.spawn((
            super::interaction::DeviceInteraction { work },
            super::interaction::AttemptOn(device),
        ));
    }

    fn execute(
        &mut self,
        geometry: &LevelDefinition,
        entity: Entity,
        depth: usize,
        source: i32,
    ) -> Ran {
        let (device, owner, ..) = self.devices.get(entity).unwrap();
        let level = owner.0;
        let definition = &geometry.triggers[device.definition];
        if depth > MAX_DEPTH {
            warn!("trigger {} recurses too deeply", definition.name);
            return Ran::Declined;
        }
        // A door waits while any device it runs is still moving (retail
        // `CanRunLinkedTriggers`), before a key is tried.
        if definition.door.is_some() && self.links_moving(entity, 0) {
            return Ran::Declined;
        }
        let chosen = match self.select_use(entity, level, definition) {
            None => return Ran::Declined,
            Some(Selected::Guarded) => {
                if let Some(lock) = definition.lock {
                    self.confront(entity, lock);
                }
                return Ran::Answered;
            }
            Some(Selected::Use { chosen, keyed, next }) => {
                // Missing implementation is not a retail action failure.
                // Preserve selection and locks so installing support cannot
                // skip this action.
                if let TriggerAction::Unsupported(code) = chosen.action {
                    self.effects.write(TriggerEffect::Unsupported {
                        device: definition.name.clone(),
                        code,
                    });
                    return Ran::Answered;
                }
                let (_, _, mut state, door, ..) = self.devices.get_mut(entity).unwrap();
                if keyed {
                    door.unwrap().locked = false;
                    self.commands.entity(entity).remove::<Guarded>();
                }
                if let Some(next) = next
                    && state.use_alternate != next
                {
                    state.use_alternate = next;
                }
                chosen
            }
        };
        // A level callback runs once the action is selected, before it
        // executes (retail `Trigger::Run`).
        if let Some(&BellButton { dial }) = self.devices.get(entity).unwrap().7 {
            let [mut button, mut ring] = self.devices.get_many_mut([entity, dial]).unwrap();
            let position = ring.4.as_ref().unwrap().position;
            let outcome = super::scripts::ring_bell(position, &mut button.2, &mut ring.2);
            self.effects.write(outcome);
        }
        let changes_state = !matches!(chosen.action, TriggerAction::Dial);
        // The prop the device moves, as its level authored it.
        let model = self
            .devices
            .get(entity)
            .unwrap()
            .6
            .and_then(|prop| geometry.props.get(prop.definition));
        match &chosen.action {
            TriggerAction::Nothing => {}
            TriggerAction::NoRuntimeAction => return Ran::Declined,
            action @ (TriggerAction::OpenDoor | TriggerAction::ToggleDoor) => {
                let Some(model) = model else { return Ran::Declined };
                let (_, _, mut trigger, door, _, motion, ..) =
                    self.devices.get_mut(entity).unwrap();
                let (Some(mut door), Some(mut motion)) = (door, motion) else {
                    return Ran::Declined;
                };
                if door.locked {
                    self.effects.write(TriggerEffect::Locked);
                    return Ran::Declined;
                }
                let open = *action == TriggerAction::OpenDoor || !door.open;
                if motion.frame != motion.target || door.open == open {
                    return Ran::Declined;
                }
                door.open = open;
                motion.target = if open { model.last_frame() as f32 } else { 0.0 };
                trigger.awaits_animation = true;
            }
            TriggerAction::Animate(animation) => {
                let Some(model) = model.filter(|_| self.devices.get(entity).unwrap().5.is_some())
                else {
                    return Ran::Declined;
                };
                if self.containers.contains(entity) {
                    match self.use_container(entity) {
                        ContainerUse::Busy => return Ran::Declined,
                        ContainerUse::Deposited => return Ran::Answered,
                        ContainerUse::Toggle => {}
                        ContainerUse::Look { open } => {
                            self.commands.spawn((
                                super::interaction::DeviceInteraction { work: Work::Looting },
                                super::interaction::AttemptOn(entity),
                            ));
                            if open {
                                return Ran::Answered;
                            }
                        }
                    }
                }
                let (_, _, mut trigger, door, _, motion, ..) =
                    self.devices.get_mut(entity).unwrap();
                PropMotion::activate(motion.unwrap(), model, *animation);
                if let Some(mut door) = door {
                    door.open = !door.open;
                }
                trigger.awaits_animation = true;
            }
            TriggerAction::Sound => {
                self.play_authored_sound(geometry, entity, definition);
                return Ran::Declined;
            }
            TriggerAction::CameraSway => {
                self.levels.get_mut(level).unwrap().1.camera_sway = source > 0;
                return Ran::Answered;
            }
            TriggerAction::Relay(source) => {
                if self.devices.get(entity).unwrap().0.recipients.is_empty() {
                    return Ran::Declined;
                }
                self.execute_links(geometry, entity, depth, *source);
            }
            TriggerAction::Lights { sources, change } => {
                if sources.is_empty() {
                    return Ran::Declined;
                }
                let mut lights = self.levels.get_mut(level).unwrap().2;
                for (name, initial) in sources {
                    let enabled = lights.light_enabled(name, *initial);
                    lights.lights.insert(name.to_uppercase(), change.apply(enabled));
                }
            }
            TriggerAction::Particles { sources, change } => {
                let mut any = false;
                for (mut emitter, owner) in &mut self.emitters {
                    if owner.0 == level && sources.contains(&emitter.definition()) {
                        let enabled = change.apply(emitter.emitting());
                        emitter.set_active(enabled);
                        any = true;
                    }
                }
                if !any {
                    return Ran::Declined;
                }
            }
            TriggerAction::AmbientSounds { names, change } => {
                if names.is_empty() {
                    return Ran::Declined;
                }
                let mut state = self.levels.get_mut(level).unwrap().1;
                for name in names {
                    let enabled = state.ambient_sound_enabled(name);
                    state.ambient_sounds.insert(name.to_uppercase(), change.apply(enabled));
                }
            }
            TriggerAction::SetDevices(change) => {
                let recipients = self.devices.get(entity).unwrap().0.recipients.clone();
                if recipients.is_empty() {
                    return Ran::Declined;
                }
                for recipient in recipients {
                    let mut state = self.devices.get_mut(recipient).unwrap().2;
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
                // Retail uses wall-clock modulo. The simulation RNG makes
                // replay and continuation independent of rendering and
                // machine timing.
                let index = self.rng.below(arrivals.len() as u32) as usize;
                self.request_move(level, PartyMove::Teleport(arrivals[index].clone()));
            }
            TriggerAction::Teleport(arrival) => {
                self.request_move(level, PartyMove::Teleport(arrival.clone()));
            }
            TriggerAction::CameraPath(path) => {
                self.request_move(level, PartyMove::Fly(*path));
            }
            TriggerAction::HurtParty(dice) => self.damage_party(*dice),
            TriggerAction::Fountain(fountain) => {
                let at = self.sound_position(geometry, entity, definition);
                if !self.drink_fountain(entity, *fountain, at) {
                    return Ran::Declined;
                }
            }
            TriggerAction::DamageZone => {
                if definition.place.is_some() {
                    self.commands.entity(entity).insert(walk::ActiveDamageZone::new());
                }
            }
            TriggerAction::Shake { seconds, intensity, fade_out } => {
                self.effects.write(TriggerEffect::Shake {
                    seconds: *seconds,
                    intensity: *intensity,
                    fade_out: *fade_out,
                });
            }
            TriggerAction::Dial => {
                let Some(model) = model else { return Ran::Declined };
                let (_, _, _, _, dial, motion, ..) = self.devices.get_mut(entity).unwrap();
                let (Some(mut dial), Some(mut motion)) = (dial, motion) else {
                    return Ran::Declined;
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
                if let Some((name, _)) = &definition.state_change {
                    let mut state = self.levels.get_mut(level).unwrap().1;
                    state.set_variable(&name.numbered(old), false);
                    state.set_variable(&name.numbered(next), true);
                }
            }
            TriggerAction::Unsupported(_) => unreachable!("reported before selection commits"),
        }
        if definition.sound_on_commit {
            self.play_authored_sound(geometry, entity, definition);
        }
        if definition.fire_linked {
            self.execute_links(geometry, entity, depth, -1);
        }
        let (level_definition, mut state, _) = self.levels.get_mut(level).unwrap();
        if changes_state && let Some((name, change)) = &definition.state_change {
            let value = change.apply(state.variable(name));
            if state.variable(name) != value {
                state.set_variable(name, value);
            }
        }
        if let Some(message) = chosen.message {
            let source = level_definition.messages.clone();
            self.effects.write(TriggerEffect::Message { source, id: message });
        }
        let mut state = self.devices.get_mut(entity).unwrap().2;
        if !state.fired {
            state.fired = true;
        }
        Ran::Committed
    }

    /// Retail `Trigger::RunAction` case 3: a held item goes in; otherwise
    /// the contents are drawn once, their gold goes to the party, a lone
    /// item to the hand, and several are looked through.
    fn use_container(&mut self, entity: Entity) -> ContainerUse {
        let (_, _, _, _, _, motion, _, _, busy) = self.devices.get(entity).unwrap();
        let shut = motion.unwrap().target <= 0.0;
        let mut container = self.containers.get_mut(entity).unwrap();
        let (_, inventory, roster, _) = &mut *self.party;
        if let Some(held) = inventory.hand.held.take() {
            container.contents.items.push(held);
            return ContainerUse::Deposited;
        }
        if busy {
            return ContainerUse::Busy;
        }
        if let Some(table) = container.table.take() {
            let level = crate::game::party::average_level(
                roster.iter().map(|m| self.members.get(m).unwrap().0.level()),
            );
            super::containers::draw_loot(
                &mut container.contents,
                table,
                level,
                &self.content,
                (&mut self.ids, &mut self.rng),
            );
        }
        let contents = &mut container.contents;
        if contents.gold > 0 {
            inventory.gold += contents.gold;
            self.effects.write(TriggerEffect::Gold(contents.gold));
            contents.gold = 0;
        }
        match contents.items.len() {
            0 => ContainerUse::Toggle,
            1 => {
                inventory.hand.held = contents.items.pop();
                ContainerUse::Toggle
            }
            _ => ContainerUse::Look { open: !shut },
        }
    }
}
