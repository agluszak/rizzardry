//! Live devices and synchronous trigger operations. Definitions stay in the
//! level asset; runtime links and mutable state belong to device entities.
//!
//! An action commits its sound, ordered recipients, location variable, message
//! and fired flag synchronously. A relay also runs its recipients as its action,
//! preserving retail's double execution when it is marked to fire on commit.
//! See `docs/research/trigger-actions.md`.

use std::collections::HashMap;
use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::level::{Departing, Level};
use super::props::{Prop, PropMotion};
use super::scripts::{BellButton, LevelScript};
use crate::content::items::{Dice, ItemCatalog, LootTables};
use crate::content::level::{
    Animation, Fountain, LevelGeometry, Restores, StateChange, TriggerAction, TriggerDefinition,
    TriggerUse,
};
use crate::game::characters::Character;
use crate::game::data::GameData;
use crate::game::inventory::{ItemIds, ItemInstance, PartyInventory, make_item};
use crate::game::loot::roll_loot;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

const DAMAGE_ZONE_PERIOD: f32 = 0.5;
const MAX_DEPTH: usize = 16;

/// A device's immutable definition and links, resolved once per visit.
/// A shared device has its own entity; a device used by one prop lives on it.
#[derive(Component, Debug)]
pub struct Device {
    pub level: Entity,
    pub definition: usize,
    pub recipients: Vec<Entity>,
    pub prop: Option<Entity>,
}

/// Only level-wide variables and visit state live on the level.
#[derive(Component, Debug, Default)]
pub struct LevelState {
    variables: HashMap<String, bool>,
    entered: bool,
}

impl LevelState {
    pub fn variable(&self, name: &str) -> bool {
        self.variables.get(&name.to_uppercase()).copied().unwrap_or(false)
    }

    pub fn set_variable(&mut self, name: &str, value: bool) {
        let name = name.to_uppercase();
        self.variables.insert(name, value);
    }
}

#[derive(Component, Debug)]
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

#[derive(Component, Debug)]
pub struct Container {
    table: Option<usize>,
    pub contents: Option<Contents>,
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

#[derive(Component, Debug, Default)]
pub struct DeviceProximity {
    inside: bool,
    zone: Option<Timer>,
}

/// Dormant records use authored indices, never runtime entity references.
#[derive(Debug)]
pub struct TriggerMemory {
    devices: HashMap<usize, KeptDevice>,
    variables: HashMap<String, bool>,
}

#[derive(Debug)]
struct KeptDevice {
    on: bool,
    fired: bool,
    use_alternate: bool,
    door: Option<DoorState>,
    dial: Option<DialState>,
    contents: Option<Contents>,
    fountain: Option<FountainState>,
}

#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub items: Vec<ItemInstance>,
    pub gold: u32,
}

/// Capture device state without persisting animation waits, proximity or zones.
#[derive(SystemParam)]
pub struct DeviceRecords<'w, 's> {
    devices: Query<
        'w,
        's,
        (
            &'static Device,
            &'static TriggerState,
            Option<&'static DoorState>,
            Option<&'static DialState>,
            Option<&'static Container>,
            Option<&'static FountainState>,
        ),
    >,
}

impl DeviceRecords<'_, '_> {
    pub fn remember(&self, level: Entity, state: &LevelState) -> TriggerMemory {
        let devices = self
            .devices
            .iter()
            .filter(|(device, ..)| device.level == level)
            .map(|(device, state, door, dial, container, fountain)| {
                (
                    device.definition,
                    KeptDevice {
                        on: state.on,
                        fired: state.fired,
                        use_alternate: state.use_alternate,
                        door: door.cloned(),
                        dial: dial.cloned(),
                        contents: container.and_then(|c| c.contents.clone()),
                        fountain: fountain.cloned(),
                    },
                )
            })
            .collect();
        TriggerMemory { devices, variables: state.variables.clone() }
    }
}

/// Allocate identities before linking devices or building prop children. The
/// final prop is the animation source for a shared trigger, preserving the
/// previous executor's selection. Recipients use the shared device identity.
pub fn spawn_devices(
    commands: &mut Commands,
    level: Entity,
    geometry: &LevelGeometry,
    props: &[Entity],
    script: LevelScript,
    tables: &LootTables,
    memory: Option<&TriggerMemory>,
) -> Vec<Entity> {
    let mut attached = vec![Vec::new(); geometry.triggers.len()];
    for (index, prop) in geometry.props.iter().enumerate() {
        if let Some(trigger) = prop.trigger {
            attached[trigger].push(props[index]);
        }
    }
    let devices: Vec<Entity> = attached
        .iter()
        .map(|props| if props.len() == 1 { props[0] } else { commands.spawn(ChildOf(level)).id() })
        .collect();
    let mut state = LevelState::default();
    for (index, definition) in geometry.triggers.iter().enumerate() {
        let kept = memory.and_then(|m| m.devices.get(&index));
        let mut entity = commands.entity(devices[index]);
        entity.insert((
            Device {
                level,
                definition: index,
                prop: attached[index].last().copied(),
                recipients: definition.recipients.iter().map(|&i| devices[i]).collect(),
            },
            TriggerState {
                on: kept.is_none_or(|k| k.on),
                fired: kept.is_some_and(|k| k.fired),
                use_alternate: kept.is_some_and(|k| k.use_alternate),
                awaits_animation: false,
            },
        ));
        let actions = std::iter::once(&definition.initial)
            .chain(definition.alternate.iter())
            .chain(definition.fallback.iter());
        if definition.door.is_some()
            || actions
                .clone()
                .any(|b| matches!(b.action, TriggerAction::OpenDoor | TriggerAction::ToggleDoor))
        {
            entity.insert(kept.and_then(|k| k.door.clone()).unwrap_or(DoorState {
                open: false,
                locked: definition.door.is_some_and(|d| d.locked),
            }));
        }
        if actions.clone().any(|b| matches!(b.action, TriggerAction::Dial)) {
            entity.insert(kept.and_then(|k| k.dial.clone()).unwrap_or_default());
        }
        if actions.clone().any(|b| matches!(b.action, TriggerAction::Fountain(_))) {
            entity.insert(kept.and_then(|k| k.fountain.clone()).unwrap_or_default());
        }
        if let Some(name) = &definition.loot_table {
            let table = tables.0.iter().position(|t| t.name.eq_ignore_ascii_case(name));
            if table.is_none() {
                warn!("trigger {}: no loot table {name}; it holds nothing", definition.name);
            }
            entity.insert(Container { table, contents: kept.and_then(|k| k.contents.clone()) });
        }
        if definition.place.is_some() {
            entity.insert(DeviceProximity::default());
        }
        if let (Some((name, _)), Some(value)) = (&definition.state_change, definition.initial_state)
        {
            state.variables.entry(name.to_uppercase()).or_insert(value);
        }
    }
    for prop in &geometry.props {
        let Some(definition) = prop.trigger.map(|i| &geometry.triggers[i]) else { continue };
        if let (TriggerAction::Dial, Some((name, _))) =
            (&definition.initial.action, &definition.state_change)
        {
            for position in 0..prop.slots.len() {
                state
                    .variables
                    .entry(format!("{name}{position}").to_uppercase())
                    .or_insert(position == 0);
            }
        }
    }
    if let Some(memory) = memory {
        state.variables = memory.variables.clone();
    }
    commands.entity(level).insert(state);
    script.bind(commands, geometry, &devices);
    devices
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
    Message(i32),
    /// The party used something and nothing came of it.
    NothingHappened,
    /// A member was hurt.
    Hurt {
        member: String,
        amount: u32,
    },
    /// An item went to the party ("Party takes %s").
    PartyTakes(String),
    Gold(u32),
    Locked,
}

