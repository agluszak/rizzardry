//! Loads a retail monster species (`data/monsters/<name>.mls`) and its
//! cycles as a [`MonsterModel`].

use std::collections::HashMap;

use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages, io::Reader};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::level_loader::{LoadedTextures, PvlLoadError, load_material, sibling};
use super::mesh_record::FACE_TWO_SIDED;
use super::monster::{MonsterCycle, read_cycle, read_species};
use super::space::retail_position;
use crate::content::monster::{MonsterCycleModel, MonsterModel, MonsterPart};

#[derive(Default, TypePath)]
pub struct MlsLoader;

/// A cycle's render meshes: one per material, unwelded by (vertex, UV),
/// with retail winding reversed for the Z mirror. Faces textured `blank*`
/// are hidden, as retail does.
fn cycle_meshes(cycle: &MonsterCycle, frame0: &[Vec3]) -> Vec<(usize, Mesh, Vec<u32>)> {
    let mut by_material: Vec<usize> =
        cycle.mesh.faces.iter().map(|f| f.material as usize).collect();
    by_material.sort_unstable();
    by_material.dedup();
    let mut meshes = Vec::new();
    for material in by_material {
        let texture = cycle.mesh.materials[material].texture().map(|(_, n)| n.to_lowercase());
        if texture.as_deref().is_some_and(|t| t.starts_with("blank")) {
            continue;
        }
        let mut corners: HashMap<(u32, [u32; 2]), u32> = HashMap::new();
        let (mut positions, mut uvs, mut source, mut indices) = (vec![], vec![], vec![], vec![]);
        let mut emit = |vertices: [u32; 3], face_uvs: [[f32; 2]; 3], order: [usize; 3]| {
            for corner in order {
                let (vertex, uv) = (vertices[corner], face_uvs[corner]);
                let key = (vertex, uv.map(f32::to_bits));
                let index = *corners.entry(key).or_insert_with(|| {
                    positions.push(frame0[vertex as usize].to_array());
                    uvs.push(uv);
                    source.push(vertex);
                    positions.len() as u32 - 1
                });
                indices.push(index);
            }
        };
        for face in cycle.mesh.faces.iter().filter(|f| f.material as usize == material) {
            emit(face.vertices, face.uvs, [0, 2, 1]);
            if face.flags & FACE_TWO_SIDED != 0 {
                emit(face.vertices, face.uvs, [0, 1, 2]);
            }
        }
        let count = positions.len();
        // Kept in the main world: instances animate copies of it.
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_indices(Indices::U32(indices));
        mesh.compute_smooth_normals();
        meshes.push((material, mesh, source));
    }
    meshes
}

impl AssetLoader for MlsLoader {
    type Asset = MonsterModel;
    type Settings = ();
    type Error = PvlLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<MonsterModel, PvlLoadError> {
        let mut text = Vec::new();
        reader.read_to_end(&mut text).await?;
        let species = read_species(&String::from_utf8_lossy(&text));
        let mut textures = LoadedTextures::default();
        let mut cycles = bevy::platform::collections::HashMap::new();
        for (name, file, fps) in &species.cycles {
            let path = sibling(context, ".", &format!("{file}.mon")).ok_or(PvlLoadError::Path)?;
            let cycle = match context.read_asset_bytes(path).await {
                Ok(bytes) => read_cycle(&bytes)?,
                Err(error) => {
                    warn!("{}: cycle {name} missing: {error}", context.path());
                    continue;
                }
            };
            let frames: Vec<Vec<Vec3>> = cycle
                .frames
                .iter()
                .map(|frame| frame.iter().map(|&v| retail_position(v)).collect())
                .collect();
            let mut materials = Vec::new();
            for (index, record) in cycle.mesh.materials.iter().enumerate() {
                let label = format!("{name}/material/{index}");
                materials.push(
                    load_material(context, label, "bitmaps", &mut textures, index, record).await,
                );
            }
            let mut parts = Vec::new();
            for (material, mesh, source) in cycle_meshes(&cycle, &frames[0]) {
                let mesh = context.add_labeled_asset(format!("{name}/mesh/{material}"), mesh);
                parts.push(MonsterPart {
                    mesh,
                    material: materials[material].handle.clone(),
                    ambient: materials[material].ambient,
                    emission: materials[material].emission,
                    source,
                });
            }
            let frames_per_second = fps.unwrap_or(cycle.frames_per_second);
            cycles.insert(name.clone(), MonsterCycleModel { frames_per_second, parts, frames });
        }
        if !cycles.contains_key("IDLE") {
            warn!("{}: no IDLE cycle", context.path());
        }
        Ok(MonsterModel {
            scale: species.scale,
            walk_radius: species.walk_radius,
            movement_rate: species.movement_rate,
            cycles,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["mls"]
    }
}
