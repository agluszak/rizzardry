//! The rendering camera follows the authoritative party body.

use bevy::prelude::*;

use crate::content::UNITS_PER_METRE;
use crate::content::level::DEFAULT_VIEW_DISTANCE;
use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyBody};

/// The first-person camera on the party's body.
#[derive(Component, Debug)]
pub struct PartyCamera;

/// Put the camera on the party's body when it appears.
pub fn attach_camera(add: On<Add<PartyBody>>, mut commands: Commands) {
    let party = add.entity;
    commands.entity(party).insert(Visibility::default());
    commands.spawn((
        Name::new("Party camera"),
        PartyCamera,
        SpatialListener::new(0.2),
        Camera3d::default(),
        // The sky camera clears the frame and draws first.
        Camera { clear_color: ClearColorConfig::None, ..default() },
        Projection::Perspective(PerspectiveProjection {
            fov: 70f32.to_radians(),
            // Retail's near clip of 250 units; the level sets the far one.
            near: 250.0 / UNITS_PER_METRE,
            far: DEFAULT_VIEW_DISTANCE,
            ..default()
        }),
        Transform::from_xyz(0.0, EYE_ABOVE_CENTRE, 0.0),
        ChildOf(party),
    ));
}

/// Point the camera along the party's facing.
pub fn aim_camera(
    party: Query<(&PartyBody, crate::game::world::ActorPose), Without<PartyCamera>>,
    conversations: Query<&crate::game::npcs::conversation::Conversation>,
    npc_bodies: Query<&crate::game::npcs::NpcBodies>,
    actors: Query<
        (crate::game::world::ActorPose, &crate::game::world::population::Monster),
        Without<PartyCamera>,
    >,
    mut camera: Query<&mut Transform, With<PartyCamera>>,
) {
    if let (Ok((body, pose)), Ok(mut camera)) = (party.single(), camera.single_mut()) {
        let ordinary = Quat::from_rotation_y(body.yaw) * Quat::from_rotation_x(body.pitch);
        let rotation = conversations
            .single()
            .ok()
            .and_then(|c| npc_bodies.get(c.actor).ok())
            .and_then(|bodies| bodies.iter().find_map(|body| actors.get(body).ok()))
            .map_or(ordinary, |(target, monster)| {
                let direction = target.translation() + Vec3::Y * monster.body.eye_height
                    - (pose.translation() + camera.translation);
                if direction.length_squared() < 0.0001 {
                    ordinary
                } else {
                    Transform::IDENTITY.looking_to(direction, Vec3::Y).rotation
                }
            });
        if camera.rotation != rotation {
            camera.rotation = rotation;
        }
    }
}
