//! Props drawn turned toward the viewer.
//!
//! Retail keeps only the location of a facing prop and lays its mesh out in
//! the camera's axes (Whizzardry `MeshInstance::renderTransform`): a full
//! billboard, not one turning about the vertical only.

use bevy::prelude::*;

use crate::game::party::body::PartyCamera;

/// A prop part that faces the viewer. Its parents must not rotate: level
/// and sky roots and prop roots do not.
#[derive(Component, Debug, Default)]
pub struct FaceCamera;

/// The party body's rotation is locked, so the camera's own rotation is its
/// rotation in the world. The sky camera shares it.
pub fn face_camera(
    camera: Query<&Transform, With<PartyCamera>>,
    mut sprites: Query<&mut Transform, (With<FaceCamera>, Without<PartyCamera>)>,
) {
    let Ok(camera) = camera.single() else { return };
    for mut sprite in &mut sprites {
        sprite.rotation = camera.rotation;
    }
}
