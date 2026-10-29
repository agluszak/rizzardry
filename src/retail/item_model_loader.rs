//! Loads an item's world model (`data/items3d/<name>.itm`) as an
//! [`ItemModel`]: a version byte (2), then one mesh record (retail
//! `LoadItemFromFile`), textured from `data/items3d/bitmaps`.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::LoadError;
use super::level_loader::{LoadedTextures, load_material};
use super::mesh_record::read_mesh_record;
use super::prop_loader::{PropLighting, record_meshes};
use crate::bytes::Bytes;
use crate::content::items::{ItemModel, ItemModelPart};

#[derive(Default, TypePath)]
pub struct ItmLoader;

impl AssetLoader for ItmLoader {
    type Asset = ItemModel;
    type Settings = ();
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<ItemModel, LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let mut bytes = Bytes::new(data.get(1..).unwrap_or_default());
        if data.first() != Some(&2) {
            return Err(bytes.invalid("item model is not version 2").into());
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
        let lighting = PropLighting { ambient: Vec3::ONE, offset: Vec3::ZERO };
        let mut parts = Vec::new();
        for (material, mut mesh, _) in record_meshes(&record, &lighting, |m| materials[m].emission)
        {
            mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR);
            parts.push(ItemModelPart {
                mesh: context.add_labeled_asset(format!("mesh/{material}"), mesh),
                surface: materials[material].clone(),
            });
        }
        Ok(ItemModel { parts })
    }

    fn extensions(&self) -> &[&str] {
        &["itm"]
    }
}
