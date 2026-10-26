//! Live devices and synchronous trigger operations. Definitions stay in the
//! level asset; runtime links and mutable state belong to device entities.
//!
//! An action commits its sound, ordered recipients, location variable, message
//! and fired flag synchronously. A relay also runs its recipients as its action,
//! preserving retail's double execution when it is marked to fire on commit.
//! See `docs/research/trigger-actions.md`.

use std::collections::HashMap;
use std::time::Duration;

use bevy::ecs::system::{InRef, SystemParam};
use bevy::prelude::*;

use super::level::{Departing, Level};
use super::props::{Prop, PropMotion};
use super::scripts::{BellButton, LevelScript};
use crate::content::items::{Dice, ItemCatalog, LootTables};
use crate::content::level::{
    Fountain, LevelGeometry, Restores, StateChange, TriggerAction, TriggerDefinition, TriggerUse,
};
use crate::game::characters::{Character, Vitals};
use crate::game::data::GameData;
use crate::game::inventory::{ItemIds, ItemInstance, PartyInventory, make_item};
use crate::game::loot::roll_loot;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

const DAMAGE_ZONE_PERIOD: f32 = 0.5;
const MAX_DEPTH: usize = 16;

/// A device's immutable definition and links, resolved once per visit.
/// A prop-backed device lives on that prop; positioned devices stand alone.
#[derive(Component, Debug)]
pub struct Device {
    pub level: Entity,
    pub definition: usize,
    pub recipients: Vec<Entity>,
    required_states: Vec<usize>,
    state_change: Option<(usize, StateChange)>,
    dial_states: Vec<usize>,
}

/// Authored traversal order, resolved once per visit; not an owner of device state.
#[derive(Component, Debug, Default)]
pub struct PositionedDevices(Vec<Entity>);

/// Only level-wide variables and visit state live on the level.
#[derive(Component, Debug, Default)]
pub struct LevelState {
    variables: Vec<(String, bool)>,
    entered: bool,
}

impl LevelState {
    // Activation resolves names; normal execution reads/writes the level's value by key.
    fn resolve(&mut self, name: &str, initial: bool) -> usize {
        let name = name.to_uppercase();
        if let Some(key) = self.variables.iter().position(|(n, _)| *n == name) {
            return key;
        }
        let key = self.variables.len();
        self.variables.push((name, initial));
        key
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
#[derive(Debug, Default)]
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
        TriggerMemory { devices, variables: state.variables.iter().cloned().collect() }
    }
}

