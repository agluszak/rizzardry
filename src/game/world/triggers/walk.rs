//! Walking into positioned devices: entering one's place runs it, and an
//! active damage zone hurts whoever stands in it.

use std::time::Duration;

use bevy::prelude::*;

use super::{
    Device, LevelState, PendingPartyMove, PositionedDevices, TriggerState, damage_party, execute,
    usable,
};
use crate::content::items::Dice;
use crate::content::level::{LevelDefinition, TriggerDefinition};
use crate::game::world::level::Level;

fn contains(definition: &TriggerDefinition, eye: Vec3) -> bool {
    (definition.range.0..definition.range.1).contains(&eye.distance(definition.place.unwrap()))
}

/// Whether the party's eye has just come into a device's place.
fn entered(definition: &TriggerDefinition, eye: Vec3, previous: Option<Vec3>) -> bool {
    contains(definition, eye) && previous.is_some_and(|previous| !contains(definition, previous))
}

/// How often a damage zone hurts whoever stands in it.
const DAMAGE_ZONE_PERIOD: f32 = 0.5;
/// What a damage zone hurts each member by.
const DAMAGE_ZONE_DICE: Dice = Dice { count: 1, sides: 6, base: 2 };

/// An activated hazard's remaining timing, independent of party occupancy.
#[derive(Component, Debug)]
pub(crate) struct ActiveDamageZone(Timer);

impl ActiveDamageZone {
    pub(super) fn new() -> Self {
        ActiveDamageZone(Timer::from_seconds(DAMAGE_ZONE_PERIOD, TimerMode::Repeating))
    }

    /// One step with the party inside or not: whether it hurts now. A zone
    /// the party has left ends (`None`).
    fn step(&mut self, inside: bool, elapsed: Duration) -> Option<bool> {
        inside.then(|| self.0.tick(elapsed).just_finished())
    }
}

/// Quiet fixed ticks are ordinary ECS iteration. Only a real entry candidate
/// starts an exclusive operation; the remaining authored suffix stays inside
/// that operation because links can start/reset a later hazard in this step.
pub(crate) fn walk(
    In(sample): In<Option<(Entity, Vec3, Duration)>>,
    mut commands: Commands,
    mut levels: Query<(&Level, &PositionedDevices, &mut LevelState, Has<PendingPartyMove>)>,
    geometry: Res<Assets<LevelDefinition>>,
    mut devices: Query<(&Device, &TriggerState, Option<&mut ActiveDamageZone>)>,
) {
    let Some((level, eye, elapsed)) = sample else { return };
    let Ok((owner, positioned, mut state, pending)) = levels.get_mut(level) else { return };
    let Some(geometry) = geometry.get(&owner.definition) else { return };
    let previous_eye = state.previous_eye;
    if previous_eye != Some(eye) {
        state.previous_eye = Some(eye);
    }
    for (index, &entity) in positioned.0.iter().enumerate() {
        let (device, state, zone) = devices.get_mut(entity).unwrap();
        let definition = &geometry.triggers[device.definition];
        let inside = contains(definition, eye);
        if entered(definition, eye, previous_eye)
            && !pending
            && definition.usable
            && !(definition.once && state.fired)
        {
            commands.queue(move |world: &mut World| {
                walk_activation(world, level, eye, previous_eye, elapsed, index);
            });
            return;
        }
        if let Some(mut zone) = zone {
            match zone.step(inside, elapsed) {
                None => {
                    commands.entity(entity).remove::<ActiveDamageZone>();
                }
                Some(true) => commands.run_system_cached_with(damage_party, DAMAGE_ZONE_DICE),
                Some(false) => {}
            }
        }
    }
}

fn walk_activation(
    world: &mut World,
    level: Entity,
    eye: Vec3,
    previous_eye: Option<Vec3>,
    elapsed: Duration,
    start: usize,
) {
    let Some(owner) = world.get::<Level>(level) else { return };
    let handle = owner.definition.clone();
    world.resource_scope(|world, geometry: Mut<Assets<LevelDefinition>>| {
        let Some(geometry) = geometry.get(&handle) else { return };
        let count = world.get::<PositionedDevices>(level).unwrap().0.len();
        for index in start..count {
            let entity = world.get::<PositionedDevices>(level).unwrap().0[index];
            let definition = &geometry.triggers[world.get::<Device>(entity).unwrap().definition];
            let inside = contains(definition, eye);
            match world.get_mut::<ActiveDamageZone>(entity).map(|mut z| z.step(inside, elapsed)) {
                Some(None) => {
                    world.entity_mut(entity).remove::<ActiveDamageZone>();
                }
                Some(Some(true)) => {
                    // A party-less world has nobody to hurt.
                    let _ = world.run_system_cached_with(damage_party, DAMAGE_ZONE_DICE);
                }
                _ => {}
            }
            if entered(definition, eye, previous_eye)
                && world.get::<PendingPartyMove>(level).is_none()
                && usable(world, entity, definition)
            {
                execute(world, geometry, entity, 0);
            }
        }
    });
}
