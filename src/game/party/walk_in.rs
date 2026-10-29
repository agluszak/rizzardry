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
            let Some(camera) =
                geometry.get(&level.definition).and_then(|level| level.camera_paths.get(path))
            else {
                warn!("invalid camera path {path}");
                return;
            };
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
    let Some(path) = level.definition().and_then(|level| level.camera_paths.get(flight.path))
    else {
        commands.entity(entity).remove_with_requires::<PathFlight>();
        return;
    };
    let last = path.keys.len().saturating_sub(1) as f32;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::level::CameraPathModel;
    use crate::game::camp::{Camp, CampNews, CampRequest};
    use crate::game::party::body::{PartyLook, look_around};

    #[test]
    fn path_flight_blocks_manual_actions_and_landing_releases_the_party() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<LevelDefinition>>()
            .init_resource::<crate::game::time::GameClock>()
            .add_message::<PartyLook>()
            .add_message::<CampRequest>()
            .add_message::<CampNews>()
            .add_systems(Update, (look_around, crate::game::camp::request_camp));
        let geometry =
            app.world_mut().resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition {
                camera_paths: vec![CameraPathModel {
                    name: "test".into(),
                    keys: vec![Transform::from_xyz(3.0, 4.0, 5.0)],
                    keys_per_second: 1.0,
                }],
                ..default()
            });
        app.world_mut().spawn(Level::test(geometry));
        let body = app
            .world_mut()
            .spawn((
                PartyBody::default(),
                Position::default(),
                Transform::default(),
                LinearVelocity::ZERO,
                PathFlight { path: 0, frame: 0.0 },
                RigidBodyDisabled,
            ))
            .id();
        app.world_mut().write_message(PartyLook(Vec2::ONE));
        app.world_mut().write_message(CampRequest);
        app.update();
        assert!(!app.world().contains_resource::<Camp>());
        assert_eq!(app.world().get::<PartyBody>(body).unwrap().yaw, 0.0);
        // Finish the actual path operation, then submit a fresh request.
        use bevy::ecs::system::RunSystemOnce;
        app.world_mut().run_system_once(fly_party).unwrap();
        assert!(app.world().get::<PathFlight>(body).is_none());
        assert!(app.world().get::<RigidBodyDisabled>(body).is_none());
        assert_eq!(
            app.world().get::<Position>(body).unwrap().0,
            PartyBody::centre(Vec3::new(3.0, 4.0, 5.0))
        );
        app.world_mut().write_message(CampRequest);
        app.update();
        assert!(app.world().contains_resource::<Camp>());
    }
}
