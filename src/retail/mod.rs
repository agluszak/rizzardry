//! Decoding of the user's retail installation.
//!
//! Gameplay never sees archive offsets or packed records: this module turns
//! them into typed records and native Bevy assets.

pub mod automap;
pub mod credits;
pub mod files;
pub mod item_model_loader;
pub mod items;
pub mod keyframes;
pub mod level_loader;
pub mod links;
pub mod mesh_record;
pub mod monster;
pub mod monster_loader;
pub mod navmesh;
pub mod oct;
pub mod path;
pub mod pcx_loader;
pub mod playlist;
pub mod population;
pub mod prop;
pub mod prop_loader;
pub mod pvl;
pub mod sections;
pub mod sky_loader;
pub mod slf;
pub mod source;
pub mod space;
pub mod species;
pub mod stci;
pub mod sti_loader;
pub mod strings;
pub mod text_loader;
pub mod texture_loader;
pub mod trigger;

use std::sync::Arc;

use bevy::asset::AssetApp;
use bevy::prelude::*;

use crate::content::level::{LevelDefinition, LevelGeometry};

/// Registers the `retail://` and `navmesh://` asset sources and retail
/// format loaders.
///
/// Must be added before `DefaultPlugins`: Bevy requires asset sources to be
/// registered before `AssetPlugin` builds the server.
pub struct RetailPlugin {
    pub files: Arc<files::RetailFiles>,
    /// Where processed navmeshes are cached.
    pub navmesh_cache: std::path::PathBuf,
    /// How many walker size classes get a navmesh.
    pub navmesh_classes: usize,
}

impl Plugin for RetailPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_source(source::RETAIL_SOURCE, source::retail_source(self.files.clone()))
            .register_asset_source(
                crate::content::navmesh::NAVMESH_SOURCE,
                source::navmesh_source(
                    self.files.clone(),
                    self.navmesh_classes,
                    &self.navmesh_cache,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        app.init_asset::<crate::content::navmesh::LevelCollision>()
            .init_asset_loader::<navmesh::LevelCollisionLoader>()
            .init_asset::<LevelDefinition>()
            .init_asset_loader::<level_loader::PvlDefinitionLoader>()
            .init_asset::<crate::content::monster::MonsterBody>()
            .init_asset_loader::<monster_loader::MlsBodyLoader>()
            .init_asset::<crate::content::items::ItemCatalog>()
            .init_asset_loader::<items::ItemCatalogLoader>()
            .init_asset::<crate::content::items::LootTables>()
            .init_asset_loader::<items::LootTablesLoader>()
            .init_asset::<crate::content::species::MonsterCatalog>()
            .init_asset_loader::<species::MonsterCatalogLoader>()
            .init_asset::<crate::content::ui::TextTable>()
            .init_asset_loader::<text_loader::TextLoader>();
    }
}

/// Render and audio content loaders, installed only by rendered app composition.
pub struct RetailPresentationPlugin;

impl Plugin for RetailPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<LevelGeometry>()
            .init_asset_loader::<level_loader::PvlLoader>()
            .init_asset_loader::<sky_loader::LvlLoader>()
            .init_asset::<crate::content::monster::MonsterModel>()
            .init_asset_loader::<monster_loader::MlsLoader>()
            .init_asset::<crate::content::ui::Credits>()
            .init_asset_loader::<credits::CreditsLoader>()
            .init_asset::<crate::content::ui::SpriteSheet>()
            .init_asset_loader::<sti_loader::StiLoader>()
            .init_asset_loader::<pcx_loader::PcxLoader>()
            .init_asset::<crate::content::items::ItemModel>()
            .init_asset_loader::<item_model_loader::ItmLoader>()
            .init_asset::<crate::content::music::Playlist>()
            .init_asset_loader::<playlist::PlaylistLoader>()
            .init_asset::<texture_loader::RetailTexture>()
            .init_asset_loader::<texture_loader::RetailTextureLoader>();
    }
}
