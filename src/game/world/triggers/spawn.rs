//! A level's devices, made as it is: what each is and does, read from the
//! level's definition once.

use bevy::prelude::*;

use super::walk::WalkVolume;
use super::{
    ChangesState, Device, DeviceNames, DeviceSound, DialState, DoorState, FiresLinked, Key,
    LevelState, LightActivation, Once, Reach, Recipients, Requires, Usable, Uses,
};
use crate::content::items::LootTables;
use crate::content::level::{LevelDefinition, TriggerAction};
use crate::game::world::containers::Container;
use crate::game::world::fountains::Supply;
use crate::game::world::interaction::Guarded;
use crate::game::world::level::LevelOwned;

/// Make the level's devices: on their props, or standing alone at their
/// places. Every device is given an entity before any is linked.
pub fn spawn_devices(
    commands: &mut Commands,
    level: Entity,
    geometry: &LevelDefinition,
    props: &[Entity],
    tables: &LootTables,
) -> Vec<Entity> {
    super::super::particles::spawn_emitters(commands, level, geometry);
    let mut attached = vec![None; geometry.triggers.len()];
    for (index, prop) in geometry.props.iter().enumerate() {
        if let Some(trigger) = prop.trigger {
            assert!(
                attached[trigger].replace((props[index], prop)).is_none(),
                "a trigger belongs to one prop"
            );
        }
    }
    let devices: Vec<Entity> = attached
        .iter()
        .map(|prop| prop.map_or_else(|| commands.spawn(LevelOwned(level)).id(), |(prop, _)| prop))
        .collect();
    let mut state = LevelState { camera_sway: geometry.camera_sway, ..default() };
    for (index, definition) in geometry.triggers.iter().enumerate() {
        let mut device = commands.entity(devices[index]);
        device.insert((
            Device { name: definition.name.clone() },
            Uses {
                initial: definition.initial.clone(),
                alternate: definition.alternate.clone(),
                fallback: definition.fallback.clone(),
                alternate_toggles: definition.alternate_toggles,
            },
        ));
        if definition.usable {
            device.insert(Usable);
        }
        if definition.once {
            device.insert(Once);
        }
        if definition.fire_linked {
            device.insert(FiresLinked);
        }
        if !definition.recipients.is_empty() {
            device.insert(Recipients(definition.recipients.iter().map(|&i| devices[i]).collect()));
        }
        if !definition.required_states.is_empty() {
            device.insert(Requires(definition.required_states.clone()));
        }
        if let Some((name, change)) = &definition.state_change {
            device.insert(ChangesState(name.clone(), *change));
            if let Some(value) = definition.initial_state {
                state.variables.entry(name.clone()).or_insert(i32::from(value));
            }
        }
        let prop = attached[index].map(|(_, prop)| prop);
        if let Some(prop) = prop {
            device.insert(Reach { range: definition.range, centre: prop.centre() });
        }
        if let Some(sound) = &definition.sound {
            device.insert(DeviceSound {
                sound: sound.clone(),
                at: definition.place.or(prop.map(|prop| prop.centre())),
                on_commit: definition.sound_on_commit,
            });
        }
        let actions = std::iter::once(&definition.initial)
            .chain(definition.alternate.iter())
            .chain(definition.fallback.iter())
            .map(|chosen| &chosen.action);
        let has = |wanted: fn(&TriggerAction) -> bool| actions.clone().any(wanted);
        if definition.door.is_some()
            || has(|a| matches!(a, TriggerAction::OpenDoor | TriggerAction::ToggleDoor))
        {
            let locked = definition.door.is_some_and(|d| d.locked);
            device.insert(DoorState { open: false, locked });
        }
        if let Some(key) = definition.door.and_then(|door| door.key) {
            device.insert(Key(key));
        }
        if has(|a| matches!(a, TriggerAction::Dial)) {
            let variable = definition.state_change.as_ref().map(|(name, _)| name.clone());
            // A dial's slots each have a variable of their own, the first
            // set.
            if let (Some(name), Some(prop)) = (&variable, prop) {
                for position in 0..prop.slots.len() {
                    let made = state.variables.entry(name.numbered(position));
                    made.or_insert(i32::from(position == 0));
                }
            }
            device.insert(DialState { position: 0, variable });
        }
        if has(|a| matches!(a, TriggerAction::Fountain(_))) {
            device.insert(Supply::at(definition.place.or(prop.map(|prop| prop.centre()))));
        }
        if let Some(name) = &definition.loot_table {
            let table = tables.0.iter().position(|t| t.name.eq_ignore_ascii_case(name));
            // Retail itself has two containers naming tables that do not
            // exist (camp's AWAYCAMPCHEST, Rapax's CONSTABLE CHEST); they
            // hold nothing rather than failing their levels.
            if table.is_none() {
                warn!("trigger {}: no loot table {name}; it holds nothing", definition.name);
            }
            device.insert(Container { table, gold: 0 });
        }
        if let Some(lock) = definition.lock {
            device.insert(Guarded::new(lock));
        }
        if let Some(centre) = definition.place {
            device.insert(WalkVolume { centre, range: definition.range, order: index });
        }
    }
    let names = geometry.triggers.iter().zip(&devices);
    let names = names.map(|(definition, &device)| (definition.name.to_ascii_lowercase(), device));
    commands.entity(level).insert((
        state,
        LightActivation::default(),
        DeviceNames(names.collect()),
    ));
    devices
}