/// Moving the party somewhere, which the party module carries out.
#[derive(Clone, Debug)]
pub enum PartyMove {
    Teleport(crate::content::level::Arrival),
    /// Fly the party's eye along a camera path, by name (ladders, climbs).
    Fly(String),
}

/// A move requested by a device, scoped to its originating level.
/// Taking the move discards it if that level is no longer active.
#[derive(Resource, Debug, Default)]
pub struct PendingPartyMove(pub Option<(Entity, PartyMove)>);

/// Focused ECS access for synchronous operations. Queries stay queries: merely
/// assessing a device never extracts mutable props or party members.
#[derive(SystemParam)]
pub struct DeviceOperations<'w, 's> {
    levels: Query<'w, 's, (&'static Level, &'static mut LevelState), Without<Departing>>,
    geometry: Res<'w, Assets<LevelGeometry>>,
    devices: Query<'w, 's, (Entity, &'static Device)>,
    states: Query<'w, 's, &'static mut TriggerState>,
    doors: Query<'w, 's, &'static mut DoorState>,
    dials: Query<'w, 's, &'static mut DialState>,
    containers: Query<'w, 's, &'static mut Container>,
    fountains: Query<'w, 's, &'static mut FountainState>,
    proximity: Query<'w, 's, &'static mut DeviceProximity>,
    bells: Query<'w, 's, &'static BellButton>,
    props: Query<'w, 's, (&'static Prop, &'static mut PropMotion)>,
    effects: MessageWriter<'w, TriggerEffect>,
    party_move: ResMut<'w, PendingPartyMove>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
    loot: Res<'w, Assets<LootTables>>,
    rng: ResMut<'w, GameRng>,
    ids: ResMut<'w, ItemIds>,
    party: ResMut<'w, PartyInventory>,
    members: Query<'w, 's, &'static mut Character>,
    roster: Res<'w, PartyRoster>,
    clock: Res<'w, GameClock>,
}

impl<'w, 's> DeviceOperations<'w, 's> {
    pub fn take_party_move(&mut self) -> Option<PartyMove> {
        let (level, movement) = self.party_move.0.take()?;
        self.levels.get(level).ok()?;
        Some(movement)
    }

    fn ready(&self) -> bool {
        self.data
            .as_ref()
            .is_some_and(|d| self.catalogs.contains(&d.items) && self.loot.contains(&d.loot))
    }

    fn definition(&self, entity: Entity) -> Option<&'w TriggerDefinition> {
        let (_, device) = self.devices.get(entity).ok()?;
        let (level, _) = self.levels.get(device.level).ok()?;
        let geometry = Res::clone(&self.geometry).into_inner();
        geometry.get(&level.geometry)?.triggers.get(device.definition)
    }

    pub fn usable(&self, entity: Entity) -> bool {
        let Some(definition) = self.definition(entity) else { return false };
        let Ok(state) = self.states.get(entity) else { return false };
        definition.usable && !(definition.once && state.fired)
    }

    /// Gameplay callers provide a resolved device and hit distance. The
    /// operation owns reach/eligibility validation, independent of mouse input.
    pub fn use_device(&mut self, entity: Entity, distance: f32) -> bool {
        let Some(definition) = self.definition(entity) else { return false };
        if !self.usable(entity)
            || !(definition.range.0..=definition.range.1).contains(&distance)
            || !self.ready()
        {
            return false;
        }
        let committed = self.run(entity);
        if !committed {
            self.effects.write(TriggerEffect::NothingHappened);
        }
        committed
    }

    pub fn run(&mut self, entity: Entity) -> bool {
        self.ready() && self.run_device(entity, 0)
    }

    pub fn walk(&mut self, eye: Vec3, elapsed: Duration) {
        if !self.ready() {
            return;
        }
        let Ok((_, mut state)) = self.levels.single_mut() else { return };
        let first = !state.entered;
        if first {
            state.entered = true;
        }
        // Authored order determines relay side effects and gameplay RNG use.
        let mut positioned: Vec<_> = self
            .devices
            .iter()
            .filter(|(entity, device)| {
                self.proximity.contains(*entity) && self.levels.get(device.level).is_ok()
            })
            .map(|(entity, device)| (device.definition, entity))
            .collect();
        positioned.sort_by_key(|&(index, _)| index);
        for (_, entity) in positioned {
            let definition = self.definition(entity).unwrap();
            let inside = (definition.range.0..definition.range.1)
                .contains(&eye.distance(definition.place.unwrap()));
            let mut proximity = self.proximity.get_mut(entity).unwrap();
            let entering = inside && !proximity.inside;
            if proximity.inside != inside {
                proximity.inside = inside;
            }
            let hurt = if !inside {
                if proximity.zone.is_some() {
                    proximity.zone = None;
                }
                false
            } else if proximity.zone.is_some() {
                proximity.zone.as_mut().unwrap().tick(elapsed).just_finished()
            } else {
                false
            };
            if hurt {
                self.damage_party(Dice { count: 1, sides: 6, base: 2 });
            }
            if entering && !first && self.party_move.0.is_none() && self.usable(entity) {
                self.run_device(entity, 0);
            }
        }
    }

