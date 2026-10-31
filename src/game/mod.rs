//! Gameplay state and rules.

pub mod automap;
pub mod camp;
pub mod characters;
pub mod combat;
pub mod data;
pub mod difficulty;
pub mod factions;
pub mod freedom;
pub mod inventory;
pub mod loot;
pub mod magic;
pub mod npcs;
pub mod party;
pub mod rng;
pub mod time;
pub mod world;

/// Only the boundaries that share gameplay randomness or require accepted
/// state from an earlier operation. Presentation remains independently scheduled.
#[derive(bevy::prelude::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameplaySet {
    LevelActivation,
    /// Inventory has redistributed burden outside combat. Other stats are live rules.
    Burden,
    Combat,
}

pub mod session;

use bevy::prelude::*;

/// The game's rules and state: shared campaign services, the world, the
/// party, time, combat and magic.
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<rng::GameRng>()
            .init_resource::<factions::Factions>()
            .init_resource::<difficulty::Difficulty>()
            .add_message::<factions::FactionNews>()
            .add_systems(Startup, data::load_catalogs)
            .add_systems(Update, data::publish_catalogs.run_if(resource_exists::<data::Loading>))
            .configure_sets(
                Update,
                (GameplaySet::LevelActivation, GameplaySet::Burden, GameplaySet::Combat).chain(),
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
                inventory::plugin,
            ))
            // Navmeshes: retail serves a level's collision, navigation bakes
            // it on request and caches the result.
            .register_asset_loader(world::navigation::asset::NavmeshLoader {
                cache: bevy::asset::io::file::FileAssetReader::get_base_path()
                    .join(world::navigation::asset::CACHE),
            });
    }
}

#[cfg(test)]
mod tests {
    /// Every gameplay system's parameters can run together: a conflicting
    /// access only shows when its schedule first runs.
    #[test]
    fn every_system_initializes() {
        use bevy::prelude::*;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::state::app::StatesPlugin))
            .add_plugins(super::GamePlugin);
        app.finish();
        app.cleanup();
        let world = app.world_mut();
        let mut schedules = world.remove_resource::<Schedules>().unwrap();
        for (_, schedule) in schedules.iter_mut() {
            schedule.initialize(world).unwrap();
        }
    }
}
