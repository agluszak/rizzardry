//! Props drawn turned toward the viewer.
//!
//! Retail keeps only the location of a facing prop and lays its mesh out in
//! the camera's axes (Whizzardry `MeshInstance::renderTransform`): a full
//! billboard, not one turning about the vertical only.

use bevy::prelude::*;

use crate::presentation::party_view::PartyCamera;

/// A visual that faces the viewer. Its immediate parent may be an animated
/// gameplay part; ancestors above that part have identity rotations.
#[derive(Component, Debug, Default)]
pub struct FaceCamera;

/// The party body's rotation is locked, so the camera's own rotation is its
/// rotation in the world. The sky camera shares it.
pub fn face_camera(
    camera: Query<&Transform, With<PartyCamera>>,
    parents: Query<&Transform, (Without<FaceCamera>, Without<PartyCamera>)>,
    mut sprites: Query<(&ChildOf, &mut Transform), (With<FaceCamera>, Without<PartyCamera>)>,
) {
    let Ok(camera) = camera.single() else { return };
    for (parent, mut sprite) in &mut sprites {
        let Ok(parent) = parents.get(parent.parent()) else { continue };
        sprite.rotation = parent.rotation.inverse() * camera.rotation;
    }
}
