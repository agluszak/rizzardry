//! Animated level textures.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::world::level::ActiveLevel;

/// Show each animated material's current frame. Runs on virtual time, so
/// animations pause with the game as in retail.
pub fn animate_level_textures(
    time: Res<Time<Virtual>>,
    active: Option<Res<ActiveLevel>>,
    levels: Res<Assets<LevelGeometry>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(level) = active.and_then(|active| levels.get(&active.geometry)) else { return };
    let seconds = time.elapsed_secs();
    for animation in &level.animated_textures {
        let frame = &animation.frames[animation.frame_at(seconds)];
        let shown = materials.get(&animation.material).and_then(|m| m.base_color_texture.as_ref());
        if shown != Some(frame)
            && let Some(mut material) = materials.get_mut(&animation.material) {
                material.base_color_texture = Some(frame.clone());
            }
    }
}