/// Allocate identities before linking devices or building prop children.
pub fn spawn_devices(
    commands: &mut Commands,
    level: Entity,
    geometry: &LevelGeometry,
    props: &[Entity],
    script: LevelScript,
    tables: &LootTables,
    mut memory: Option<TriggerMemory>,
) -> Vec<Entity> {
    let mut attached = vec![None; geometry.triggers.len()];
    for (index, prop) in geometry.props.iter().enumerate() {
        if let Some(trigger) = prop.trigger {
            assert!(
                attached[trigger].replace(props[index]).is_none(),
                "a trigger belongs to one prop"
            );
        }
    }
    let devices: Vec<Entity> = attached
        .into_iter()
        .map(|prop| prop.unwrap_or_else(|| commands.spawn(ChildOf(level)).id()))
        .collect();
    let restored = memory.is_some();
    let mut state = LevelState::default();
    if let Some(memory) = &mut memory {
        state.variables = std::mem::take(&mut memory.variables).into_iter().collect();
    } else {
        for definition in &geometry.triggers {
            if let (Some((name, _)), Some(value)) =
                (&definition.state_change, definition.initial_state)
            {
                state.resolve(name, value);
            }
        }
    }
    let mut dial_states = vec![Vec::new(); geometry.triggers.len()];
    for prop in &geometry.props {
        let Some(index) = prop.trigger else { continue };
        let definition = &geometry.triggers[index];
        let Some((name, _)) = &definition.state_change else { continue };
        if std::iter::once(&definition.initial)
            .chain(definition.alternate.iter())
            .chain(definition.fallback.iter())
            .any(|action| matches!(action.action, TriggerAction::Dial))
        {
            dial_states[index] = (0..prop.slots.len().max(1))
                .map(|position| {
                    state.resolve(
                        &format!("{name}{position}"),
                        !restored
                            && position == 0
                            && matches!(definition.initial.action, TriggerAction::Dial),
                    )
                })
                .collect();
        }
    }
    let mut positioned = PositionedDevices::default();
    for (index, definition) in geometry.triggers.iter().enumerate() {
        let mut kept = memory.as_mut().and_then(|m| m.devices.remove(&index));
        let mut entity = commands.entity(devices[index]);
        entity.insert((
            Device {
                level,
                definition: index,
                recipients: definition.recipients.iter().map(|&i| devices[i]).collect(),
                required_states: definition
                    .required_states
                    .iter()
                    .map(|name| state.resolve(name, false))
                    .collect(),
                state_change: definition
                    .state_change
                    .as_ref()
                    .map(|(name, change)| (state.resolve(name, false), *change)),
                dial_states: std::mem::take(&mut dial_states[index]),
            },
            TriggerState {
                on: kept.as_ref().is_none_or(|k| k.on),
                fired: kept.as_ref().is_some_and(|k| k.fired),
                use_alternate: kept.as_ref().is_some_and(|k| k.use_alternate),
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
            entity.insert(kept.as_mut().and_then(|k| k.door.take()).unwrap_or(DoorState {
                open: false,
                locked: definition.door.is_some_and(|d| d.locked),
            }));
        }
        if actions.clone().any(|b| matches!(b.action, TriggerAction::Dial)) {
            entity.insert(kept.as_mut().and_then(|k| k.dial.take()).unwrap_or_default());
        }
        if actions.clone().any(|b| matches!(b.action, TriggerAction::Fountain(_))) {
            entity.insert(kept.as_mut().and_then(|k| k.fountain.take()).unwrap_or_default());
        }
        if let Some(name) = &definition.loot_table {
            let table = tables.0.iter().position(|t| t.name.eq_ignore_ascii_case(name));
            if table.is_none() {
                warn!("trigger {}: no loot table {name}; it holds nothing", definition.name);
            }
            entity.insert(Container {
                table,
                contents: kept.as_mut().and_then(|k| k.contents.take()),
            });
        }
        if definition.place.is_some() {
            entity.insert(DeviceProximity::default());
            positioned.0.push(devices[index]);
        }
    }
    commands.entity(level).insert((state, positioned));
    script.bind(commands, geometry, &devices);
    devices
}

fn open_container(
    In(entity): In<Entity>,
    mut containers: Query<&mut Container>,
    data: Res<GameData>,
    catalogs: Res<Assets<ItemCatalog>>,
    tables: Res<Assets<LootTables>>,
    mut party: ResMut<PartyInventory>,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    mut outcomes: MessageWriter<TriggerEffect>,
    mut item_news: MessageWriter<crate::game::inventory::ItemNews>,
) {
    let mut container = containers.get_mut(entity).unwrap();
    let catalog = catalogs.get(&data.items).unwrap();
    let loot = tables.get(&data.loot).unwrap();
    let Some(table) = container.table else { return };
    let contents = container.contents.get_or_insert_with(|| {
        let (found, gold) = roll_loot(&loot.0[table], catalog, 1, &mut rng);
        let items = found
            .into_iter()
            .filter_map(|id| catalog.get(id))
            .map(|d| make_item(d, &mut ids, &mut rng, false, false))
            .collect();
        Contents { items, gold }
    });
    if contents.gold > 0 {
        party.gold += contents.gold;
        outcomes.write(TriggerEffect::Gold(contents.gold));
        contents.gold = 0;
    }
    for (name, _) in crate::game::inventory::operations::transfer_contents(
        &mut contents.items,
        &mut party,
        catalog,
    ) {
        outcomes.write(TriggerEffect::PartyTakes(name));
    }
    if !contents.items.is_empty() {
        item_news.write(crate::game::inventory::ItemNews::NoRoom);
    }
}

fn damage_party(
    In(dice): In<Dice>,
    roster: Res<PartyRoster>,
    mut members: Query<&mut Vitals>,
    mut rng: ResMut<GameRng>,
    mut formation: ResMut<crate::game::party::formation::Formation>,
    mut outcomes: MessageWriter<TriggerEffect>,
) {
    for entity in roster.0.iter() {
        let Ok(member) = members.get(*entity) else { continue };
        if member.is_dead() {
            continue;
        }
        let rolled = rng.roll(dice);
        let amount = if rolled > 0 {
            let mut member = members.get_mut(*entity).unwrap();
            crate::game::party::damage_member(*entity, &mut member, &mut formation, rolled)
        } else {
            0
        };
        outcomes.write(TriggerEffect::Hurt { member: *entity, amount });
    }
}

fn drink_fountain(
    In((entity, fountain, at)): In<(Entity, Fountain, Option<Vec3>)>,
    mut fountains: Query<&mut FountainState>,
    clock: Res<GameClock>,
    roster: Res<PartyRoster>,
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
    for entity in roster.0.iter() {
        let Ok((profile, member)) = members.get(*entity) else { continue };
        if member.is_dead()
            || match fountain.restores {
                Restores::Health => member.hp >= profile.max_hp(),
                Restores::Stamina => {
                    member.stamina >= profile.max_stamina()
                        && !member.conditions.contains(&(
                            crate::game::characters::Condition::Unconscious,
                            crate::game::characters::Lasting::Indefinite,
                        ))
                }
            }
        {
            continue;
        }
        let (profile, mut member) = members.get_mut(*entity).unwrap();
        match fountain.restores {
            Restores::Health => member.heal(fountain.amount, profile.max_hp()),
            Restores::Stamina => member.restore_stamina(fountain.amount, profile.max_stamina()),
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
    /// A member was hurt.
    Hurt {
        member: Entity,
        amount: u32,
    },
    /// An item went to the party ("Party takes %s").
    PartyTakes(String),
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

/// A move requested by a device, scoped to its originating level.
/// Taking the move discards it if that level is no longer active.
#[derive(Resource, Debug, Default)]
pub struct PendingPartyMove(pub Option<(Entity, PartyMove)>);

impl PendingPartyMove {
    pub fn take_for(&mut self, level: Entity) -> Option<PartyMove> {
        let (origin, movement) = self.0.take()?;
        (origin == level).then_some(movement)
    }
}

/// Complete a device request synchronously. A hit distance means player use;
/// linked/script activation does not apply the player's reach/usable gate.
pub fn activate(world: &mut World, entity: Entity, distance: Option<f32>) -> bool {
    if distance.is_some()
        && !world.run_system_cached(crate::game::screen::party_free).unwrap_or(false)
    {
        return false;
    }
    let Some(device) = world.get::<Device>(entity) else { return false };
    let level = device.level;
    if world.get::<Departing>(level).is_some() {
        return false;
    }
    let Some(level) = world.get::<Level>(level) else { return false };
    let handle = level.geometry.clone();
    // Borrow immutable definitions for the entire synchronous operation. Nested
    // links use these same definitions, without cloning a graph or a definition.
    world.resource_scope(|world, geometry: Mut<Assets<LevelGeometry>>| {
        let Some(geometry) = geometry.get(&handle) else { return false };
        let definition = &geometry.triggers[world.get::<Device>(entity).unwrap().definition];
        if distance.is_some_and(|distance| {
            !distance.is_finite()
                || !usable(world, entity, definition)
                || !(definition.range.0..=definition.range.1).contains(&distance)
        }) || !ready(world, entity)
        {
            return false;
        }
        let committed = execute(world, geometry, entity, 0);
        if distance.is_some() && !committed {
            world.write_message(TriggerEffect::NothingHappened);
        }
        committed
    })
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
    world.get_resource::<GameData>().is_some_and(|data| {
        world.get_resource::<Assets<ItemCatalog>>().is_some_and(|a| a.contains(&data.items))
            && world.get_resource::<Assets<LootTables>>().is_some_and(|a| a.contains(&data.loot))
    })
}

fn select_use(
    (In(entity), InRef(definition)): (In<Entity>, InRef<TriggerDefinition>),
    mut devices: Query<(&Device, &mut TriggerState, Option<&mut DoorState>, Option<&PropMotion>)>,
    levels: Query<&LevelState>,
    inventory: Option<Res<PartyInventory>>,
) -> Option<TriggerUse> {
    let (device, mut state, door, motion) = devices.get_mut(entity).ok()?;
    let running = state.awaits_animation && motion.is_some_and(|m| m.frame != m.target);
    if !state.on
        || (definition.once && state.fired)
        || (running && !matches!(definition.initial.action, TriggerAction::Shake { .. }))
    {
        return None;
    }
    let variables = levels.get(device.level).ok()?;
    if !device.required_states.iter().all(|&key| variables.variables[key].1) {
        return definition.fallback.clone();
    }
    if let Some(mut door) = door
        && door.locked
        && let Some(key) = definition.door.and_then(|d| d.key)
    {
        if inventory.as_ref().and_then(|i| i.held.as_ref()).is_some_and(|h| h.item.item == key) {
            door.locked = false;
        } else {
            return definition.fallback.clone();
        }
    }
    let (chosen, next) = match &definition.alternate {
        Some(alternate) if state.use_alternate => (alternate, !definition.alternate_toggles),
        alternate => (&definition.initial, alternate.is_some()),
    };
    if state.use_alternate != next {
        state.use_alternate = next;
    }
    Some(chosen.clone())
}

fn operate_door(
    In((entity, open_only)): In<(Entity, bool)>,
    mut doors: Query<&mut DoorState>,
    mut props: Query<(&Prop, &mut PropMotion, &mut TriggerState)>,
    mut outcomes: MessageWriter<TriggerEffect>,
) -> bool {
    let Ok(mut door) = doors.get_mut(entity) else { return false };
    if door.locked {
        outcomes.write(TriggerEffect::Locked);
        return false;
    }
    let Ok((prop, mut motion, mut trigger)) = props.get_mut(entity) else { return false };
    let open = open_only || !door.open;
    if motion.frame != motion.target || door.open == open {
        return false;
    }
    door.open = open;
    motion.target = if open { prop.last_frame as f32 } else { 0.0 };
    trigger.awaits_animation = true;
    true
}

fn animate_device(
    In((entity, animation)): In<(Entity, crate::content::level::Animation)>,
    mut props: Query<(&Prop, &mut PropMotion, Option<&mut DoorState>, &mut TriggerState)>,
) {
    let (prop, motion, door, mut state) = props.get_mut(entity).unwrap();
    PropMotion::activate(motion, prop, animation);
    if let Some(mut door) = door {
        door.open = !door.open;
    }
    state.awaits_animation = true;
}

fn turn_dial(
    In(entity): In<Entity>,
    mut dials: Query<(&Device, &mut DialState, &Prop, &mut PropMotion)>,
    mut levels: Query<&mut LevelState>,
) -> bool {
    let Ok((device, mut dial, prop, mut motion)) = dials.get_mut(entity) else { return false };
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
    if !device.dial_states.is_empty() {
        let mut level = levels.get_mut(device.level).unwrap();
        level.variables[device.dial_states[old]].1 = false;
        level.variables[device.dial_states[next]].1 = true;
    }
    true
}

fn ring_bell(
    In(entity): In<Entity>,
    bells: Query<&BellButton>,
    dials: Query<&DialState>,
    mut states: Query<&mut TriggerState>,
) -> Option<TriggerEffect> {
    Some(bells.get(entity).ok()?.ring(entity, &dials, &mut states))
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

fn execute_links(world: &mut World, geometry: &LevelGeometry, entity: Entity, depth: usize) {
    let count = world.get::<Device>(entity).unwrap().recipients.len();
    for index in 0..count {
        let recipient = world.get::<Device>(entity).unwrap().recipients[index];
        execute(world, geometry, recipient, depth + 1);
    }
}

fn execute(world: &mut World, geometry: &LevelGeometry, entity: Entity, depth: usize) -> bool {
    if !ready(world, entity) {
        return false;
    }
    let Some(device) = world.get::<Device>(entity) else { return false };
    let level = device.level;
    let definition = &geometry.triggers[device.definition];
    if depth > MAX_DEPTH {
        warn!("trigger {} recurses too deeply", definition.name);
        return false;
    }
    let Some(chosen) = world.run_system_cached_with(select_use, (entity, definition)).unwrap()
    else {
        return false;
    };
    if let Some(outcome) = world.run_system_cached_with(ring_bell, entity).unwrap() {
        world.write_message(outcome);
    }
    let changes_state = !matches!(chosen.action, TriggerAction::Dial);
    match chosen.action {
        TriggerAction::Nothing => {}
        action @ (TriggerAction::OpenDoor | TriggerAction::ToggleDoor) => {
            if !world
                .run_system_cached_with(operate_door, (entity, action == TriggerAction::OpenDoor))
                .unwrap()
            {
                return false;
            }
        }
        TriggerAction::Animate(animation) => {
            if world.get::<Prop>(entity).is_none() || world.get::<PropMotion>(entity).is_none() {
                return false;
            }
            if world.get::<Container>(entity).is_some() {
                world.run_system_cached_with(open_container, entity).unwrap();
            }
            world.run_system_cached_with(animate_device, (entity, animation)).unwrap();
        }
        TriggerAction::Sound => {
            play_authored_sound(world, entity, definition);
            return false;
        }
        TriggerAction::Relay => {
            if world.get::<Device>(entity).unwrap().recipients.is_empty() {
                return false;
            }
            execute_links(world, geometry, entity, depth);
        }
        TriggerAction::Teleport(arrival) => {
            world.resource_mut::<PendingPartyMove>().0 =
                Some((level, PartyMove::Teleport(arrival)));
        }
        TriggerAction::CameraPath(path) => {
            world.resource_mut::<PendingPartyMove>().0 = Some((level, PartyMove::Fly(path)));
        }
        TriggerAction::HurtParty(dice) => {
            world.run_system_cached_with(damage_party, dice).unwrap();
        }
        TriggerAction::Fountain(fountain) => {
            let at = sound_position(world, entity, definition);
            if !world.run_system_cached_with(drink_fountain, (entity, fountain, at)).unwrap() {
                return false;
            }
        }
        TriggerAction::DamageZone => {
            if let Some(mut proximity) = world.get_mut::<DeviceProximity>(entity) {
                proximity.zone =
                    Some(Timer::from_seconds(DAMAGE_ZONE_PERIOD, TimerMode::Repeating));
            }
        }
        TriggerAction::Shake { seconds, intensity, fade_out } => {
            world.write_message(TriggerEffect::Shake { seconds, intensity, fade_out });
        }
        TriggerAction::Dial => {
            if !world.run_system_cached_with(turn_dial, entity).unwrap() {
                return false;
            }
        }
        TriggerAction::Unsupported(code) => {
            debug!("trigger {}: action {code:#x} is not implemented", definition.name);
            return false;
        }
    }
    if definition.sound_on_commit {
        play_authored_sound(world, entity, definition);
    }
    if definition.fire_linked {
        execute_links(world, geometry, entity, depth);
    }
    if changes_state && let Some((key, change)) = world.get::<Device>(entity).unwrap().state_change
    {
        let mut state = world.get_mut::<LevelState>(level).unwrap();
        let value = match change {
            StateChange::Set => true,
            StateChange::Clear => false,
            StateChange::Toggle => !state.variables[key].1,
        };
        if state.variables[key].1 != value {
            state.variables[key].1 = value;
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
    true
}

fn begin_walk(
    mut levels: Query<
        (Entity, &Level, &mut LevelState),
        (With<PositionedDevices>, Without<Departing>),
    >,
) -> Option<(Entity, Handle<LevelGeometry>, bool)> {
    let (entity, level, mut state) = levels.single_mut().ok()?;
    let first = !state.entered;
    if first {
        state.entered = true;
    }
    Some((entity, level.geometry.clone(), first))
}

fn step_proximity(
    (In(entity), InRef(definition), In((eye, elapsed))): (
        In<Entity>,
        InRef<TriggerDefinition>,
        In<(Vec3, Duration)>,
    ),
    mut proximity: Query<&mut DeviceProximity>,
) -> (bool, bool) {
    let inside =
        (definition.range.0..definition.range.1).contains(&eye.distance(definition.place.unwrap()));
    let mut proximity = proximity.get_mut(entity).unwrap();
    let entering = inside && !proximity.inside;
    if proximity.inside != inside {
        proximity.inside = inside;
    }
    let hurt = if !inside {
        if proximity.zone.is_some() {
            proximity.zone = None;
        }
        false
    } else {
        proximity.zone.as_mut().is_some_and(|zone| zone.tick(elapsed).just_finished())
    };
    (entering, hurt)
}

pub fn walk(world: &mut World, eye: Vec3, elapsed: Duration) {
    let Some((level, handle, first)) = world.run_system_cached(begin_walk).unwrap() else { return };
    world.resource_scope(|world, geometry: Mut<Assets<LevelGeometry>>| {
        let Some(geometry) = geometry.get(&handle) else { return };
        let count = world.get::<PositionedDevices>(level).unwrap().0.len();
        for index in 0..count {
            let entity = world.get::<PositionedDevices>(level).unwrap().0[index];
            let definition = &geometry.triggers[world.get::<Device>(entity).unwrap().definition];
            let (entering, hurt) = world
                .run_system_cached_with(step_proximity, (entity, definition, (eye, elapsed)))
                .unwrap();
            if hurt {
                world
                    .run_system_cached_with(damage_party, Dice { count: 1, sides: 6, base: 2 })
                    .unwrap();
            }
            if entering
                && !first
                && world.resource::<PendingPartyMove>().0.is_none()
                && usable(world, entity, definition)
            {
                execute(world, geometry, entity, 0);
            }
        }
    });
}

pub(crate) fn arrive(
    eye: Vec3,
    positioned: &PositionedDevices,
    geometry: &LevelGeometry,
    devices: &Query<&Device>,
    proximity: &mut Query<&mut DeviceProximity>,
) {
    for &entity in &positioned.0 {
        let definition = &geometry.triggers[devices.get(entity).unwrap().definition];
        let inside = (definition.range.0..definition.range.1)
            .contains(&eye.distance(definition.place.unwrap()));
        if let Ok(mut proximity) = proximity.get_mut(entity)
            && proximity.inside != inside
        {
            proximity.inside = inside;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{ItemDefinition, ItemId, LootTable};
    use crate::content::level::{Animation, Arrival, DoorLock, PropModel, TriggerSound};
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
            world.init_resource::<crate::game::party::formation::Formation>();
            world.init_resource::<State<crate::game::screen::Screen>>();
            world.init_resource::<Messages<crate::game::inventory::ItemNews>>();
            world.init_resource::<ItemIds>();
            world.init_resource::<PartyRoster>();
            world.insert_resource(GameClock::at(0));
            world.insert_resource(GameRng::seeded(5));
            let items = world.resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog::default());
            let loot = world.resource_mut::<Assets<LootTables>>().add(LootTables::default());
            world.insert_resource(GameData { items, loot, species: default() });
            let root = world.spawn_empty().id();
            let mut scenario = Self {
                world,
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

        fn visit(&mut self, memory: Option<TriggerMemory>) {
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
            let profile = Character::new(name, Race::Human, Gender::Male, Profession::Fighter);
            let member = self.world.spawn((Vitals::new(&profile), profile)).id();
            self.world.resource_mut::<PartyRoster>().0.insert(member);
            self.world.resource_mut::<crate::game::party::formation::Formation>().seat(member);
            self.members.push(member);
            member
        }

        fn effects(&mut self) -> Vec<TriggerEffect> {
            self.world.resource_mut::<Messages<TriggerEffect>>().drain().collect()
        }

        fn run(&mut self, name: &str) -> (bool, Vec<TriggerEffect>) {
            let device = self.device(name);
            let committed = activate(&mut self.world, device, None);
            (committed, self.effects())
        }

        fn walk(&mut self, eye: Vec3, elapsed: Duration) -> Option<PartyMove> {
            walk(&mut self.world, eye, elapsed);
            self.world.resource_mut::<PendingPartyMove>().take_for(self.root)
        }

        fn motion(&self, name: &str) -> &PropMotion {
            self.world.get::<PropMotion>(self.device(name)).unwrap()
        }

        fn settle(&mut self) {
            for mut motion in self.world.query::<&mut PropMotion>().iter_mut(&mut self.world) {
                motion.frame = motion.target;
            }
        }

        fn variable(&self, name: &str) -> bool {
            self.world
                .get::<LevelState>(self.root)
                .unwrap()
                .variables
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .is_some_and(|(_, value)| *value)
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
        assert_eq!(level.run("DOOR"), (true, vec![TriggerEffect::Message { source: None, id: 3 }]));
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
        level.visit(Some(memory));
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
        level.visit(Some(memory));
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

    #[test]
    fn each_dial_position_rings_its_own_bell() {
        let mut level = bells();
        assert!(level.variable("RING0"));
        let (committed, effects) = level.run("BELL_BUTTON");
        assert!(committed);
        assert_eq!(effects, [TriggerEffect::BellRang(1)]);
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
        assert_eq!(level.run("BELL_BUTTON").1, [TriggerEffect::BellRang(2)]);
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
        assert_eq!(effects[0], TriggerEffect::BellRang(3));
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
        assert_eq!(level.run("BELL_BUTTON").1, [TriggerEffect::BellRang(1)]);
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
        let full = level.world.get::<Vitals>(level.members[0]).unwrap().hp;
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
        assert!(level.members.iter().all(|&m| level.world.get::<Vitals>(m).unwrap().hp < full));
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
        let full = level.world.get::<Vitals>(level.members[0]).unwrap().hp;
        level.world.get_mut::<Vitals>(level.members[0]).unwrap().hp = 1;
        assert!(level.run("WELL").0);
        assert!(!level.run("WELL").0, "dry after its one use");
        let memory = level.remember();
        level.visit(Some(memory));
        assert!(!level.run("WELL").0, "still dry after returning");
        level.world.resource_mut::<GameClock>().ms = 2 * 720 * 1000;
        assert!(level.run("WELL").0);
        assert_eq!(level.world.get::<Vitals>(level.members[0]).unwrap().hp, 11.min(full));
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
        assert!(!activate(&mut level.world, door, Some(100.0)));
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
            level.world.query_filtered::<Entity, Changed<Vitals>>().iter(&level.world).count(),
            0
        );
        assert!(activate(&mut level.world, door, Some(1.0)));
        let changed: Vec<_> = level
            .world
            .query_filtered::<Entity, Changed<PropMotion>>()
            .iter(&level.world)
            .collect();
        assert_eq!(changed, [door]);
        assert!(!changed.contains(&other));
        assert_eq!(
            level.world.query_filtered::<Entity, Changed<Vitals>>().iter(&level.world).count(),
            0
        );
        assert_eq!(
            level.world.query_filtered::<Entity, Changed<LevelState>>().iter(&level.world).count(),
            0
        );
        level.settle();
        level.world.clear_trackers();
        assert!(!level.run("DOOR").0, "opening an open door is a no-op");
        assert_eq!(
            level
                .world
                .query_filtered::<Entity, Or<(Changed<DoorState>, Changed<PropMotion>)>>()
                .iter(&level.world)
                .count(),
            0
        );
    }

    #[test]
    fn relay_recipients_complete_in_order_before_the_senders_commit() {
        let mut first = trigger("FIRST", TriggerAction::Nothing);
        first.state_change = Some(("rEaDy".into(), StateChange::Set));
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
                    TriggerEffect::Message { source: None, id: 1 },
                    TriggerEffect::Message { source: None, id: 2 },
                    TriggerEffect::Message { source: None, id: 3 }
                ]
            )
        );
        let memory = level.remember();
        level.visit(Some(memory));
        assert_eq!(level.run("SECOND").1, [TriggerEffect::Message { source: None, id: 2 }]);
    }

    #[test]
    fn damage_uses_roster_order_and_leaves_nonmembers_alone() {
        let dice = Dice { count: 2, sides: 4, base: 0 };
        let mut level = Scenario::new(
            vec![trigger("TRAP", TriggerAction::HurtParty(dice))],
            &[],
            LevelScript::None,
        );
        let profile = Character::new("NPC", Race::Human, Gender::Male, Profession::Fighter);
        let outsider = level.world.spawn((Vitals::new(&profile), profile)).id();
        let outsider_hp = level.world.get::<Vitals>(outsider).unwrap().hp;
        let second = level.add_member("B");
        let first = level.members[0];
        level.world.resource_mut::<PartyRoster>().0 = [second, first].into_iter().collect();
        level.world.clear_trackers();
        let before = [second, first].map(|e| level.world.get::<Vitals>(e).unwrap().hp);
        let effects = level.run("TRAP").1;
        let hits: Vec<_> = effects
            .into_iter()
            .filter_map(|effect| match effect {
                TriggerEffect::Hurt { member, amount } => Some((member, amount)),
                _ => None,
            })
            .collect();
        assert_eq!(hits.iter().map(|&(member, _)| member).collect::<Vec<_>>(), [second, first]);
        for ((member, amount), hp) in hits.into_iter().zip(before) {
            assert!((2..=8).contains(&amount));
            assert_eq!(level.world.get::<Vitals>(member).unwrap().hp, hp - amount);
        }
        assert_eq!(level.world.get::<Vitals>(outsider).unwrap().hp, outsider_hp);
        let changed: Vec<_> =
            level.world.query_filtered::<Entity, Changed<Vitals>>().iter(&level.world).collect();
        assert!(!changed.contains(&outsider));
    }

    #[test]
    fn only_loot_operations_require_catalogs_and_departed_devices_cannot_run() {
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.alternate = Some(act(TriggerAction::ToggleDoor));
        let mut chest = trigger("CHEST", TriggerAction::Animate(Animation::Toggle));
        chest.loot_table = Some("Loot".into());
        let mut level = Scenario::new(
            vec![door, chest],
            &[PropSpec("DOOR", 10.0, false, &[]), PropSpec("CHEST", 5.0, false, &[])],
            LevelScript::None,
        );
        let data = level.world.remove_resource::<GameData>().unwrap();
        assert!(level.run("DOOR").0, "door state does not depend on loot catalogs");
        level.world.clear_trackers();
        assert!(!level.run("CHEST").0);
        assert_eq!(
            level
                .world
                .query_filtered::<Entity, Changed<TriggerState>>()
                .iter(&level.world)
                .count(),
            0
        );
        level.world.insert_resource(data);
        level.settle();
        level.world.entity_mut(level.root).insert(Departing);
        assert!(!level.run("DOOR").0);
        level.world.entity_mut(level.root).remove::<Departing>();
        assert!(level.run("DOOR").0);
        assert_eq!(level.motion("DOOR").target, 0.0);
    }

    #[test]
    fn door_clicks_and_arrival_do_not_require_unrelated_gameplay_resources() {
        use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyBody};
        use crate::game::party::walk_in::move_party;
        use crate::game::world::level::Travel;
        use avian3d::prelude::Position;
        use bevy::ecs::system::RunSystemOnce;
        let mut door = trigger("DOOR", TriggerAction::OpenDoor);
        door.usable = true;
        let mut level =
            Scenario::new(vec![door], &[PropSpec("DOOR", 10.0, false, &[])], LevelScript::None);
        let door = level.device("DOOR");
        level.world.entity_mut(door).insert(Name::new("door"));
        level.world.remove_resource::<GameData>();
        level.world.remove_resource::<Assets<ItemCatalog>>();
        level.world.remove_resource::<Assets<LootTables>>();
        level.world.remove_resource::<GameRng>();
        level.world.remove_resource::<ItemIds>();
        level.world.remove_resource::<PartyInventory>();
        level.world.remove_resource::<PartyRoster>();
        level.world.remove_resource::<GameClock>();
        assert!(activate(&mut level.world, door, Some(1.0)));
        assert_eq!(level.world.get::<PropMotion>(door).unwrap().target, 10.0);
        level.world.init_resource::<Messages<Travel>>();
        let party = level
            .world
            .spawn((PartyBody::default(), Position::default(), Transform::default()))
            .id();
        level.world.resource_mut::<PendingPartyMove>().0 =
            Some((level.root, PartyMove::Teleport(Arrival::Here { eye: Vec3::X, yaw: 0.0 })));
        level.world.run_system_once(move_party).unwrap();
        assert_eq!(
            level.world.get::<Position>(party).unwrap().0,
            Vec3::X - Vec3::Y * EYE_ABOVE_CENTRE
        );
    }

    #[test]
    fn same_named_props_keep_their_own_device_state_and_animation() {
        let mut level = Scenario::new(
            vec![
                trigger("FIRST", TriggerAction::OpenDoor),
                trigger("SECOND", TriggerAction::OpenDoor),
            ],
            &[PropSpec("FIRST", 10.0, false, &[]), PropSpec("SECOND", 20.0, false, &[])],
            LevelScript::None,
        );
        for definition in &mut level.geometry.triggers {
            definition.sound = Some(TriggerSound { path: definition.name.clone(), volume: 1.0 });
            definition.sound_on_commit = true;
            definition.name = "SAME".into();
        }
        level.visit(None);
        let [first, second] = level.devices[..] else { panic!("two devices") };
        assert!(level.world.get::<Prop>(first).is_some());
        level.world.get_mut::<Prop>(second).unwrap().centre = Vec3::X;
        assert!(activate(&mut level.world, first, None));
        assert_eq!(
            level.effects(),
            [TriggerEffect::Sound { path: "FIRST".into(), volume: 1.0, at: Some(Vec3::ZERO) }]
        );
        assert_eq!(level.world.get::<PropMotion>(first).unwrap().target, 10.0);
        assert_eq!(level.world.get::<PropMotion>(second).unwrap().target, 0.0);
        assert!(!level.world.get::<TriggerState>(second).unwrap().fired);
        assert!(activate(&mut level.world, second, None));
        assert_eq!(
            level.effects(),
            [TriggerEffect::Sound { path: "SECOND".into(), volume: 1.0, at: Some(Vec3::X) }]
        );
        assert_eq!(level.world.get::<PropMotion>(second).unwrap().target, 20.0);
        let memory = level.remember();
        level.visit(Some(memory));
        for old in [first, second] {
            assert!(level.world.get_entity(old).is_err());
        }
        for &device in &level.devices {
            assert!(level.world.get::<DoorState>(device).unwrap().open);
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
        assert!(level.world.resource_mut::<PendingPartyMove>().take_for(level.root).is_none());
        assert!(level.run("PORTAL").0);
        assert!(level.world.resource_mut::<PendingPartyMove>().take_for(level.root).is_some());
    }
    #[test]
    fn lethal_trigger_damage_removes_formation_seats_synchronously() {
        use crate::game::party::formation::Formation;
        let mut level = Scenario::new(
            vec![trigger(
                "TRAP",
                TriggerAction::HurtParty(Dice { count: 0, sides: 0, base: 1000 }),
            )],
            &[],
            LevelScript::None,
        );
        let member = level.members[0];
        assert!(level.world.resource::<Formation>().row_of(member).is_some());
        assert!(level.run("TRAP").0);
        assert!(level.world.get::<Vitals>(member).unwrap().is_dead());
        assert!(level.world.resource::<Formation>().row_of(member).is_none());
    }
    #[test]
    fn nonlethal_trigger_damage_does_not_mark_formation_changed() {
        use crate::game::party::formation::Formation;
        let mut level = Scenario::new(
            vec![trigger("TRAP", TriggerAction::HurtParty(Dice { count: 0, sides: 0, base: 1 }))],
            &[],
            LevelScript::None,
        );
        let member = level.members[0];
        let place = level.world.resource::<Formation>().place_of(member);
        level.world.clear_trackers();
        assert!(level.run("TRAP").0);
        assert_eq!(level.world.resource::<Formation>().place_of(member), place);
        assert!(!level.world.get_resource_ref::<Formation>().unwrap().is_changed());
    }
}
