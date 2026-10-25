//! The active level and its local objects.

pub mod items;
pub mod level;
pub mod navigation;
pub mod population;
pub mod props;
pub mod scripts;
pub mod search;
pub mod triggers;

use bevy::prelude::*;

/// The highest step the party and walking monsters climb: 300 retail units
/// (0.6 m).
///
/// Retail has no explicit step height: it slides a 500-unit sphere, which
/// rides over edges well above the first 210-unit guess. At 210 units the
/// upper monastery's navmesh split into 22 pieces, the largest 61% of the
/// walkable area; at 300 one piece covers 98%. Raising it to 600 merged
/// nothing further. Characterize against retail stairs.
pub const STEP_HEIGHT: f32 = 300.0 / crate::retail::space::UNITS_PER_METRE;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(navigation::NavigationPlugin)
            .add_message::<level::LevelReady>()
            .add_message::<level::Travel>()
            .add_message::<crate::game::inventory::ItemNews>()
            .add_message::<search::ToggleSearch>()
            .add_message::<search::SearchNews>()
            .init_resource::<search::Search>()
            .init_resource::<level::LevelMemory>()
            .init_resource::<crate::game::rng::GameRng>()
            .init_resource::<crate::game::inventory::ItemIds>()
            .init_resource::<crate::game::inventory::PartyInventory>()
            .add_systems(Startup, crate::game::data::load_game_data)
            .add_message::<triggers::TriggerEffect>()
            // The level is swapped before anything builds on it this frame.
            .add_systems(PreUpdate, level::travel)
            .add_systems(
                Update,
                (
                    level::build_level_geometry,
                    level::build_sky,
                    population::populate_level,
                    population::measure_monsters,
                    (search::search_key, search::toggle_search, search::search_pulse).chain(),
                ),
            )
            .add_systems(FixedUpdate, props::animate_props);
    }
}
