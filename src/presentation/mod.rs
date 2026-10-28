//! Rendering, animation and audio presentation of game state.

pub mod ambience;
pub mod automap;
pub mod billboards;
pub mod camp;
pub mod character_view;
pub mod controls;
pub mod conversation;
pub mod device_panel;
pub mod dialog;
pub mod effects;
pub mod environment;
pub mod frontend;
pub mod hover;
pub mod inventory;
pub mod journal;
pub mod main_screen;
pub mod monsters;
mod morph_picking;
pub mod music;
pub mod names;
pub mod notices;
pub mod options;
pub mod party_view;
mod party_voice;
pub mod retail_controls;
pub mod screen_art;
pub mod selection;
pub mod settings;
pub mod sky;
mod surfaces;
#[cfg(test)]
mod test_input;
pub mod textures;
pub mod voice;
pub mod world_items;

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use controls::Exploration;

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
    focus: Res<bevy::input_focus::InputFocus>,
    widgets: Query<(), With<bevy::input_focus::tab_navigation::TabIndex>>,
    controls: Query<(Entity, Option<&ContextActivity<Exploration>>), With<Exploration>>,
) {
    let active = freedom.free() && !focus.get().is_some_and(|entity| widgets.contains(entity));
    for (entity, activity) in &controls {
        if activity.is_none_or(|a| **a != active) {
            commands.entity(entity).insert(ContextActivity::<Exploration>::new(active));
        }
    }
}

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            main_screen::MainScreenPlugin,
            conversation::ConversationViewPlugin,
            inventory::InventoryPlugin,
            frontend::FrontendPlugin,
            device_panel::plugin,
            journal::plugin,
            options::plugin,
        ))
        // Retail's double click.
        .insert_resource(bevy::picking::PickingSettings {
            multi_click_interval: std::time::Duration::from_millis(400),
            ..default()
        })
        .add_input_context::<Exploration>()
        .add_observer(retail_controls::primary_buttons_only)
        .add_observer(party_view::attach_camera)
        .add_observer(environment::request_level_visuals)
        .add_observer(sky::request_skies)
        .add_observer(dialog::open_travel_question)
        .add_observer(automap::close_ended_map)
        .add_systems(Startup, controls::spawn_exploration_controls)
        .add_systems(PreUpdate, controls::read_controls.after(EnhancedInputSystems::Apply))
        .add_message::<voice::SpeechFinished>()
        .add_message::<party_voice::SpokenRemark>()
        .add_message::<automap::OpenAutomap>()
        .init_resource::<effects::CameraShake>()
        .init_resource::<notices::TextLog>()
        .init_resource::<hover::Hover>()
        .init_resource::<selection::WorldSelection>()
        .init_resource::<music::Music>()
        .init_resource::<PresentationRng>()
        .add_input_context::<automap::MapKey>()
        .add_input_context::<automap::AutomapScreen>()
        .add_systems(OnEnter(crate::game::screen::Screen::Automap), automap::enter_automap)
        .add_systems(
            PreUpdate,
            (hold_exploration, automap::route_map_keys)
                .after(bevy::input_focus::InputFocusSystems::Dispatch)
                .before(EnhancedInputSystems::Update),
        )
        .add_systems(PostUpdate, automap::finish_note_input.after(bevy::text::EditableTextSystems))
        .add_input_context::<dialog::QuestionDialog>()
        .add_input_context::<camp::CampScreen>()
        .add_systems(
            PostUpdate,
            (
                retail_controls::sync_scrollbars,
                retail_controls::sync_button_navigation,
                retail_controls::paint_controls,
                retail_controls::paint_focus,
                screen_art::render_pictures,
                screen_art::render_tiled_pictures,
                screen_art::render_text,
            )
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
                (effects::play_trigger_effects, effects::play_lock_sounds, notices::queue_notices)
                    .after(selection::activate_selection),
                (hover::hover, hover::show_tooltip).chain().after(selection::activate_selection),
            )
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::camera::CameraUpdateSystems)
                .after(
                    bevy::camera::visibility::VisibilitySystems::MarkNewlyHiddenEntitiesInvisible,
                ),
        )
        .add_systems(
            PostUpdate,
            (
                party_voice::queue_device_remarks.run_if(crate::game::world::level::level_active),
                party_voice::play_device_remarks,
            )
                .chain()
                .after(selection::activate_selection)
                .before(notices::queue_notices),
        )
        .add_systems(
            Startup,
            (
                automap::spawn_map_key,
                automap::load_automap_art,
                notices::load_notice_art,
                screen_art::load_string_table,
                dialog::load_dialog_art,
                camp::load_camp_art,
            ),
        )
        .add_systems(
            Update,
            (
                screen_art::scale_ui,
                party_view::aim_camera.after(crate::game::GameplaySet::Combat),
                camp::show_camp,
                (
                    automap::request_automap,
                    automap::build_automap.run_if(any_with_component::<automap::Automap>),
                    automap::sync_note_input,
                    automap::draw_note_input,
                    (
                        automap::paint_buttons,
                        automap::update_map_view,
                        automap::draw_notes,
                        automap::draw_layer_label,
                    )
                        .chain()
                        .run_if(any_with_component::<automap::Automap>),
                    automap::update_map_cursor,
                    automap::glow_note_markers.run_if(any_with_component::<automap::NoteMarker>),
                )
                    .chain(),
            ),
        )
        .add_systems(
            PostUpdate,
            sky::follow_with_sky_camera.before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            Update,
            (
                textures::animate_textures
                    .after(sky::build_sky)
                    .after(environment::attach_level_visuals)
                    .after(monsters::build_monster_instances)
                    .after(world_items::build_world_items),
                surfaces::tint_surfaces
                    .after(monsters::light_monsters)
                    .after(world_items::light_world_items),
                environment::apply_environment,
                sky::shade_clouds,
                sky::build_sky,
                environment::prepare_appearance.before(crate::game::GameplaySet::LevelActivation),
                environment::attach_level_visuals,
                billboards::face_camera.after(party_view::aim_camera),
                music::play_music,
                (ambience::place_ambient_sounds, ambience::play_ambient_sounds).chain(),
                (dialog::show_travel_question, dialog::focus_ready_question)
                    .chain()
                    .after(crate::game::GameplaySet::LevelActivation),
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
                    .chain()
                    // A monster killed this frame starts dying this frame.
                    .after(crate::game::GameplaySet::Combat),
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

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::game::{screen::Screen, world::level::Level};
    use bevy::input_focus::{InputFocus, tab_navigation::TabIndex};

    #[test]
    fn focused_ui_prevents_the_same_arrows_from_driving_exploration() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<Screen>()
            .init_resource::<InputFocus>()
            .add_systems(Update, hold_exploration);
        app.world_mut().spawn(Level::test(default()));
        let controls =
            app.world_mut().spawn((Exploration, ContextActivity::<Exploration>::new(true))).id();
        let widget = app.world_mut().spawn(TabIndex(0)).id();
        app.insert_resource(InputFocus::from_entity(widget));
        app.update();
        assert!(!**app.world().get::<ContextActivity<Exploration>>(controls).unwrap());
        app.world_mut().resource_mut::<InputFocus>().clear();
        app.update();
        assert!(**app.world().get::<ContextActivity<Exploration>>(controls).unwrap());
    }
}
