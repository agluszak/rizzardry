//! Animated level textures.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::world::level::Level;

/// Show each animated material's current frame. Runs on virtual time, so
/// animations pause with the game as in retail.
pub fn animate_level_textures(
    time: Res<Time<Virtual>>,
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let seconds = time.elapsed_secs();
    let animations = levels
        .iter()
        .filter_map(|level| geometry.get(&level.geometry))
        .flat_map(|geometry| &geometry.animated_textures);
    for animation in animations {
        let frame = &animation.frames[animation.frame_at(seconds)];
        let shown = materials.get(&animation.material).and_then(|m| m.base_color_texture.as_ref());
        if shown != Some(frame)
            && let Some(mut material) = materials.get_mut(&animation.material)
        {
            material.base_color_texture = Some(frame.clone());
        }
    }
}
