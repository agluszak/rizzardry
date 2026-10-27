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
        items: assets.load(crate::content::paths::ITEMS),
        loot: assets.load(crate::content::paths::LOOT_TABLES),
        species: assets.load(crate::content::paths::SPECIES),
        spells: assets.load(crate::content::paths::SPELLS),
    });
}
