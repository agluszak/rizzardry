//! Rendering, animation and audio presentation of game state.

pub mod ambience;
pub mod automap;
pub mod billboards;
pub mod dialog;
pub mod effects;
pub mod environment;
pub mod hud;
pub mod mipmaps;
pub mod monsters;
pub mod music;
pub mod notices;
pub mod sky;
pub mod textures;
pub mod voice;

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<voice::SpeechFinished>()
            .add_message::<dialog::TravelAnswer>()
            .init_resource::<crate::game::automap::MapNotes>()
            .init_resource::<effects::CameraShake>()
            .init_resource::<notices::NoticeQueue>()
            .init_resource::<music::Music>()
            .init_resource::<ambience::AmbienceNoise>()
            .add_input_context::<automap::Screens>()
            .add_systems(
                Startup,
                (
                    automap::spawn_screen_controls,
                    automap::load_automap_art,
                    notices::load_notice_art,
                    dialog::load_dialog_art,
                ),
            )
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
                        automap::draw_layer_label,
                        automap::update_map_cursor,
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
                    effects::play_trigger_effects,
                    music::play_music,
                    (hud::spawn_party_bar, hud::show_portraits, hud::update_health_bars).chain(),
                    (ambience::place_ambient_sounds, ambience::play_ambient_sounds).chain(),
                    (notices::queue_notices, notices::show_notices, notices::fade_notices).chain(),
                    (
                        dialog::show_travel_question,
                        dialog::answer_with_keys,
                        dialog::settle_travel_question,
                        dialog::paint_dialog_buttons,
                    )
                        .chain(),
                    effects::shake_camera.after(crate::game::party::body::aim_camera),
                    sky::orbit_sun_and_moon,
                    voice::finish_speech,
                    (
                        monsters::choose_walker_cycles,
                        monsters::build_monster_instances,
                        monsters::animate_monsters,
                        monsters::light_monsters,
                    )
                        .chain(),
                ),
            );
    }
}
