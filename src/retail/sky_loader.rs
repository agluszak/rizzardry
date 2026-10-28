//! Loads an uncompiled `.lvl` world, which retail uses for skies, as
//! [`LevelGeometry`]: one mesh record (the dome), then the level sections
//! without sentinels (the clouds, Sun and Moon as props).

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::LoadError;
use super::level_loader::{LoadedTextures, build_props, load_material};
use super::mesh_record::read_mesh_record;
use super::prop_loader::{PropLighting, record_meshes};
use super::sections::read_uncompiled_sections;
use crate::bytes::Bytes;
use crate::content::level::{LevelGeometry, LevelPart};
use crate::content::surface::Playback;

#[derive(Default, TypePath)]
pub struct LvlLoader;

impl AssetLoader for LvlLoader {
    type Asset = LevelGeometry;
    type Settings = ();
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<LevelGeometry, LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let mut bytes = Bytes::new(&data);
        let record = read_mesh_record(&mut bytes)?;
        let sections = read_uncompiled_sections(&mut bytes)?;
        let directory = "bitmaps";
        let mut textures = LoadedTextures::default();
        // The sky is drawn as authored, unlit by the level.
        let unlit = default();
        let lighting = PropLighting { ambient: Vec3::ONE, level: &unlit };

        let mut materials = Vec::with_capacity(record.materials.len());
        for (index, material) in record.materials.iter().enumerate() {
            let label = format!("material/{index}");
            materials.push(
                load_material(context, label, directory, &mut textures, index, material).await,
            );
        }
        // Only the dome gradients are driven by the game clock.
        for surface in &mut materials {
            surface.animation.playback = Playback::Manual;
        }
        // The dome's vertices are already in sky space around the origin,
        // where the sky camera sits; its record placement is the dome's
        // pivot vertex, not a transform to apply.
        let transform = Transform::default();
        let mut parts = Vec::new();
        for (material, mesh, vertex_lighting) in
            record_meshes(&record, &transform, &lighting, |m| materials[m].emission)
        {
            let mesh = context.add_labeled_asset(format!("mesh/{material}"), mesh);
            parts.push(LevelPart {
                mesh,
                surface: materials[material].clone(),
                lighting: vertex_lighting,
            });
        }
        let props = sections.props.iter().chain(&sections.bitmaps);
        let props = build_props(
            context,
            // The sky world's props run no triggers.
            props,
            directory,
            &mut textures,
            &lighting,
        )
        .await;
        Ok(LevelGeometry { parts, props, ..default() })
    }

    fn extensions(&self) -> &[&str] {
        &["lvl"]
    }
}
