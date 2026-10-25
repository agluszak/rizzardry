//! Loads an item's world model (`data/items3d/<name>.itm`) as an
//! [`ItemModel`]: a version byte (2), then one mesh record (retail
//! `LoadItemFromFile`), textured from `data/items3d/bitmaps`.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::bytes::Bytes;
use super::level_loader::{LoadedTextures, PvlLoadError, load_material};
use super::mesh_record::read_mesh_record;
use super::prop_loader::{PropLighting, record_meshes};
use super::pvl::invalid;
use crate::content::items::{ItemModel, ItemModelPart};

#[derive(Default, TypePath)]
pub struct ItmLoader;

impl AssetLoader for ItmLoader {
    type Asset = ItemModel;
    type Settings = ();
    type Error = PvlLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<ItemModel, PvlLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let mut bytes = Bytes::new(data.get(1..).unwrap_or_default());
        if data.first() != Some(&2) {
            return Err(invalid("item model is not version 2", &bytes).into());
        }
        let record = read_mesh_record(&mut bytes)?;
        let mut textures = LoadedTextures::default();
        let mut materials = Vec::with_capacity(record.materials.len());
        for (index, material) in record.materials.iter().enumerate() {
            let label = format!("material/{index}");
            materials.push(
                load_material(context, label, "bitmaps", &mut textures, index, material).await,
            );
        }
        // The vertex colours are the instance's to set; bake nothing.
        let unlit = default();
        let lighting = PropLighting { ambient: Vec3::ONE, level: &unlit };
        let mut parts = Vec::new();
        for (material, mesh, _) in
            record_meshes(&record, &Transform::default(), &lighting, |m| materials[m].emission)
        {
            parts.push(ItemModelPart {
                mesh: context.add_labeled_asset(format!("mesh/{material}"), mesh),
                material: materials[material].handle.clone(),
                ambient: materials[material].ambient,
                emission: materials[material].emission,
            });
        }
        Ok(ItemModel { parts })
    }

    fn extensions(&self) -> &[&str] {
        &["itm"]
    }
}
