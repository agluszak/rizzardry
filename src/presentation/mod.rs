//! Rendering, animation and audio presentation of game state.

pub mod environment;
pub mod mipmaps;
pub mod sky;
pub mod textures;

use bevy::prelude::*;

pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                textures::animate_level_textures,
                environment::apply_environment,
                sky::follow_with_sky_camera.after(crate::game::party::body::aim_camera),
                sky::shade_clouds,
            ),
        );
    }
}
