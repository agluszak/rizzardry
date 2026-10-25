//! Teleport triggers moving the party within a level.

use std::collections::HashSet;

use avian3d::prelude::*;
use bevy::prelude::*;

use super::body::{EYE_ABOVE_CENTRE, PartyBody};
use crate::content::level::LevelGeometry;
use crate::game::world::level::Level;

/// The teleport triggers the party's eye is inside. Retail runs a positioned
/// trigger when the eye enters its range and not again until it leaves.
#[derive(Resource, Debug, Default)]
pub struct InsideTriggers(HashSet<String>);

pub fn run_teleport_triggers(
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    mut party: Query<(&mut PartyBody, &mut Position, &mut Transform)>,
    mut inside: ResMut<InsideTriggers>,
) {
    let Ok((mut body, mut position, mut transform)) = party.single_mut() else { return };
    let Some(level) = levels.iter().find_map(|l| geometry.get(&l.geometry)) else { return };
    let contains = |eye: Vec3, centre: Vec3, (near, far): (f32, f32)| {
        let distance = eye.distance(centre);
        distance >= near && distance < far
    };
    let eye = position.0 + Vec3::Y * EYE_ABOVE_CENTRE;
    let mut arrival = None;
    for trigger in &level.teleports {
        if !contains(eye, trigger.centre, trigger.range) {
            inside.0.remove(&trigger.name);
        } else if inside.0.insert(trigger.name.clone()) && arrival.is_none() {
            arrival = Some(trigger);
        }
    }
    let Some(trigger) = arrival else { return };
    // Same-level destinations place the eye at the trigger, unprobed.
    position.0 = trigger.destination - Vec3::Y * EYE_ABOVE_CENTRE;
    transform.translation = position.0;
    body.yaw = trigger.yaw;
    info!("teleport {} to {:.2}", trigger.name, trigger.destination);
    // Arriving inside a two-way portal must not send the party back.
    for other in &level.teleports {
        if contains(trigger.destination, other.centre, other.range) {
            inside.0.insert(other.name.clone());
        }
    }
}
