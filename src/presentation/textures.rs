//! Surface behavior follows the material actually attached to a rendered part.
use crate::content::surface::{Playback, TextureFrames};
use crate::game::time::GameClock;
use bevy::prelude::*;

/// Virtual time pauses ordinary animations; sky gradients use the game clock.
pub fn animate_textures(
    time: Res<Time<Virtual>>,
    clock: Res<GameClock>,
    surfaces: Query<(&TextureFrames, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (animation, material) in &surfaces {
        if animation.frames.is_empty() {
            continue;
        }
        let index = match animation.playback {
            Playback::Manual => clock.phase() as usize % animation.frames.len(),
            _ => animation.frame_at(time.elapsed_secs()),
        };
        let frame = &animation.frames[index];
        let shown = materials.get(&material.0).and_then(|m| m.base_color_texture.as_ref());
        if shown != Some(frame)
            && let Some(mut material) = materials.get_mut(&material.0)
        {
            material.base_color_texture = Some(frame.clone());
        }
    }
}
