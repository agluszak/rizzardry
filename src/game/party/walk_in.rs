//! Walking into positioned triggers, and moving the party where triggers
//! send it: teleports, travel and camera paths.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::body::PartyBody;
use crate::content::level::{Arrival, LevelDefinition};
use crate::game::world::level::{Level, PlayedLevel, Travel, ask_travel};
use crate::game::world::props::pose;
use crate::game::world::triggers::{LevelState, PartyMove, PendingPartyMove};

pub fn run_walk_in_triggers(
    time: Res<Time>,
    party: Query<&Position, With<PartyBody>>,
    levels: Query<Entity, With<Level>>,
) -> Option<(Entity, Vec3, std::time::Duration)> {
    Some((levels.single().ok()?, PartyBody::eye(party.single().ok()?), time.delta()))
}

/// The party's eye flying along a camera path; input and physics wait
/// until it lands (retail `UpdateCameraPathState`).
#[derive(Component, Debug)]
#[require(RigidBodyDisabled)]
pub struct PathFlight {
    path: usize,
    frame: f32,
}

/// Teleport the party within the level, travel to another level's entrance,
/// or start flying it along a camera path.
pub fn move_party(
    mut commands: Commands,
    mut party: Query<(Entity, &mut PartyBody, &mut Position, &mut Transform, Has<PathFlight>)>,
    mut levels: Query<(Entity, &Level, &mut LevelState, &PendingPartyMove)>,
    geometry: Res<Assets<LevelDefinition>>,
    mut travel: MessageWriter<Travel>,
) {
    let Ok((origin, level, mut state, pending)) = levels.single_mut() else { return };
    let party_move = pending.0.clone();
    commands.entity(origin).remove::<PendingPartyMove>();
    let Ok((entity, mut body, mut position, mut transform, flying)) = party.single_mut() else {
        return;
    };
    match party_move {
        PartyMove::Teleport(Arrival::Here { eye, yaw }) => {
            // Same-level destinations place the eye at the trigger, unprobed.
            body.stand(&mut position, &mut transform, eye, yaw);
            info!("teleport to {eye:.2}");
            state.arrive(eye);
        }
        PartyMove::Teleport(Arrival::Entrance { entrance, ask: false }) => {
            travel.write(Travel { entrance });
        }
        // Retail asks first; until answered the party stays where it is.
        PartyMove::Teleport(Arrival::Entrance { entrance, ask: true }) => {
            commands.queue(move |world: &mut World| {
                ask_travel(world, origin, entrance);
            });
        }
        // Retail ignores starting a path that is already playing; the eye
        // flying through its trigger runs it again.
        PartyMove::Fly(_) if flying => {}
        PartyMove::Fly(path) => {
            let camera = &geometry.get(&level.definition).unwrap().camera_paths[path];
            info!("flying camera path {}", camera.name);
            commands.entity(entity).insert(PathFlight { path, frame: 0.0 });
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
    level: PlayedLevel,
) {
    let Ok((entity, mut flight, mut body, mut position, mut transform, mut velocity)) =
        party.single_mut()
    else {
        return;
    };
    let path = &level.definition().unwrap().camera_paths[flight.path];
    let last = (path.keys.len() - 1) as f32;
    flight.frame = (flight.frame + path.keys_per_second * time.delta_secs()).min(last);
    let key = pose(&path.keys, flight.frame);
    (body.yaw, body.pitch, _) = key.rotation.to_euler(EulerRot::YXZ);
    position.0 = PartyBody::centre(key.translation);
    transform.translation = position.0;
    velocity.0 = Vec3::ZERO;
    if flight.frame >= last {
        body.pitch = 0.0;
        commands.entity(entity).remove_with_requires::<PathFlight>();
    }
}
