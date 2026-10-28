//! Walking into positioned devices: entering one's place runs it, and an
//! active damage zone hurts whoever stands in it.

use std::time::Duration;

use bevy::prelude::*;

use super::{
    Device, LevelState, PendingPartyMove, PositionedDevices, damage_party, execute, usable,
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

/// One step of the party's eye. Each positioned device, in authored order,
/// advances its damage zone and then runs if the eye has just come into its
/// place. It is one pass so that a device run here can start or reset a
/// later hazard within the same step.
pub(crate) fn walk(In(sample): In<Option<(Entity, Vec3, Duration)>>, world: &mut World) {
    let Some((level, eye, elapsed)) = sample else { return };
    let Some(owner) = world.get::<Level>(level) else { return };
    let handle = owner.definition.clone();
    if world.get::<PositionedDevices>(level).is_none() {
        return;
    }
    world.resource_scope(|world, geometry: Mut<Assets<LevelDefinition>>| {
        let Some(geometry) = geometry.get(&handle) else { return };
        let Some(mut state) = world.get_mut::<LevelState>(level) else { return };
        let previous_eye = state.previous_eye;
        if previous_eye != Some(eye) {
            state.previous_eye = Some(eye);
        }
        let count = world.get::<PositionedDevices>(level).unwrap().0.len();
        for index in 0..count {
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
