//! Devices as a visit creates them.

use bevy::prelude::*;

use super::{
    Device, DeviceProgram, DialState, DoorState, FountainState, LevelState, LightActivation,
    TriggerState,
};
use crate::content::items::LootTables;
use crate::content::level::{LevelDefinition, TriggerAction};
use crate::game::world::containers::Container;
use crate::game::world::interaction::Guarded;
use crate::game::world::level::LevelOwned;
use crate::game::world::scripts::LevelScript;

/// Allocate identities before linking devices or building prop children.
pub fn spawn_devices(
    commands: &mut Commands,
    level: Entity,
    geometry: &LevelDefinition,
    props: &[Entity],
    script: LevelScript,
    tables: &LootTables,
) -> Vec<Entity> {
    super::super::particles::spawn_emitters(commands, level, geometry);
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
        .map(|prop| prop.unwrap_or_else(|| commands.spawn(LevelOwned(level)).id()))
        .collect();
    let mut state = LevelState { camera_sway: geometry.camera_sway, ..default() };
    for definition in &geometry.triggers {
        if let (Some((name, _)), Some(value)) = (&definition.state_change, definition.initial_state)
        {
            state.variables.entry(name.clone()).or_insert(value);
        }
    }
    for prop in &geometry.props {
        let Some(index) = prop.trigger else { continue };
        let definition = &geometry.triggers[index];
        if let (TriggerAction::Dial, Some((name, _))) =
            (&definition.initial.action, &definition.state_change)
        {
            for position in 0..prop.slots.len() {
                state.variables.entry(name.numbered(position)).or_insert(position == 0);
            }
        }
    }
    for (index, definition) in geometry.triggers.iter().enumerate() {
        let prop_centre = geometry
            .props
            .iter()
            .find(|prop| prop.trigger == Some(index))
            .map(|prop| prop.centre());
        let mut entity = commands.entity(devices[index]);
        entity.insert((
            Device {
                name: definition.name.clone(),
                program: std::sync::Arc::new(DeviceProgram {
                    usable: definition.usable,
                    once: definition.once,
                    fire_linked: definition.fire_linked,
                    initial: definition.initial.clone(),
                    alternate: definition.alternate.clone(),
                    fallback: definition.fallback.clone(),
                    alternate_toggles: definition.alternate_toggles,
                    sound: definition.sound.clone(),
                    sound_on_commit: definition.sound_on_commit,
                    required_states: definition.required_states.clone(),
                    state_change: definition.state_change.clone(),
                    range: definition.range,
                    door: definition.door,
                    lock: definition.lock,
                    at: definition.place.or(prop_centre),
                    reach_origin: prop_centre,
                }),
                recipients: definition.recipients.iter().map(|&i| devices[i]).collect(),
            },
            TriggerState::default(),
        ));
        let actions = std::iter::once(&definition.initial)
            .chain(definition.alternate.iter())
            .chain(definition.fallback.iter());
        if definition.door.is_some()
            || actions
                .clone()
                .any(|b| matches!(b.action, TriggerAction::OpenDoor | TriggerAction::ToggleDoor))
        {
            entity.insert(DoorState {
                open: false,
                locked: definition.door.is_some_and(|d| d.locked),
            });
        }
        if actions.clone().any(|b| matches!(b.action, TriggerAction::Dial)) {
            entity.insert(DialState::default());
        }
        if actions.clone().any(|b| matches!(b.action, TriggerAction::Fountain(_))) {
            entity.insert(FountainState::default());
        }
        if let Some(name) = &definition.loot_table {
            let table = tables.0.iter().position(|t| t.name.eq_ignore_ascii_case(name));
            // Retail itself has two containers naming tables that do not
            // exist (camp's AWAYCAMPCHEST, Rapax's CONSTABLE CHEST); they
            // hold nothing rather than failing their levels.
            if table.is_none() {
                warn!("trigger {}: no loot table {name}; it holds nothing", definition.name);
            }
            entity.insert(Container { table, gold: 0 });
        }
        if let Some(guarded) = definition.lock.map(Guarded::new) {
            entity.insert(guarded);
        }
        if let Some(centre) = definition.place {
            entity.insert(super::walk::WalkVolume {
                centre,
                range: definition.range,
                order: index,
            });
        }
    }
    commands.entity(level).insert((state, LightActivation::default()));
    script.bind(commands, geometry, &devices);
    devices
}
