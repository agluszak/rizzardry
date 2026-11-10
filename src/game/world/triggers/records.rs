//! Devices as a visit creates them, and the dormant records a level keeps
//! of them between visits.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{
    Device, DialState, DoorState, FountainState, LevelState, LightActivation, PositionedDevices,
    TriggerState,
};
use crate::content::items::{ItemCatalog, LootTables};
use crate::content::level::{LevelDefinition, TriggerAction, VariableName};
use crate::game::items::{ItemStack, Items, Place, Put, unpack};
use crate::game::world::containers::Container;
use crate::game::world::interaction::Guarded;
use crate::game::world::level::LevelOwned;
use crate::game::world::scripts::LevelScript;

/// Dormant records use authored indices, never runtime entity references.
#[derive(Debug, Default)]
pub struct TriggerMemory {
    camera_sway: bool,
    devices: HashMap<usize, KeptDevice>,
    variables: HashMap<VariableName, bool>,
    ambient_sounds: HashMap<String, bool>,
    particles: super::super::particles::ParticleMemory,
    lights: LightActivation,
}

#[derive(Debug)]
struct KeptDevice {
    trigger: TriggerState,
    door: Option<DoorState>,
    dial: Option<DialState>,
    /// The container with what was in it.
    container: Option<(Container, Vec<ItemStack>)>,
    guarded: Option<Guarded>,
    fountain: Option<FountainState>,
}

/// Keep the same device values between visits; proximity and zones are transient.
#[derive(SystemParam)]
pub struct DeviceRecords<'w, 's> {
    particles: Query<'w, 's, &'static super::super::particles::Emitter>,
    items: Items<'w, 's>,
    devices: Query<
        'w,
        's,
        (
            Entity,
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
    pub fn remember(
        &self,
        owned: &[Entity],
        state: &LevelState,
        lights: &LightActivation,
        catalog: &ItemCatalog,
    ) -> TriggerMemory {
        let devices = self
            .devices
            .iter_many(owned)
            .matched()
            .map(|(entity, device, state, door, dial, container, guarded, fountain)| {
                (
                    device.definition,
                    KeptDevice {
                        trigger: state.clone(),
                        door: door.cloned(),
                        dial: dial.cloned(),
                        container: container.map(|container| {
                            let inside = self.items.stowed(entity).iter();
                            let kept = inside
                                .filter_map(|&item| self.items.stack(item, catalog))
                                .collect();
                            (container.clone(), kept)
                        }),
                        guarded: guarded.cloned(),
                        fountain: fountain.cloned(),
                    },
                )
            })
            .collect();
        TriggerMemory {
            camera_sway: state.camera_sway,
            devices,
            variables: state.variables.clone(),
            ambient_sounds: state.ambient_sounds.clone(),
            particles: super::super::particles::remember(self.particles.iter_many(owned).matched()),
            lights: lights.clone(),
        }
    }
}

/// Allocate identities before linking devices or building prop children.
pub fn spawn_devices(
    commands: &mut Commands,
    level: Entity,
    geometry: &LevelDefinition,
    props: &[Entity],
    script: LevelScript,
    (tables, catalog): (&LootTables, &ItemCatalog),
    mut memory: Option<TriggerMemory>,
) -> Vec<Entity> {
    super::super::particles::spawn_emitters(
        commands,
        level,
        geometry,
        memory.as_mut().map(|memory| std::mem::take(&mut memory.particles)).unwrap_or_default(),
    );
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
    let mut state = LevelState {
        camera_sway: memory.as_ref().map_or(geometry.camera_sway, |memory| memory.camera_sway),
        ..default()
    };
    let lights = memory
        .as_mut()
        .map_or_else(LightActivation::default, |memory| std::mem::take(&mut memory.lights));
    if let Some(memory) = &mut memory {
        state.variables = std::mem::take(&mut memory.variables);
        state.ambient_sounds = std::mem::take(&mut memory.ambient_sounds);
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
        let kept = memory.as_mut().and_then(|m| m.devices.remove(&index));
        let mut stowed = Vec::new();
        let mut entity = commands.entity(devices[index]);
        entity.insert(Device {
            definition: index,
            recipients: definition.recipients.iter().map(|&i| devices[i]).collect(),
        });
        if let Some(kept) = kept {
            entity.insert(kept.trigger);
            if let Some(value) = kept.door {
                entity.insert(value);
            }
            if let Some(value) = kept.dial {
                entity.insert(value);
            }
            if let Some((container, kept)) = kept.container {
                entity.insert(container);
                stowed = kept;
            }
            if let Some(value) = kept.guarded {
                entity.insert(value);
            }
            if let Some(value) = kept.fountain {
                entity.insert(value);
            }
        } else {
            entity.insert(TriggerState::default());
            let actions = std::iter::once(&definition.initial)
                .chain(definition.alternate.iter())
                .chain(definition.fallback.iter());
            if definition.door.is_some()
                || actions.clone().any(|b| {
                    matches!(b.action, TriggerAction::OpenDoor | TriggerAction::ToggleDoor)
                })
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
        }
        // What a container held is in it again.
        for stack in &stowed {
            let item = unpack(commands, &catalog[stack.item], stack);
            commands.trigger(Put { item, at: Place::Container(devices[index]) });
        }
        if definition.place.is_some() {
            positioned.0.push(devices[index]);
        }
    }
    commands.entity(level).insert((state, lights, positioned));
    script.bind(commands, geometry, &devices);
    devices
}
