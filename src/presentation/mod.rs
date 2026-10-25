//! Rendering, animation and audio presentation of game state.

pub mod automap;
pub mod billboards;
pub mod environment;
pub mod mipmaps;
pub mod monsters;
pub mod sky;
pub mod textures;
pub mod voice;

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<voice::SpeechFinished>()
            .init_resource::<crate::game::automap::MapNotes>()
            .add_input_context::<automap::Screens>()
            .add_systems(Startup, (automap::spawn_screen_controls, automap::load_automap_art))
            .add_systems(
                Update,
                (
                    automap::scale_ui,
                    (
                        automap::toggle_automap,
                        automap::type_note,
                        automap::pan_with_keys,
                        automap::paint_buttons,
                        automap::update_map_view,
                        automap::draw_notes,
                        automap::glow_note_markers,
                    )
                        .chain(),
                ),
            )
            .add_systems(
                Update,
                (
                    textures::animate_level_textures,
                    environment::apply_environment,
                    sky::follow_with_sky_camera.after(crate::game::party::body::aim_camera),
                    sky::shade_clouds,
                    billboards::face_camera.after(crate::game::party::body::aim_camera),
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
