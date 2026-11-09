//! Gameplay state and rules.

pub mod automap;
pub mod camp;
pub mod characters;
pub mod combat;
pub mod data;
pub mod difficulty;
pub mod factions;
pub mod freedom;
pub mod hits;
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
    /// Every body's derived facts follow their sources: where it stands,
    /// whose side it is on, what it is to what lands on it.
    Bodies,
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
                (
                    GameplaySet::LevelActivation,
                    GameplaySet::Bodies,
                    GameplaySet::Burden,
                    GameplaySet::Combat,
                )
                    .chain(),
            )
            // Outside a fight deeds land as the frame's gameplay ends; a fight
            // settles its own on the fixed step.
            .add_systems(
                Update,
                (hits::fly, hits::resolve, characters::settle_deaths)
                    .chain()
                    .after(GameplaySet::Combat)
                    .run_if(world::level::level_active)
                    .run_if(not(any_with_component::<combat::Battle>)),
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
                hits::HitsPlugin,
            ))
            // Navmeshes: retail serves a level's collision, navigation bakes
            // it on request and caches the result.
            .register_asset_loader(world::navigation::asset::NavmeshLoader {
                cache: bevy::asset::io::file::FileAssetReader::get_base_path()
                    .join(world::navigation::asset::CACHE),
            });
    }
}
