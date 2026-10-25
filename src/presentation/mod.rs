//! Rendering, animation and audio presentation of game state.

pub mod mipmaps;
pub mod textures;

use bevy::prelude::*;

pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, textures::animate_level_textures);
    }
}
