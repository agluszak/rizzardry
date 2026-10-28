//! Gameplay state and rules.

pub mod automap;
pub mod camp;
pub mod characters;
pub mod combat;
pub mod data;
pub mod difficulty;
pub mod factions;
pub mod inventory;
pub mod loot;
pub mod magic;
pub mod npcs;
pub mod party;
pub mod rng;
pub mod screen;
pub mod time;
pub mod world;

/// Only the boundaries that share gameplay randomness or require accepted
/// state from an earlier operation. Presentation remains independently scheduled.
#[derive(bevy::prelude::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameplaySet {
    LevelActivation,
    /// Members' derived stats are refreshed ([`stats`]); what reads them
    /// runs after.
    Stats,
    Combat,
}

pub mod session;
pub mod stats;

use bevy::prelude::*;

/// The game's rules and state: shared campaign services, the world, the
/// party, time, combat and magic.
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<rng::GameRng>()
            .init_resource::<inventory::ItemIds>()
            .init_resource::<factions::Factions>()
            .init_resource::<difficulty::Difficulty>()
            .init_state::<screen::Screen>()
            .add_message::<inventory::ItemNews>()
            .add_systems(Startup, data::load_game_data)
            .configure_sets(
                Update,
                (GameplaySet::LevelActivation, GameplaySet::Stats, GameplaySet::Combat).chain(),
            )
            .add_plugins((
                avian3d::prelude::PhysicsPlugins::default(),
                world::WorldPlugin,
                npcs::NpcPlugin,
                party::PartyPlugin,
                time::TimePlugin,
                camp::CampPlugin,
                combat::CombatPlugin,
                magic::MagicPlugin,
                stats::plugin,
            ))
            // Navmeshes: retail serves a level's collision, navigation bakes
            // it on request and caches the result.
            .register_asset_loader(world::navigation::asset::NavmeshLoader {
                cache: bevy::asset::io::file::FileAssetReader::get_base_path()
                    .join(world::navigation::asset::CACHE),
            });
    }
}
