//! The retail databases the rules read, loaded once.

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
        items: assets.load("retail://data/databases/items.dbs"),
        loot: assets.load("retail://data/databases/itemtables.dbs"),
        species: assets.load("retail://data/databases/monsters.dbs"),
        spells: assets.load("retail://data/databases/spelltables.dbs"),
    });
}
