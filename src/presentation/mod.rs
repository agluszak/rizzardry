//! Rendering, animation and audio presentation of game state.

pub mod ambience;
pub mod automap;
pub mod billboards;
pub mod camp;
pub mod dialog;
pub mod effects;
pub mod environment;
pub mod hover;
pub mod inventory;
pub mod main_screen;
pub mod mipmaps;
pub mod monsters;
pub mod music;
pub mod names;
pub mod notices;
pub mod party_view;
pub mod screen_art;
pub mod selection;
pub mod sky;
pub mod textures;
pub mod voice;
pub mod world_items;

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::game::party::body::Exploration;

pub struct PresentationPlugin;

/// Presentation's own randomness (music, camera shakes, ambience), seeded
/// from the system: never gameplay's [`crate::game::rng::GameRng`], so what
/// is seen and heard cannot change what happens.
#[derive(Resource, Debug)]
pub struct PresentationRng(pub rand::rngs::SmallRng);

impl Default for PresentationRng {
    fn default() -> Self {
        PresentationRng(rand::SeedableRng::from_os_rng())
    }
}

/// Exploring takes input only while gameplay says the party is free
/// (`game::screen`); presentation routes input by that rule, it does not
/// make it.
fn hold_exploration(
    mut commands: Commands,
    freedom: crate::game::screen::Freedom,
    controls: Query<(Entity, Option<&ContextActivity<Exploration>>), With<Exploration>>,
) {
    let active = freedom.free();
    for (entity, activity) in &controls {
        if activity.is_none_or(|a| **a != active) {
            commands.entity(entity).insert(ContextActivity::<Exploration>::new(active));
        }
    }
}

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((main_screen::MainScreenPlugin, inventory::InventoryPlugin))
            .add_message::<voice::SpeechFinished>()
            .add_message::<automap::OpenAutomap>()
            .init_resource::<effects::CameraShake>()
            .init_resource::<notices::TextLog>()
            .init_resource::<hover::Hover>()
            .init_resource::<selection::WorldSelection>()
            .init_resource::<music::Music>()
            .init_resource::<PresentationRng>()
            .add_input_context::<automap::MapKey>()
            .add_input_context::<automap::AutomapScreen>()
            .add_input_context::<automap::NoteEditing>()
            .add_systems(OnEnter(crate::game::screen::Screen::Automap), automap::enter_automap)
            .add_systems(
                PreUpdate,
                (hold_exploration, automap::route_map_keys).before(EnhancedInputSystems::Update),
            )
            .add_input_context::<dialog::QuestionDialog>()
            .add_input_context::<camp::CampScreen>()
            .add_systems(
                PostUpdate,
                (screen_art::render_pictures, screen_art::render_text)
                    .chain()
                    .before(bevy::ui::UiSystems::Content),
            )
            // Pick the same propagated camera/meshes and visibility used for
            // rendering, after fixed physics and Update animation/aiming. UI
            // rasterization observes resulting text on its next presentation pass.
            .add_systems(
                PostUpdate,
                (
                    (selection::select_world_target, selection::activate_selection).chain(),
                    (effects::play_trigger_effects, notices::queue_notices)
                        .after(selection::activate_selection),
                    (hover::hover, hover::show_tooltip)
                        .chain()
                        .after(selection::activate_selection),
                )
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::CameraUpdateSystems)
                    .after(bevy::camera::visibility::VisibilitySystems::MarkNewlyHiddenEntitiesInvisible),
            )
            .add_systems(
                Startup,
                (
                    automap::spawn_map_key,
                    automap::load_automap_art,
                    notices::load_notice_art,
                    dialog::load_dialog_art,
                    camp::load_camp_art,
                ),
            )
            .add_systems(
                Update,
                (
                    automap::scale_ui,
                    screen_art::paint_hover_art,
                    (party_view::attach_camera, party_view::aim_camera)
                        .chain()
                        .after(crate::game::GameplaySet::Combat),
                    camp::show_camp,
                    (
                        automap::close_ended_map,
                        automap::request_automap,
                        automap::build_automap,
                        automap::type_note,
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
            .add_systems(PostUpdate, sky::follow_with_sky_camera.before(bevy::transform::TransformSystems::Propagate))
            .add_systems(
                Update,
                (
                    textures::animate_level_textures.after(environment::request_level_visuals),
                    environment::apply_environment
                        .after(environment::request_level_visuals)
                        .after(party_view::attach_camera),
                    sky::shade_clouds,
                    sky::build_sky.after(environment::request_level_visuals),
                    (environment::request_level_visuals, environment::attach_level_visuals)
                        .chain().after(crate::game::GameplaySet::LevelActivation),
                    billboards::face_camera.after(party_view::aim_camera),
                    music::play_music,
                    (ambience::place_ambient_sounds, ambience::play_ambient_sounds).chain(),
                    dialog::show_travel_question,
                    effects::shake_camera.after(party_view::aim_camera),
                    sky::orbit_sun_and_moon,
                    voice::finish_speech,
                    (
                        monsters::choose_walker_cycles,
                        monsters::show_deaths,
                        monsters::request_monster_visuals,
                        monsters::build_monster_instances,
                        monsters::animate_monsters,
                        monsters::light_monsters,
                    )
                        .chain(),
                    (
                        world_items::show_world_items,
                        world_items::build_world_items,
                        world_items::turn_world_items,
                        world_items::light_world_items,
                    )
                        .chain(),
                ),
            );
    }
}
