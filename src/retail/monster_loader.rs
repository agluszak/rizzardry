//! Loads a retail monster species (`data/monsters/<name>.mls`) and its
//! cycles as a [`MonsterModel`]. Body metadata has its own typed loader,
//! which finishes without reading cycles or constructing render assets.

use std::collections::HashMap;

use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages, io::Reader};
use bevy::camera::primitives::Aabb;
use bevy::mesh::{
    Indices, PrimitiveTopology,
    morph::{MAX_MORPH_WEIGHTS, MorphAttributes},
};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::level_loader::{LoadedTextures, PvlLoadError, load_material, sibling};
use super::mesh_record::FACE_TWO_SIDED;
use super::monster::{MonsterCycle, read_cycle, read_species};
use super::space::retail_position;
use crate::content::monster::{MonsterBody, MonsterCycleModel, MonsterModel, MonsterPart};

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
        // Shared by all instances; CPU access is retained for morph-aware picking.
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

/// Expand the source mapping once, in target-major render-vertex order.
fn morph_targets(mesh: &mut Mesh, frames: &[Vec<Vec3>], source: &[u32]) -> Aabb {
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    let mut targets = Vec::with_capacity(frames.len() * source.len());
    for frame in frames {
        for &vertex in source {
            let v = vertex as usize;
            let position = frame[v];
            low = low.min(position);
            high = high.max(position);
            targets.push(MorphAttributes::new(position - frames[0][v], Vec3::ZERO, Vec3::ZERO));
        }
    }
    mesh.set_morph_targets(targets);
    Aabb::from_min_max(low, high)
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
            if cycle.frames.len() > MAX_MORPH_WEIGHTS {
                return Err(PvlLoadError::MorphTargets(cycle.frames.len()));
            }
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
            for (material, mut mesh, source) in cycle_meshes(&cycle, &frames[0]) {
                let bounds = morph_targets(&mut mesh, &frames, &source);
                let mesh = context.add_labeled_asset(format!("{name}/mesh/{material}"), mesh);
                parts.push(MonsterPart { mesh, surface: materials[material].clone(), bounds });
            }
            let frames_per_second = fps.unwrap_or(cycle.frames_per_second);
            cycles.insert(
                name.clone(),
                MonsterCycleModel { frames_per_second, parts, frame_count: frames.len() },
            );
        }
        if !cycles.contains_key("IDLE") {
            warn!("{}: no IDLE cycle", context.path());
        }
        Ok(MonsterModel { cycles })
    }

    fn extensions(&self) -> &[&str] {
        &["mls"]
    }
}

/// The gameplay view of a species file. Bevy selects the `.mls` loader by
/// requested asset type, so the same authored file has independent readiness
/// for required body metadata and optional visual construction.
#[derive(Default, TypePath)]
pub struct MlsBodyLoader;

impl AssetLoader for MlsBodyLoader {
    type Asset = MonsterBody;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<MonsterBody, Self::Error> {
        let mut text = Vec::new();
        reader.read_to_end(&mut text).await?;
        Ok(read_species(&String::from_utf8_lossy(&text)).body)
    }

    fn extensions(&self) -> &[&str] {
        &["mls"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_morphs_reconstruct_cycle_poses_with_conservative_bounds() {
        let files = crate::retail::files::retail_files();
        for species in ["savant_trooper", "dark_savant", "baruta_fish"] {
            let bytes = files.read(&format!("data/monsters/{species}.mls")).unwrap().unwrap();
            let definition = read_species(&String::from_utf8_lossy(&bytes));
            for (name, file, fps) in definition.cycles {
                let bytes = files.read(&format!("data/monsters/{file}.mon")).unwrap().unwrap();
                let cycle = read_cycle(&bytes).unwrap();
                let frames: Vec<Vec<Vec3>> = cycle
                    .frames
                    .iter()
                    .map(|f| f.iter().map(|&v| retail_position(v)).collect())
                    .collect();
                assert!(frames.len() <= MAX_MORPH_WEIGHTS);
                let model = MonsterCycleModel {
                    frame_count: frames.len(),
                    frames_per_second: fps.unwrap_or(cycle.frames_per_second),
                    parts: vec![],
                };
                if species == "baruta_fish" && name == "WALK" {
                    assert_eq!((cycle.frames_per_second, model.frames_per_second), (30., 60.));
                }
                let mut weights = vec![0.; frames.len()];
                for (_, mut mesh, source) in cycle_meshes(&cycle, &frames[0]) {
                    let bounds = morph_targets(&mut mesh, &frames, &source);
                    let targets = mesh.get_morph_targets().unwrap();
                    for index in 0..frames.len() {
                        let frame = index as f32 + 0.375;
                        model.write_weights(frame, &mut weights);
                        for (render, &vertex) in source.iter().enumerate() {
                            let v = vertex as usize;
                            let expected = frames[index][v]
                                .lerp(frames[(index + 1).min(frames.len() - 1)][v], 0.375);
                            let actual = weights.iter().enumerate().fold(
                                frames[0][v],
                                |position, (target, &w)| {
                                    position + targets[target * source.len() + render].position * w
                                },
                            );
                            assert!(actual.abs_diff_eq(expected, 1e-5), "{species} {name} {frame}");
                            assert!(
                                (actual - Vec3::from(bounds.center))
                                    .abs()
                                    .cmple(Vec3::from(bounds.half_extents) + Vec3::splat(1e-5))
                                    .all()
                            );
                        }
                    }
                }
            }
        }
    }
}
