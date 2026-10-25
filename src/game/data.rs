//! The retail databases the rules read, loaded once.

use bevy::prelude::*;

use crate::content::items::{ItemCatalog, LootTables};

#[derive(Resource, Debug)]
pub struct GameData {
    pub items: Handle<ItemCatalog>,
    pub loot: Handle<LootTables>,
}

pub fn load_game_data(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(GameData {
        items: assets.load("retail://data/databases/items.dbs"),
        loot: assets.load("retail://data/databases/itemtables.dbs"),
    });
}