    pub fn arrive(&mut self, eye: Vec3) {
        for (entity, _) in &self.devices {
            let Some(definition) = self.definition(entity) else { continue };
            let Some(place) = definition.place else { continue };
            let inside = (definition.range.0..definition.range.1).contains(&eye.distance(place));
            if let Ok(mut proximity) = self.proximity.get_mut(entity)
                && proximity.inside != inside
            {
                proximity.inside = inside;
            }
        }
    }

    fn sound_position(&self, entity: Entity) -> Option<Vec3> {
        let prop = self.devices.get(entity).ok()?.1.prop?;
        self.props.get(prop).ok().map(|(prop, _)| prop.centre)
    }

    fn playing(&self, entity: Entity) -> bool {
        self.devices
            .get(entity)
            .ok()
            .and_then(|(_, d)| d.prop)
            .and_then(|p| self.props.get(p).ok())
            .is_some_and(|(_, m)| m.frame != m.target)
    }

    /// Selection advances the alternate choice even if the action later fails
    /// (retail `SelectAction`). Merely assessing usability never selects.
    fn select<'a>(
        &mut self,
        entity: Entity,
        definition: &'a TriggerDefinition,
    ) -> Option<&'a TriggerUse> {
        let (_, device) = self.devices.get(entity).ok()?;
        let (_, variables) = self.levels.get(device.level).ok()?;
        let state = self.states.get(entity).ok()?;
        let running = state.awaits_animation && self.playing(entity);
        let shakes = matches!(definition.initial.action, TriggerAction::Shake { .. });
        if !state.on || (definition.once && state.fired) || (running && !shakes) {
            return None;
        }
        if !definition.required_states.iter().all(|name| variables.variable(name)) {
            return definition.fallback.as_ref();
        }
        if let Ok(door) = self.doors.get(entity)
            && door.locked
            && let Some(key) = definition.door.and_then(|d| d.key)
        {
            if self.party.held.as_ref().is_some_and(|h| h.item.item == key) {
                self.doors.get_mut(entity).unwrap().locked = false;
            } else {
                return definition.fallback.as_ref();
            }
        }
        let mut state = self.states.get_mut(entity).ok()?;
        match &definition.alternate {
            Some(alternate) if state.use_alternate => {
                let next = !definition.alternate_toggles;
                if state.use_alternate != next {
                    state.use_alternate = next;
                }
                Some(alternate)
            }
            alternate => {
                let next = alternate.is_some();
                if state.use_alternate != next {
                    state.use_alternate = next;
                }
                Some(&definition.initial)
            }
        }
    }

    fn run_device(&mut self, entity: Entity, depth: usize) -> bool {
        let Some(definition) = self.definition(entity) else { return false };
        if depth > MAX_DEPTH {
            warn!("trigger {} recurses too deeply", definition.name);
            return false;
        }
        let Some(chosen) = self.select(entity, definition) else { return false };
        if let Ok(bell) = self.bells.get(entity) {
            self.effects.write(bell.ring(entity, &self.dials.as_readonly(), &mut self.states));
        }
        let (_, device) = self.devices.get_inner(entity).unwrap();
        let (level, prop, recipients) = (device.level, device.prop, &device.recipients);
        let mut changes_state = true;
        match &chosen.action {
            TriggerAction::Nothing => {}
            action @ (TriggerAction::OpenDoor | TriggerAction::ToggleDoor) => {
                let Ok(door) = self.doors.get(entity) else { return false };
                if door.locked {
                    self.effects.write(TriggerEffect::Locked);
                    return false;
                }
                if self.playing(entity) {
                    return false;
                }
                let Some(prop) = prop else { return false };
                let Ok((prop, mut motion)) = self.props.get_mut(prop) else { return false };
                let mut door = self.doors.get_mut(entity).unwrap();
                if *action == TriggerAction::OpenDoor && door.open {
                    return false;
                }
                door.open = *action == TriggerAction::OpenDoor || !door.open;
                motion.target = if door.open { prop.last_frame as f32 } else { 0.0 };
                self.states.get_mut(entity).unwrap().awaits_animation = true;
            }
            TriggerAction::Animate(animation) => {
                let Some(prop) = prop.filter(|&p| self.props.contains(p)) else { return false };
                self.open_container(entity);
                let (prop, mut motion) = self.props.get_mut(prop).unwrap();
                let playing = motion.frame != motion.target;
                match animation {
                    Animation::Toggle if playing => motion.target = motion.frame,
                    Animation::Toggle | Animation::Start if !playing => {
                        if prop.wraps {
                            motion.frame = 0.0;
                            motion.target = prop.last_frame as f32;
                        } else {
                            motion.target = if motion.frame >= prop.last_frame as f32 {
                                0.0
                            } else {
                                prop.last_frame as f32
                            };
                        }
                    }
                    Animation::Stop if playing => motion.target = motion.frame,
                    _ => {}
                }
                if let Ok(mut door) = self.doors.get_mut(entity) {
                    door.open = !door.open;
                }
                self.states.get_mut(entity).unwrap().awaits_animation = true;
            }
            TriggerAction::Sound => {
                if let Some(sound) = &definition.sound {
                    let at = self.sound_position(entity);
                    self.effects.write(TriggerEffect::Sound {
                        path: sound.path.clone(),
                        volume: sound.volume,
                        at,
                    });
                }
                return false;
            }
            TriggerAction::Relay => {
                if recipients.is_empty() {
                    return false;
                }
                for &recipient in recipients {
                    self.run_device(recipient, depth + 1);
                }
            }
            TriggerAction::Teleport(arrival) => {
                self.party_move.0 = Some((level, PartyMove::Teleport(arrival.clone())));
            }
            TriggerAction::CameraPath(path) => {
                self.party_move.0 = Some((level, PartyMove::Fly(path.clone())));
            }
            TriggerAction::HurtParty(dice) => self.damage_party(*dice),
            TriggerAction::Fountain(fountain) => {
                if !self.use_fountain(entity, fountain) {
                    return false;
                }
            }
            TriggerAction::DamageZone => {
                if let Ok(mut proximity) = self.proximity.get_mut(entity) {
                    proximity.zone =
                        Some(Timer::from_seconds(DAMAGE_ZONE_PERIOD, TimerMode::Repeating));
                }
            }
            &TriggerAction::Shake { seconds, intensity, fade_out } => {
                self.effects.write(TriggerEffect::Shake { seconds, intensity, fade_out });
            }
            TriggerAction::Dial => {
                let Some(prop) = prop else { return false };
                let Ok((prop, mut motion)) = self.props.get_mut(prop) else { return false };
                let mut dial = self.dials.get_mut(entity).unwrap();
                let old = dial.position;
                let new = if prop.slots.len() < 3 {
                    0
                } else {
                    let next = (old + 1) % (prop.slots.len() - 1);
                    motion.frame = f32::from(prop.slots[next]);
                    motion.target = f32::from(prop.slots[next + 1]);
                    next
                };
                if old != new {
                    dial.position = new;
                }
                if let Some((name, _)) = &definition.state_change {
                    let (_, mut state) = self.levels.get_mut(level).unwrap();
                    state.set_variable(&format!("{name}{old}"), false);
                    state.set_variable(&format!("{name}{new}"), true);
                }
                changes_state = false;
            }
            TriggerAction::Unsupported(code) => {
                debug!("trigger {}: action {code:#x} is not implemented", definition.name);
                return false;
            }
        }
        if let Some(sound) = definition.sound.as_ref().filter(|_| definition.sound_on_commit) {
            let at = self.sound_position(entity);
            self.effects.write(TriggerEffect::Sound {
                path: sound.path.clone(),
                volume: sound.volume,
                at,
            });
        }
        if definition.fire_linked {
            for &recipient in recipients {
                self.run_device(recipient, depth + 1);
            }
        }
        if changes_state && let Some((name, change)) = &definition.state_change {
            let (_, mut state) = self.levels.get_mut(level).unwrap();
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
            self.effects.write(TriggerEffect::Message(message));
        }
        let mut state = self.states.get_mut(entity).unwrap();
        if !state.fired {
            state.fired = true;
        }
        true
    }

    fn damage_party(&mut self, dice: Dice) {
        for entity in self.roster.0.iter() {
            let Ok(member) = self.members.get(*entity) else { continue };
            if member.is_dead() {
                continue;
            }
            let mut member = self.members.get_mut(*entity).unwrap();
            let amount = member.damage(self.rng.roll(dice));
            self.effects.write(TriggerEffect::Hurt { member: member.name.clone(), amount });
        }
    }

    fn use_fountain(&mut self, entity: Entity, fountain: &Fountain) -> bool {
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
        for entity in self.roster.0.iter() {
            if !self.members.get(*entity).is_ok_and(|m| !m.is_dead()) {
                continue;
            }
            let mut member = self.members.get_mut(*entity).unwrap();
            match fountain.restores {
                Restores::Health => member.heal(fountain.amount),
                Restores::Stamina => member.restore_stamina(fountain.amount),
            }
        }
        let at = self.sound_position(entity);
        self.effects.write(TriggerEffect::Sound {
            path: "data/sound/misc/fountain_magic.wav".into(),
            volume: 1.0,
            at,
        });
        true
    }

    fn open_container(&mut self, entity: Entity) {
        let Ok(container) = self.containers.get(entity) else { return };
        let Some(table) = container.table else { return };
        let data = self.data.as_ref().unwrap();
        let catalog = self.catalogs.get(&data.items).unwrap();
        let loot = self.loot.get(&data.loot).unwrap();
        let mut container = self.containers.get_mut(entity).unwrap();
        let contents = container.contents.get_or_insert_with(|| {
            let (found, gold) = roll_loot(&loot.0[table], catalog, 1, &mut self.rng);
            let items = found
                .into_iter()
                .filter_map(|id| catalog.get(id))
                .map(|d| make_item(d, &mut self.ids, &mut self.rng, false, false))
                .collect();
            Contents { items, gold }
        });
        if contents.gold > 0 {
            self.party.gold += contents.gold;
            self.effects.write(TriggerEffect::Gold(contents.gold));
            contents.gold = 0;
        }
        let mut kept = Vec::new();
        for item in contents.items.drain(..) {
            let name = catalog.get(item.item).map(|d| d.name.clone()).unwrap_or_default();
            match self.party.place_in_pool(item, catalog) {
                Ok(()) => {
                    self.effects.write(TriggerEffect::PartyTakes(name));
                }
                Err(item) => kept.push(item),
            }
        }
        contents.items = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{ItemDefinition, ItemId, LootTable};
    use crate::content::level::{Arrival, DoorLock, PropModel, TriggerSound};
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::inventory::Held;

    fn act(action: TriggerAction) -> TriggerUse {
        TriggerUse { action, message: None }
    }

    fn trigger(name: &str, action: TriggerAction) -> TriggerDefinition {
        TriggerDefinition {
            name: name.into(),
            usable: false,
            once: false,
            fire_linked: false,
            initial: act(action),
            alternate: None,
            fallback: None,
            alternate_toggles: false,
            recipients: vec![],
            sound: None,
            sound_on_commit: false,
            required_states: vec![],
            state_change: None,
            initial_state: None,
            range: (0.0, 10.0),
            door: None,
            place: None,
            loot_table: None,
        }
    }

    /// Point each trigger's recipients at the named ones, as the decoder
    /// resolves them.
    fn link(triggers: &mut [TriggerDefinition], links: &[(&str, &[&str])]) {
        let index = |name: &str| triggers.iter().position(|t| t.name == name).unwrap();
        let resolved: Vec<(usize, Vec<usize>)> = links
            .iter()
            .map(|(from, to)| (index(from), to.iter().map(|t| index(t)).collect()))
            .collect();
        for (from, to) in resolved {
            triggers[from].recipients = to;
        }
    }

    /// A prop running `trigger`: its frames to the end, whether each play
    /// restarts, and its dial slots.
    struct PropSpec<'a>(&'a str, f32, bool, &'a [u16]);

    use bevy::ecs::system::SystemState;

    /// The actual ECS state and operation parameter, with no presentation.
    struct Scenario {
        world: World,
        operations: SystemState<DeviceOperations<'static, 'static>>,
        geometry: LevelGeometry,
        script: LevelScript,
        last_frames: Vec<f32>,
        root: Entity,
        devices: Vec<Entity>,
        members: Vec<Entity>,
    }

    impl Scenario {
        fn new(triggers: Vec<TriggerDefinition>, props: &[PropSpec], script: LevelScript) -> Self {
            let models = props
                .iter()
                .map(|&PropSpec(name, _, wraps, slots)| PropModel {
                    name: name.into(),
                    collidable: false,
                    face_camera: false,
                    frames_per_second: 10.0,
                    parts: vec![],
                    wraps,
                    slots: slots.to_vec(),
                    trigger: triggers.iter().position(|t| t.name == name),
                })
                .collect();
            let geometry = LevelGeometry { triggers, props: models, ..default() };
            let mut world = World::new();
            world.init_resource::<Assets<LevelGeometry>>();
            world.init_resource::<Assets<ItemCatalog>>();
            world.init_resource::<Assets<LootTables>>();
            world.init_resource::<Messages<TriggerEffect>>();
            world.init_resource::<PendingPartyMove>();
            world.init_resource::<PartyInventory>();
            world.init_resource::<ItemIds>();
            world.init_resource::<PartyRoster>();
            world.insert_resource(GameClock::at(0));
            world.insert_resource(GameRng::seeded(5));
            let items = world.resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog::default());
            let loot = world.resource_mut::<Assets<LootTables>>().add(LootTables::default());
            world.insert_resource(GameData { items, loot, species: default() });
            let root = world.spawn_empty().id();
            let operations = SystemState::new(&mut world);
            let mut scenario = Self {
                world,
                operations,
                geometry,
                script,
                last_frames: props.iter().map(|p| p.1).collect(),
                root,
                devices: vec![],
                members: vec![],
            };
            scenario.visit(None);
            scenario.add_member("A");
            scenario
        }

        fn visit(&mut self, memory: Option<&TriggerMemory>) {
            self.world.despawn(self.root);
            let handle =
                self.world.resource_mut::<Assets<LevelGeometry>>().add(self.geometry.clone());
            self.root = self
                .world
                .spawn(super::super::level::Level {
                    path: "test".into(),
                    geometry: handle,
                    script: self.script,
                    navmeshes: vec![],
                    sky: None,
                    messages: None,
                })
                .id();
            let props: Vec<_> = self
                .geometry
                .props
                .iter()
                .map(|_| self.world.spawn(ChildOf(self.root)).id())
                .collect();
            let tables = LootTables(
                self.world
                    .resource::<Assets<LootTables>>()
                    .get(&self.world.resource::<GameData>().loot)
                    .unwrap()
                    .0
                    .clone(),
            );
            let mut state = SystemState::<Commands>::new(&mut self.world);
            let mut commands = state.get_mut(&mut self.world).unwrap();
            self.devices = spawn_devices(
                &mut commands,
                self.root,
                &self.geometry,
                &props,
                self.script,
                &tables,
                memory,
            );
            for (index, model) in self.geometry.props.iter().enumerate() {
                commands.entity(props[index]).insert((
                    Prop {
                        definition: index,
                        trigger: model.trigger.map(|i| self.devices[i]),
                        centre: Vec3::ZERO,
                        door: false,
                        frames_per_second: 10.0,
                        last_frame: self.last_frames[index] as usize,
                        wraps: model.wraps,
                        slots: model.slots.clone(),
                    },
                    PropMotion::default(),
                ));
            }
            state.apply(&mut self.world);
        }

        fn device(&self, name: &str) -> Entity {
            self.devices[self.geometry.triggers.iter().position(|t| t.name == name).unwrap()]
        }

        fn add_member(&mut self, name: &str) -> Entity {
            let member = self
                .world
                .spawn(Character::new(name, Race::Human, Gender::Male, Profession::Fighter))
                .id();
            self.world.resource_mut::<PartyRoster>().0.insert(member);
            self.members.push(member);
            member
        }

        fn effects(&mut self) -> Vec<TriggerEffect> {
            self.world.resource_mut::<Messages<TriggerEffect>>().drain().collect()
        }

        fn run(&mut self, name: &str) -> (bool, Vec<TriggerEffect>) {
            let device = self.device(name);
            let committed = self.operations.get_mut(&mut self.world).unwrap().run(device);
            (committed, self.effects())
        }

        fn walk(&mut self, eye: Vec3, elapsed: Duration) -> Option<PartyMove> {
            let mut operations = self.operations.get_mut(&mut self.world).unwrap();
            operations.walk(eye, elapsed);
            operations.take_party_move()
        }

        fn motion(&self, name: &str) -> &PropMotion {
            let prop = self.world.get::<Device>(self.device(name)).unwrap().prop.unwrap();
            self.world.get::<PropMotion>(prop).unwrap()
        }

        fn settle(&mut self) {
            for mut motion in self.world.query::<&mut PropMotion>().iter_mut(&mut self.world) {
                motion.frame = motion.target;
            }
        }

        fn variable(&self, name: &str) -> bool {
            self.world.get::<LevelState>(self.root).unwrap().variable(name)
        }

        fn remember(&mut self) -> TriggerMemory {
            let mut records = SystemState::<DeviceRecords>::new(&mut self.world);
            records
                .get(&self.world)
                .unwrap()
                .remember(self.root, self.world.get::<LevelState>(self.root).unwrap())
        }
    }

    #[test]
    fn a_door_opens_then_toggles_and_a_locked_one_stays_shut() {
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.alternate = Some(act(TriggerAction::ToggleDoor));
        let mut locked = trigger("LOCKED", TriggerAction::OpenDoor);
        locked.door = Some(DoorLock { locked: true, key: None });
        let mut level = Scenario::new(
            vec![door, locked],
            &[PropSpec("DOOR", 15.0, false, &[]), PropSpec("LOCKED", 15.0, false, &[])],
            LevelScript::None,
        );
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        // Busy until the door finishes opening.
        assert!(!level.run("DOOR").0);
        level.settle();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 0.0);
        level.settle();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        assert_eq!(level.run("LOCKED"), (false, vec![TriggerEffect::Locked]));
        assert_eq!(level.motion("LOCKED").target, 0.0);
    }

    #[test]
    fn a_key_in_hand_unlocks_its_door_for_good_and_otherwise_the_fallback_runs() {
        let key = ItemId(7);
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.door = Some(DoorLock { locked: true, key: Some(key) });
        door.fallback = Some(TriggerUse { action: TriggerAction::Nothing, message: Some(3) });
        let mut level =
            Scenario::new(vec![door], &[PropSpec("DOOR", 15.0, false, &[])], LevelScript::None);
        // Without the key: the fallback, which shows its message.
        assert_eq!(level.run("DOOR"), (true, vec![TriggerEffect::Message(3)]));
        assert_eq!(level.motion("DOOR").target, 0.0);
        let definition = ItemDefinition { id: key, ..default() };
        let item =
            make_item(&definition, &mut ItemIds::default(), &mut GameRng::seeded(5), false, true);
        level.world.resource_mut::<PartyInventory>().held = Some(Held { item, origin: None });
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 15.0);
        // Unlocked for good: kept while the party is away.
        let memory = level.remember();
        let old = level.device("DOOR");
        level.visit(Some(&memory));
        assert_ne!(old, level.device("DOOR"));
        assert!(!level.world.get::<DoorState>(level.device("DOOR")).unwrap().locked);
    }

    #[test]
    fn a_container_opens_its_table_once_and_one_without_a_table_holds_nothing() {
        let chest = |name: &str, table: &str| {
            let mut chest = trigger(name, TriggerAction::Animate(Animation::Toggle));
            chest.loot_table = Some(table.into());
            chest
        };
        let props = [PropSpec("CHEST", 5.0, false, &[]), PropSpec("LOST", 5.0, false, &[])];
        let mut level = Scenario::new(
            vec![chest("CHEST", "Monastery Chest"), chest("LOST", "No Such Table")],
            &props,
            LevelScript::None,
        );
        // The tables are looked up when the level is prepared.
        let loot = LootTables(vec![LootTable {
            name: "MONASTERY CHEST".into(),
            entries: vec![],
            level_scaled: false,
            item_count: Dice::default(),
            gold: Dice { count: 0, sides: 0, base: 5 },
        }]);
        let handle = level.world.resource::<GameData>().loot.clone();
        level.world.resource_mut::<Assets<LootTables>>().insert(&handle, loot).unwrap();
        level.visit(None);
        assert_eq!(level.run("CHEST"), (true, vec![TriggerEffect::Gold(5)]));
        level.settle();
        assert_eq!(level.run("CHEST"), (true, vec![]), "its gold was taken");
        assert_eq!(level.run("LOST"), (true, vec![]));
        let lost = level.device("LOST");
        assert!(
            level.world.get::<Container>(lost).unwrap().contents.is_none(),
            "no contents are made up"
        );
        assert_eq!(level.world.resource::<PartyInventory>().gold, 5);
        let memory = level.remember();
        level.visit(Some(&memory));
        assert!(level.run("CHEST").1.is_empty(), "returning never rolls the gold again");
        level.visit(None);
        let chest = level.device("CHEST");
        level.world.entity_mut(chest).remove::<PropMotion>();
        assert!(!level.run("CHEST").0, "a missing prop cannot open a container");
        assert!(level.world.get::<Container>(chest).unwrap().contents.is_none());
        assert_eq!(level.world.resource::<PartyInventory>().gold, 5);
    }

    #[test]
    fn sounds_play_on_commit_only_when_marked_and_always_for_the_sound_action() {
        let sound = TriggerSound { path: "data/sound/click.wav".into(), volume: 1.0 };
        let mut quiet = trigger("QUIET", TriggerAction::Nothing);
        quiet.sound = Some(sound.clone());
        let mut loud = trigger("LOUD", TriggerAction::Nothing);
        loud.sound = Some(sound.clone());
        loud.sound_on_commit = true;
        let mut only = trigger("ONLY", TriggerAction::Sound);
        only.sound = Some(sound);
        let mut level = Scenario::new(vec![quiet, loud, only], &[], LevelScript::None);
        let click =
            TriggerEffect::Sound { path: "data/sound/click.wav".into(), volume: 1.0, at: None };
        assert_eq!(level.run("QUIET"), (true, vec![]));
        assert_eq!(level.run("LOUD"), (true, vec![click.clone()]));
        assert_eq!(level.run("ONLY"), (false, vec![click]), "plays without committing");
    }

    /// The upper monastery's bells, as authored (`docs/research/trigger-actions.md`).
    fn bells() -> Scenario {
        let toggle = || TriggerAction::Animate(Animation::Toggle);
        let mut dial = trigger("BELLRINGSWITCH", TriggerAction::Dial);
        dial.usable = true;
        dial.state_change = Some(("RING".into(), StateChange::Set));
        dial.initial_state = Some(false);
        let mut button = trigger("BELL_BUTTON", toggle());
        button.usable = true;
        button.fire_linked = true;
        let mut windows = trigger("BELLTRIGGER01", TriggerAction::Nothing);
        windows.once = true;
        windows.fire_linked = true;
        windows.required_states = vec!["RING2".into()];
        let relay = |name: &str, ring: &str| {
            let mut relay = trigger(name, TriggerAction::Relay);
            relay.fire_linked = true;
            relay.required_states = vec![ring.into()];
            relay
        };
        let shake = TriggerAction::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
        let mut triggers = vec![
            dial,
            button,
            windows,
            relay("BELLTRIGGERC", "RING0"),
            relay("BELL_ACTIVATE_2", "RING1"),
            relay("BELL_ACTIVATE_3", "RING2"),
            trigger("BELL_1", toggle()),
            trigger("BELL_2", toggle()),
            trigger("BELL_3", toggle()),
            trigger("GLASS1", toggle()),
            trigger("MONACAMSHAKE", shake),
        ];
        link(
            &mut triggers,
            &[
                (
                    "BELL_BUTTON",
                    &["BELLTRIGGER01", "BELLTRIGGERC", "BELL_ACTIVATE_2", "BELL_ACTIVATE_3"],
                ),
                ("BELLTRIGGER01", &["GLASS1", "MONACAMSHAKE"]),
                ("BELLTRIGGERC", &["BELL_1"]),
                ("BELL_ACTIVATE_2", &["BELL_2"]),
                ("BELL_ACTIVATE_3", &["BELL_3", "MONACAMSHAKE"]),
            ],
        );
        Scenario::new(
            triggers,
            &[
                PropSpec("BELLRINGSWITCH", 20.0, false, &[0, 1, 10, 20]),
                PropSpec("BELL_BUTTON", 11.0, false, &[]),
                PropSpec("BELL_1", 36.0, true, &[]),
                PropSpec("BELL_2", 36.0, true, &[]),
                PropSpec("BELL_3", 36.0, true, &[]),
                PropSpec("GLASS1", 2.0, false, &[]),
            ],
            LevelScript::Monastery2,
        )
    }

    fn bell_sound(n: u32) -> TriggerEffect {
        TriggerEffect::Sound {
            path: format!("data/sound/ambients/mon2bell{n}.wav"),
            volume: 1.0,
            at: None,
        }
    }

    #[test]
    fn each_dial_position_rings_its_own_bell() {
        let mut level = bells();
        assert!(level.variable("RING0"));
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").target, 36.0);
        assert_eq!(level.motion("BELL_2").target, 0.0);
        level.settle();
        assert!(level.run("BELLRINGSWITCH").0);
        assert!(!level.variable("RING0") && level.variable("RING1"));
        assert_eq!(
            (level.motion("BELLRINGSWITCH").frame, level.motion("BELLRINGSWITCH").target),
            (1.0, 10.0)
        );
        level.settle();
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(2)]);
        assert_eq!(level.motion("BELL_2").target, 36.0);
        assert_eq!(level.motion("GLASS1").target, 0.0, "the windows hold until the third bell");
    }

    #[test]
    fn the_third_bell_breaks_the_windows_once_and_ends_the_puzzle() {
        let mut level = bells();
        level.run("BELLRINGSWITCH");
        level.settle();
        level.run("BELLRINGSWITCH");
        level.settle();
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects[0], bell_sound(3));
        let shake = TriggerEffect::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
        assert!(effects.contains(&shake));
        assert_eq!(level.motion("GLASS1").target, 2.0);
        assert_eq!(level.motion("BELL_3").target, 36.0);
        level.settle();
        // Dial and button are off for good; the windows trigger has fired.
        assert_eq!(level.run("BELL_BUTTON"), (false, vec![]));
        assert_eq!(level.run("BELLRINGSWITCH"), (false, vec![]));
        let windows = level.device("BELLTRIGGER01");
        assert!(level.world.get::<TriggerState>(windows).unwrap().fired);
    }

    #[test]
    fn a_swinging_bell_is_not_restarted() {
        let mut level = bells();
        level.run("BELL_BUTTON");
        for mut motion in level.world.query::<&mut PropMotion>().iter_mut(&mut level.world) {
            motion.frame = (motion.target / 2.0).floor();
        }
        // The button finished; the bell is halfway.
        let button = level.device("BELL_BUTTON");
        let mut motion = level.world.get_mut::<PropMotion>(button).unwrap();
        motion.frame = motion.target;
        let bell = level.motion("BELL_1").frame;
        assert_eq!(level.run("BELL_BUTTON").1, [bell_sound(1)]);
        assert_eq!(level.motion("BELL_1").frame, bell);
        assert_eq!(level.motion("BELL_1").target, 36.0);
    }

    #[test]
    fn walk_in_triggers_fire_on_entering_when_usable() {
        let teleport = |name: &str, x: f32, usable: bool| {
            let arrival = Arrival::Entrance { entrance: format!("{name}00"), ask: false };
            let mut trigger = trigger(name, TriggerAction::Teleport(arrival));
            trigger.usable = usable;
            trigger.place = Some(Vec3::new(x, 0.0, 0.0));
            trigger.range = (0.0, 2.0);
            trigger
        };
        let triggers = vec![
            teleport("START", 0.0, true),
            teleport("EXIT", 10.0, true),
            teleport("OFF", 20.0, false),
        ];
        let mut level = Scenario::new(triggers, &[], LevelScript::None);
        let mut walk = |eye: Vec3| level.walk(eye, Duration::from_millis(100));
        // Standing in a trigger when the level is first seen does not fire it.
        assert!(walk(Vec3::ZERO).is_none());
        assert!(walk(Vec3::new(5.0, 0.0, 0.0)).is_none());
        assert!(matches!(
            walk(Vec3::new(10.5, 0.0, 0.0)),
            Some(PartyMove::Teleport(Arrival::Entrance { entrance, .. })) if entrance == "EXIT00"
        ));
        // Staying inside does not fire it again; a trigger the party may not
        // use never fires.
        assert!(walk(Vec3::new(10.0, 0.0, 0.0)).is_none());
        assert!(walk(Vec3::new(20.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn traps_hurt_each_member_by_their_own_roll_and_zones_hurt_while_stood_in() {
        let trap = trigger("TRAP", TriggerAction::HurtParty(Dice { count: 2, sides: 4, base: 0 }));
        let mut zone = trigger("ACID", TriggerAction::DamageZone);
        zone.usable = true;
        zone.place = Some(Vec3::ZERO);
        zone.range = (0.0, 2.0);
        let mut level = Scenario::new(vec![trap, zone], &[], LevelScript::None);
        level.add_member("B");
        let full = level.world.get::<Character>(level.members[0]).unwrap().hp;
        let tick = Duration::from_millis(100);
        let (committed, effects) = level.run("TRAP");
        assert!(committed);
        let hurt: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                TriggerEffect::Hurt { amount, .. } => Some(*amount),
                _ => None,
            })
            .collect();
        assert_eq!(hurt.len(), 2);
        assert!(hurt.iter().all(|n| (2..=8).contains(n)));
        level.walk(Vec3::new(5.0, 0.0, 0.0), tick);
        level.walk(Vec3::ZERO, tick);
        for _ in 0..10 {
            level.walk(Vec3::ZERO, tick);
        }
        let zone_hits =
            level.effects().iter().filter(|e| matches!(e, TriggerEffect::Hurt { .. })).count();
        assert_eq!(zone_hits, 2 * 2, "two ticks for each of two members");
        level.walk(Vec3::new(5.0, 0.0, 0.0), tick);
        level.effects();
        level.walk(Vec3::new(5.0, 0.0, 0.0), Duration::from_secs(1));
        assert!(level.effects().is_empty());
        assert!(level.members.iter().all(|&m| level.world.get::<Character>(m).unwrap().hp < full));
    }

    #[test]
    fn fountains_restore_until_dry_and_refill_with_game_time() {
        let fountain = Fountain {
            restores: Restores::Health,
            amount: 5,
            uses: Some(1),
            refills_after: Some(2 * 720 * 1000),
        };
        let mut level = Scenario::new(
            vec![trigger("WELL", TriggerAction::Fountain(fountain))],
            &[],
            LevelScript::None,
        );
        let full = level.world.get::<Character>(level.members[0]).unwrap().hp;
        level.world.get_mut::<Character>(level.members[0]).unwrap().hp = 1;
        assert!(level.run("WELL").0);
        assert!(!level.run("WELL").0, "dry after its one use");
        let memory = level.remember();
        level.visit(Some(&memory));
        assert!(!level.run("WELL").0, "still dry after returning");
        level.world.resource_mut::<GameClock>().ms = 2 * 720 * 1000;
        assert!(level.run("WELL").0);
        assert_eq!(level.world.get::<Character>(level.members[0]).unwrap().hp, 11.min(full));
    }

    #[test]
    fn assessment_and_door_use_leave_unrelated_components_unchanged() {
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.usable = true;
        let mut level = Scenario::new(
            vec![door, trigger("OTHER", TriggerAction::Animate(Animation::Toggle))],
            &[PropSpec("DOOR", 10.0, false, &[]), PropSpec("OTHER", 10.0, false, &[])],
            LevelScript::None,
        );
        let door = level.device("DOOR");
        let other = level.device("OTHER");
        level.world.clear_trackers();
        {
            let mut operations = level.operations.get_mut(&mut level.world).unwrap();
            assert!(operations.usable(door));
            assert!(!operations.use_device(door, 100.0));
        }
        assert_eq!(
            level
                .world
                .query_filtered::<Entity, Changed<TriggerState>>()
                .iter(&level.world)
                .count(),
            0
        );
        assert_eq!(
            level.world.query_filtered::<Entity, Changed<PropMotion>>().iter(&level.world).count(),
            0
        );
        assert_eq!(
            level.world.query_filtered::<Entity, Changed<Character>>().iter(&level.world).count(),
            0
        );
        assert!(level.operations.get_mut(&mut level.world).unwrap().use_device(door, 1.0));
        let changed: Vec<_> = level
            .world
            .query_filtered::<Entity, Changed<PropMotion>>()
            .iter(&level.world)
            .collect();
        assert_eq!(changed, [door]);
        assert!(!changed.contains(&other));
        assert_eq!(
            level.world.query_filtered::<Entity, Changed<Character>>().iter(&level.world).count(),
            0
        );
        assert_eq!(
            level.world.query_filtered::<Entity, Changed<LevelState>>().iter(&level.world).count(),
            0
        );
    }

    #[test]
    fn relay_recipients_complete_in_order_before_the_senders_commit() {
        let mut first = trigger("FIRST", TriggerAction::Nothing);
        first.state_change = Some(("READY".into(), StateChange::Set));
        first.initial.message = Some(1);
        let mut second = trigger("SECOND", TriggerAction::Nothing);
        second.required_states.push("READY".into());
        second.initial.message = Some(2);
        let mut relay = trigger("RELAY", TriggerAction::Relay);
        relay.recipients = vec![0, 1];
        relay.initial.message = Some(3);
        let mut level = Scenario::new(vec![first, second, relay], &[], LevelScript::None);
        assert_eq!(
            level.run("RELAY"),
            (
                true,
                vec![
                    TriggerEffect::Message(1),
                    TriggerEffect::Message(2),
                    TriggerEffect::Message(3)
                ]
            )
        );
    }

    #[test]
    fn damage_uses_roster_order_and_leaves_nonmembers_alone() {
        let dice = Dice { count: 2, sides: 4, base: 0 };
        let mut level = Scenario::new(
            vec![trigger("TRAP", TriggerAction::HurtParty(dice))],
            &[],
            LevelScript::None,
        );
        let outsider = level
            .world
            .spawn(Character::new("NPC", Race::Human, Gender::Male, Profession::Fighter))
            .id();
        let outsider_hp = level.world.get::<Character>(outsider).unwrap().hp;
        let second = level.add_member("B");
        let first = level.members[0];
        level.world.resource_mut::<PartyRoster>().0 = [second, first].into_iter().collect();
        level.world.clear_trackers();
        let mut expected = GameRng::seeded(5);
        let amounts = [expected.roll(dice), expected.roll(dice)];
        assert_eq!(
            level.run("TRAP").1,
            vec![
                TriggerEffect::Hurt { member: "B".into(), amount: amounts[0] },
                TriggerEffect::Hurt { member: "A".into(), amount: amounts[1] },
            ]
        );
        assert_eq!(level.world.get::<Character>(outsider).unwrap().hp, outsider_hp);
        let changed: Vec<_> =
            level.world.query_filtered::<Entity, Changed<Character>>().iter(&level.world).collect();
        assert!(!changed.contains(&outsider));
    }

    #[test]
    fn missing_catalogs_and_departed_levels_cannot_partially_run_a_device() {
        let mut definition = trigger("DOOR", TriggerAction::OpenDoor);
        definition.alternate = Some(act(TriggerAction::ToggleDoor));
        let mut level = Scenario::new(
            vec![definition],
            &[PropSpec("DOOR", 10.0, false, &[])],
            LevelScript::None,
        );
        let data = level.world.remove_resource::<GameData>().unwrap();
        level.world.clear_trackers();
        assert!(!level.run("DOOR").0);
        assert_eq!(
            level
                .world
                .query_filtered::<Entity, Changed<TriggerState>>()
                .iter(&level.world)
                .count(),
            0
        );
        level.world.insert_resource(data);
        level.world.entity_mut(level.root).insert(Departing);
        assert!(!level.run("DOOR").0);
        level.world.entity_mut(level.root).remove::<Departing>();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 10.0);
    }

    #[test]
    fn props_share_a_device_only_when_the_authored_trigger_is_shared() {
        let mut level = Scenario::new(
            vec![
                trigger("SHARED", TriggerAction::Nothing),
                trigger("ALONE", TriggerAction::Nothing),
            ],
            &[
                PropSpec("SHARED", 0.0, false, &[]),
                PropSpec("SHARED", 0.0, false, &[]),
                PropSpec("ALONE", 0.0, false, &[]),
            ],
            LevelScript::None,
        );
        let shared = level.device("SHARED");
        let alone = level.device("ALONE");
        assert!(level.world.get::<Prop>(shared).is_none());
        assert_eq!(level.world.get::<Prop>(alone).unwrap().trigger, Some(alone));
        assert_eq!(
            level
                .world
                .query::<&Prop>()
                .iter(&level.world)
                .filter(|p| p.trigger == Some(shared))
                .count(),
            2
        );
        assert_eq!(level.world.query::<&Device>().iter(&level.world).count(), 2);
        let old = level.devices.clone();
        let memory = level.remember();
        level.visit(Some(&memory));
        for entity in old {
            assert!(level.world.get_entity(entity).is_err());
        }
    }
    #[test]
    fn a_pending_move_cannot_be_applied_to_a_later_visit() {
        let arrival = Arrival::Here { eye: Vec3::X, yaw: 0.0 };
        let mut level = Scenario::new(
            vec![trigger("PORTAL", TriggerAction::Teleport(arrival))],
            &[],
            LevelScript::None,
        );
        assert!(level.run("PORTAL").0);
        level.visit(None);
        assert!(level.operations.get_mut(&mut level.world).unwrap().take_party_move().is_none());
        assert!(level.run("PORTAL").0);
        assert!(level.operations.get_mut(&mut level.world).unwrap().take_party_move().is_some());
    }
}
