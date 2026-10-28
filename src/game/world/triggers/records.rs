//! Devices as a visit creates them, and the dormant records a level keeps
//! of them between visits.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{
    Device, DialState, DoorState, FountainState, LevelState, PositionedDevices, TriggerState,
};
use crate::content::items::LootTables;
use crate::content::level::{LevelDefinition, TriggerAction, VariableName};
use crate::game::world::containers::{Container, Contents};
use crate::game::world::interaction::Guarded;
use crate::game::world::level::LevelOwned;
use crate::game::world::scripts::LevelScript;

/// Dormant records use authored indices, never runtime entity references.
#[derive(Debug, Default)]
pub struct TriggerMemory {
    devices: HashMap<usize, KeptDevice>,
    variables: HashMap<VariableName, bool>,
}

#[derive(Debug)]
struct KeptDevice {
    on: bool,
    fired: bool,
    use_alternate: bool,
    door: Option<DoorState>,
    dial: Option<DialState>,
    container: Option<Container>,
    guarded: Option<Guarded>,
    fountain: Option<FountainState>,
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
            Option<&'static Guarded>,
            Option<&'static FountainState>,
        ),
    >,
}

impl DeviceRecords<'_, '_> {
    /// Remember the devices among what a level owns.
    pub fn remember(&self, owned: &[Entity], state: &LevelState) -> TriggerMemory {
        let devices = self
            .devices
            .iter_many(owned)
            .matched()
            .map(|(device, state, door, dial, container, guarded, fountain)| {
                (
                    device.definition,
                    KeptDevice {
                        on: state.on,
                        fired: state.fired,
                        use_alternate: state.use_alternate,
                        door: door.cloned(),
                        dial: dial.cloned(),
                        container: container.cloned(),
                        guarded: guarded.cloned(),
                        fountain: fountain.cloned(),
                    },
                )
            })
            .collect();
        TriggerMemory { devices, variables: state.variables.clone() }
    }
}

/// Allocate identities before linking devices or building prop children.
pub fn spawn_devices(
    commands: &mut Commands,
    level: Entity,
    geometry: &LevelDefinition,
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
        .map(|prop| prop.unwrap_or_else(|| commands.spawn(LevelOwned(level)).id()))
        .collect();
    let mut state = LevelState::default();
    if let Some(memory) = &mut memory {
        state.variables = std::mem::take(&mut memory.variables);
    } else {
        for definition in &geometry.triggers {
            if let (Some((name, _)), Some(value)) =
                (&definition.state_change, definition.initial_state)
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
    }
    let mut positioned = PositionedDevices::default();
    for (index, definition) in geometry.triggers.iter().enumerate() {
        let mut kept = memory.as_mut().and_then(|m| m.devices.remove(&index));
        let mut entity = commands.entity(devices[index]);
        entity.insert((
            Device {
                definition: index,
                recipients: definition.recipients.iter().map(|&i| devices[i]).collect(),
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
            // Retail itself has two containers naming tables that do not
            // exist (camp's AWAYCAMPCHEST, Rapax's CONSTABLE CHEST); they
            // hold nothing rather than failing their levels.
            if table.is_none() {
                warn!("trigger {}: no loot table {name}; it holds nothing", definition.name);
            }
            entity.insert(kept.as_mut().and_then(|k| k.container.take()).unwrap_or(Container {
                table,
                generated: false,
                contents: Contents::default(),
            }));
        }
        // A lock overcome stays overcome.
        let guarded = match &kept {
            Some(kept) => kept.guarded.clone(),
            None => definition.lock.map(Guarded::new),
        };
        if let Some(guarded) = guarded {
            entity.insert(guarded);
        }
        if definition.place.is_some() {
            positioned.0.push(devices[index]);
        }
    }
    commands.entity(level).insert((state, positioned));
    script.bind(commands, geometry, &devices);
    devices
}
