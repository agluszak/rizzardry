//! Rendering, animation and audio presentation of game state.

pub mod environment;
pub mod mipmaps;
pub mod monsters;
pub mod sky;
pub mod textures;
pub mod voice;

use bevy::prelude::*;

pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<voice::SpeechFinished>().add_systems(
            Update,
            (
                textures::animate_level_textures,
                environment::apply_environment,
                sky::follow_with_sky_camera.after(crate::game::party::body::aim_camera),
                sky::shade_clouds,
                sky::orbit_sun_and_moon,
                voice::finish_speech,
                (
                    monsters::choose_walker_cycles,
                    monsters::build_monster_instances,
                    monsters::animate_monsters,
                )
                    .chain(),
            ),
        );
    }
}
