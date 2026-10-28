//! Surface behavior follows the material actually attached to a rendered part.
use crate::content::surface::{Playback, TextureFrames};
use crate::game::time::GameClock;
use bevy::prelude::*;

/// Virtual time pauses ordinary animations; sky gradients use the game clock.
pub fn animate_textures(
    time: Res<Time<Virtual>>,
    clock: Res<GameClock>,
    surfaces: Query<(
        &TextureFrames,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<super::lighting::SceneMaterial>>,
    )>,
    mut dynamic: Option<ResMut<Assets<super::lighting::SceneMaterial>>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (animation, material, lit) in &surfaces {
        if animation.frames.is_empty() {
            continue;
        }
        let index = match animation.playback {
            Playback::Manual => clock.phase() as usize % animation.frames.len(),
            _ => animation.frame_at(time.elapsed_secs()),
        };
        let frame = &animation.frames[index];
        if let (Some(lit), Some(dynamic)) = (lit, dynamic.as_deref_mut()) {
            if dynamic.get(&lit.0).and_then(|material| material.base.base_color_texture.as_ref())
                != Some(frame)
                && let Some(mut material) = dynamic.get_mut(&lit.0)
            {
                material.base.base_color_texture = Some(frame.clone());
            }
        }
        let Some(material) = material else { continue };
        let shown = materials.get(&material.0).and_then(|m| m.base_color_texture.as_ref());
        if shown != Some(frame)
            && let Some(mut material) = materials.get_mut(&material.0)
        {
            material.base_color_texture = Some(frame.clone());
        }
    }
}
