//! Teleport triggers moving the party within a level or to another.

use std::collections::HashSet;

use avian3d::prelude::*;
use bevy::prelude::*;

use super::body::{EYE_ABOVE_CENTRE, PartyBody};
use crate::content::level::{Arrival, LevelGeometry, TeleportTrigger};
use crate::game::world::level::{Level, Travel};

/// The teleport triggers the party's eye is inside. Retail runs a positioned
/// trigger when the eye enters its range and not again until it leaves.
#[derive(Resource, Debug, Default)]
pub struct InsideTriggers(HashSet<String>);

fn contains(trigger: &TeleportTrigger, eye: Vec3) -> bool {
    let distance = eye.distance(trigger.centre);
    distance >= trigger.range.0 && distance < trigger.range.1
}

impl InsideTriggers {
    /// The party arrives at `eye`: arriving inside a trigger (a two-way
    /// portal, the exit beside an entrance) must not send it straight back.
    pub fn enter_at(&mut self, level: &LevelGeometry, eye: Vec3) {
        self.0 =
            level.teleports.iter().filter(|t| contains(t, eye)).map(|t| t.name.clone()).collect();
    }
}

pub fn run_teleport_triggers(
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    mut party: Query<(&mut PartyBody, &mut Position, &mut Transform)>,
    mut inside: ResMut<InsideTriggers>,
    mut travel: MessageWriter<Travel>,
) {
    let Ok((mut body, mut position, mut transform)) = party.single_mut() else { return };
    let Some(level) = levels.iter().find_map(|l| geometry.get(&l.geometry)) else { return };
    let eye = position.0 + Vec3::Y * EYE_ABOVE_CENTRE;
    let mut fired = None;
    for trigger in &level.teleports {
        if !contains(trigger, eye) {
            inside.0.remove(&trigger.name);
        } else if inside.0.insert(trigger.name.clone()) && fired.is_none() {
            fired = Some(trigger);
        }
    }
    let Some(trigger) = fired else { return };
    match &trigger.arrival {
        Arrival::Here { eye, yaw } => {
            // Same-level destinations place the eye at the trigger, unprobed.
            position.0 = *eye - Vec3::Y * EYE_ABOVE_CENTRE;
            transform.translation = position.0;
            body.yaw = *yaw;
            info!("teleport {} to {eye:.2}", trigger.name);
            inside.enter_at(level, *eye);
        }
        Arrival::Entrance(entrance) => {
            travel.write(Travel { entrance: entrance.clone() });
        }
    }
}
