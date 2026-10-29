//! The retail databases the rules read, loaded once.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::content::items::{ItemCatalog, LootTables};
use crate::content::species::MonsterCatalog;
use crate::content::spells::SpellCatalog;

#[derive(Resource, Debug)]
pub struct GameData {
    pub items: Handle<ItemCatalog>,
    pub loot: Handle<LootTables>,
    pub species: Handle<MonsterCatalog>,
    pub spells: Handle<SpellCatalog>,
}

pub fn load_game_data(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(GameData {
        items: assets.load(crate::content::paths::ITEMS),
        loot: assets.load(crate::content::paths::LOOT_TABLES),
        species: assets.load(crate::content::paths::SPECIES),
        spells: assets.load(crate::content::paths::SPELLS),
    });
}

/// The loaded databases, for systems that read them. Each is `None` until
/// it has loaded.
#[derive(SystemParam)]
pub struct Content<'w> {
    data: Option<Res<'w, GameData>>,
    items: Option<Res<'w, Assets<ItemCatalog>>>,
    loot: Option<Res<'w, Assets<LootTables>>>,
    species: Option<Res<'w, Assets<MonsterCatalog>>>,
    spells: Option<Res<'w, Assets<SpellCatalog>>>,
}

impl Content<'_> {
    pub fn items(&self) -> Option<&ItemCatalog> {
        self.items.as_ref()?.get(&self.data.as_ref()?.items)
    }

    pub fn loot(&self) -> Option<&LootTables> {
        self.loot.as_ref()?.get(&self.data.as_ref()?.loot)
    }

    pub fn species(&self) -> Option<&MonsterCatalog> {
        self.species.as_ref()?.get(&self.data.as_ref()?.species)
    }

    pub fn spells(&self) -> Option<&SpellCatalog> {
        self.spells.as_ref()?.get(&self.data.as_ref()?.spells)
    }
}

impl GameData {
    /// The item catalog, for operations that hold the whole world.
    pub fn items(world: &World) -> Option<&ItemCatalog> {
        world.get_resource::<Assets<ItemCatalog>>()?.get(&world.get_resource::<GameData>()?.items)
    }

    /// The species catalog, for operations that hold the whole world.
    pub fn species(world: &World) -> Option<&MonsterCatalog> {
        world
            .get_resource::<Assets<MonsterCatalog>>()?
            .get(&world.get_resource::<GameData>()?.species)
    }

    /// The spell catalog, for operations that hold the whole world.
    pub fn spells(world: &World) -> Option<&SpellCatalog> {
        world.get_resource::<Assets<SpellCatalog>>()?.get(&world.get_resource::<GameData>()?.spells)
    }

    /// The loot tables, for operations that hold the whole world.
    pub fn loot(world: &World) -> Option<&LootTables> {
        world.get_resource::<Assets<LootTables>>()?.get(&world.get_resource::<GameData>()?.loot)
    }
}
