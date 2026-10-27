//! The rendering camera follows the authoritative party body.

use bevy::prelude::*;

use crate::content::UNITS_PER_METRE;
use crate::content::level::DEFAULT_VIEW_DISTANCE;
use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyBody};

/// The first-person camera on the party's body.
#[derive(Component, Debug)]
pub struct PartyCamera;

/// Put the camera on the party's body when it appears.
pub fn attach_camera(mut commands: Commands, party: Query<Entity, Added<PartyBody>>) {
    for party in &party {
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
}

/// Point the camera along the party's facing.
pub fn aim_camera(party: Query<&PartyBody>, mut camera: Query<&mut Transform, With<PartyCamera>>) {
    if let (Ok(body), Ok(mut camera)) = (party.single(), camera.single_mut()) {
        let rotation = Quat::from_rotation_y(body.yaw) * Quat::from_rotation_x(body.pitch);
        if camera.rotation != rotation {
            camera.rotation = rotation;
        }
    }
}
