//! Decoding of the user's retail installation.
//!
//! Gameplay never sees archive offsets or packed records: this module turns
//! them into typed records and native Bevy assets.

pub mod bytes;
pub mod files;
pub mod keyframes;
pub mod level_loader;
pub mod links;
pub mod mesh_record;
pub mod oct;
pub mod path;
pub mod prop;
pub mod prop_loader;
pub mod pvl;
pub mod sections;
pub mod sky_loader;
pub mod slf;
pub mod source;
pub mod space;
pub mod trigger;

use std::sync::Arc;

use bevy::asset::AssetApp;
use bevy::prelude::*;

use crate::content::level::LevelGeometry;

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
                source::NAVMESH_SOURCE,
                source::navmesh_source(
                    self.files.clone(),
                    self.navmesh_classes,
                    &self.navmesh_cache,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        app.init_asset::<LevelGeometry>()
            .init_asset_loader::<level_loader::PvlLoader>()
            .init_asset_loader::<sky_loader::LvlLoader>();
    }
}
