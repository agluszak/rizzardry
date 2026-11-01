//! Walking into positioned devices: entering one's place runs it, and an
//! active damage zone hurts whoever stands in it.

use std::time::Duration;

use bevy::prelude::*;

use super::{Devices, PendingPartyMove, PositionedDevices};
use crate::content::items::Dice;
use crate::content::level::{LevelDefinition, TriggerDefinition};

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

/// One authored-order pass: linked effects are visible to every later device
/// in this same step, including newly started or reset hazards. Walking in
/// runs nothing once a device has asked to move the party.
pub(crate) fn walk(
    In(sample): In<Option<(Entity, Vec3, Duration)>>,
    mut devices: Devices,
    levels: Query<(&PositionedDevices, Has<PendingPartyMove>)>,
    mut zones: Query<&mut ActiveDamageZone>,
    assets: Res<Assets<LevelDefinition>>,
    mut commands: Commands,
) {
    let Some((level, eye, elapsed)) = sample else { return };
    let geometry = devices.geometry(&assets, level);
    let previous_eye = devices.levels.get_mut(level).unwrap().1.previous_eye.replace(eye);
    let (positioned, pending) = levels.get(level).unwrap();
    *devices.moved = pending;
    for &entity in &positioned.0 {
        let definition = devices.definition(geometry, entity);
        let inside = contains(definition, eye);
        if let Ok(mut zone) = zones.get_mut(entity) {
            match zone.step(inside, elapsed) {
                None => {
                    commands.entity(entity).remove::<ActiveDamageZone>();
                }
                Some(true) => devices.damage_party(DAMAGE_ZONE_DICE),
                Some(false) => {}
            }
        }
        if entered(definition, eye, previous_eye)
            && !*devices.moved
            && devices.usable(entity, definition)
        {
            devices.execute(geometry, entity, 0, -1);
        }
    }
}
