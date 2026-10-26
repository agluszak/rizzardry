//! Walking into positioned triggers, and moving the party where triggers
//! send it: teleports, travel and camera paths.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::body::{EYE_ABOVE_CENTRE, PartyBody};
use crate::content::level::{Arrival, LevelGeometry};
use crate::game::world::level::{Departing, Level, Travel, TravelQuestion};
use crate::game::world::props::pose;
use crate::game::world::triggers::{
    self, Device, DeviceProximity, PartyMove, PendingPartyMove, PositionedDevices,
};

pub fn run_walk_in_triggers(
    time: Res<Time>,
    party: Query<&Position, With<PartyBody>>,
    mut commands: Commands,
) {
    let Ok(position) = party.single() else { return };
    let eye = PartyBody::eye(position);
    let elapsed = time.delta();
    commands.queue(move |world: &mut World| triggers::walk(world, eye, elapsed));
}

/// The party's eye flying along a camera path; input and physics wait
/// until it lands (retail `UpdateCameraPathState`).
#[derive(Component, Debug)]
pub struct PathFlight {
    path: usize,
    frame: f32,
}

/// Teleport the party within the level, travel to another level's entrance,
/// or start flying it along a camera path.
pub fn move_party(
    mut commands: Commands,
    mut party: Query<(Entity, &mut PartyBody, &mut Position, &mut Transform, Has<PathFlight>)>,
    levels: Query<(Entity, &Level, &PositionedDevices), Without<Departing>>,
    geometry: Res<Assets<LevelGeometry>>,
    mut travel: MessageWriter<Travel>,
    mut pending: ResMut<PendingPartyMove>,
    devices: Query<&Device>,
    mut proximity: Query<&mut DeviceProximity>,
) {
    let Ok((origin, level, positioned)) = levels.single() else { return };
    let Some(party_move) = pending.take_for(origin) else { return };
    let Ok((entity, mut body, mut position, mut transform, flying)) = party.single_mut() else {
        return;
    };
    match party_move {
        PartyMove::Teleport(Arrival::Here { eye, yaw }) => {
            // Same-level destinations place the eye at the trigger, unprobed.
            position.0 = eye - Vec3::Y * EYE_ABOVE_CENTRE;
            transform.translation = position.0;
            body.yaw = yaw;
            info!("teleport to {eye:.2}");
            if let Some(geometry) = geometry.get(&level.geometry) {
                triggers::arrive(eye, positioned, geometry, &devices, &mut proximity);
            }
        }
        PartyMove::Teleport(Arrival::Entrance { entrance, ask: false }) => {
            travel.write(Travel { entrance });
        }
        // Retail asks first; until answered the party stays where it is.
        PartyMove::Teleport(Arrival::Entrance { entrance, ask: true }) => {
            commands.spawn((TravelQuestion { entrance }, ChildOf(origin)));
        }
        // Retail ignores starting a path that is already playing; the eye
        // flying through its trigger runs it again.
        PartyMove::Fly(_) if flying => {}
        PartyMove::Fly(name) => {
            let Some(path) = geometry.get(&level.geometry).and_then(|level| {
                level.camera_paths.iter().position(|p| p.name.eq_ignore_ascii_case(&name))
            }) else {
                warn!("no camera path {name}");
                return;
            };
            info!("flying camera path {name}");
            commands.entity(entity).insert((PathFlight { path, frame: 0.0 }, RigidBodyDisabled));
        }
    }
}

/// Carry the flying party along its path at the path's keys per second,
/// looking where the path looks; land it at the end.
pub fn fly_party(
    mut commands: Commands,
    time: Res<Time>,
    mut party: Query<(
        Entity,
        &mut PathFlight,
        &mut PartyBody,
        &mut Position,
        &mut Transform,
        &mut LinearVelocity,
    )>,
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
) {
    let Ok((entity, mut flight, mut body, mut position, mut transform, mut velocity)) =
        party.single_mut()
    else {
        return;
    };
    let level = levels.iter().find_map(|level| geometry.get(&level.geometry));
    let Some(path) = level.and_then(|level| level.camera_paths.get(flight.path)) else {
        commands.entity(entity).remove::<(PathFlight, RigidBodyDisabled)>();
        return;
    };
    let last = path.keys.len().saturating_sub(1) as f32;
    flight.frame = (flight.frame + path.keys_per_second * time.delta_secs()).min(last);
    let key = pose(&path.keys, flight.frame);
    (body.yaw, body.pitch, _) = key.rotation.to_euler(EulerRot::YXZ);
    position.0 = key.translation - Vec3::Y * EYE_ABOVE_CENTRE;
    transform.translation = position.0;
    velocity.0 = Vec3::ZERO;
    if flight.frame >= last {
        body.pitch = 0.0;
        commands.entity(entity).remove::<(PathFlight, RigidBodyDisabled)>();
    }
}
