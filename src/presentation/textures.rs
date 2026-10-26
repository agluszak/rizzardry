//! Animated level textures.

use bevy::prelude::*;

use super::environment::LevelAppearance;
use crate::content::level::{LevelGeometry, Playback};
use crate::game::time::GameClock;

/// Show each animated material's current frame. Runs on virtual time, so
/// animations pause with the game as in retail.
pub fn animate_level_textures(
    time: Res<Time<Virtual>>,
    clock: Res<GameClock>,
    levels: Query<(&LevelAppearance, Option<&super::sky::LevelSky>)>,
    geometry: Res<Assets<LevelGeometry>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let seconds = time.elapsed_secs();
    let animations = levels
        .iter()
        .flat_map(|(level, sky)| {
            std::iter::once(&level.0).chain(sky.and_then(|sky| sky.0.as_ref()))
        })
        .filter_map(|handle| geometry.get(handle))
        .flat_map(|geometry| &geometry.animated_textures);
    for animation in animations {
        // Manual animations (the sky gradients) show the time of day.
        let index = match animation.playback {
            Playback::Manual => clock.phase() as usize % animation.frames.len(),
            _ => animation.frame_at(seconds),
        };
        let frame = &animation.frames[index];
        let shown = materials.get(&animation.material).and_then(|m| m.base_color_texture.as_ref());
        if shown != Some(frame)
            && let Some(mut material) = materials.get_mut(&animation.material)
        {
            material.base_color_texture = Some(frame.clone());
        }
    }
}
