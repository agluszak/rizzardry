//! Navmeshes through Bevy's asset processor.
//!
//! Each retail level has a `navmesh://<level>.navmesh` input (its `.oct`
//! collision and `.pvl` placement bytes, see [`crate::retail::source`]). The processor bakes it once with
//! `LoadTransformAndSave<LevelCollisionLoader, BakeNavmesh, NavmeshSaver>` and caches the result under `imported_assets/navmesh`,
//! re-baking only when the level or the [`NavmeshBakeSettings`] in its meta
//! change. The game loads the processed file as a Landmass [`NavMesh3d`].

use std::sync::Arc;

use bevy::asset::io::{Reader, Writer};
use bevy::asset::saver::{AssetSaver, SavedAsset};
use bevy::asset::transformer::{AssetTransformer, TransformedAsset};
use bevy::asset::{AssetLoader, AssetPath, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::tasks::futures_lite::AsyncWriteExt;
use bevy_landmass::{HeightNavigationMesh3d, HeightPolygon, NavMesh3d, NavigationMesh3d};

use super::bake::{NavmeshBakeSettings, SIZE_CLASSES, bake};
use crate::content::level::CollisionMesh;
use crate::retail::bytes::{Bytes, Truncated};
use crate::retail::oct::read_collision;
use crate::retail::prop::PROP_COLLIDABLE;
use crate::retail::prop_loader::{part_transforms, record_collision};
use crate::retail::pvl::{PvlError, read_level_geometry};
use crate::retail::sections::read_sections;
use crate::retail::source::navmesh_input;
use crate::retail::source::{NAVMESH_EXTENSION, NAVMESH_SOURCE, navmesh_input_path};

/// The navmesh path for a retail level path (`levels/x/y.pvl`) and walker
/// size class.
pub fn navmesh_path(level: &str, class: usize) -> AssetPath<'static> {
    AssetPath::from(navmesh_input_path(level, class)).with_source(NAVMESH_SOURCE)
}

/// A level's collision and the walker radius to bake for: the bake's input.
#[derive(Asset, TypePath, Debug)]
pub struct LevelCollision {
    pub collision: CollisionMesh,
    pub agent_radius: f32,
}

/// An unvalidated baked navmesh, between the transformer and the saver.
#[derive(Asset, TypePath)]
pub struct BakedNavmesh(pub NavigationMesh3d);

#[derive(Debug, thiserror::Error)]
pub enum NavmeshAssetError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Pvl(#[from] PvlError),
    #[error(transparent)]
    Truncated(#[from] Truncated),
    #[error("navmesh bake failed: {0}")]
    Bake(String),
    #[error("not a navmesh file of format version {VERSION}")]
    Format,
    #[error("invalid navmesh: {0}")]
    Invalid(String),
}

/// Reads a level's collision from its navmesh input (`.oct` and `.pvl`).
#[derive(Default, TypePath)]
pub struct LevelCollisionLoader;

impl AssetLoader for LevelCollisionLoader {
    type Asset = LevelCollision;
    type Settings = ();
    type Error = NavmeshAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<LevelCollision, NavmeshAssetError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let (class, octree, level) = navmesh_input(&data).ok_or(NavmeshAssetError::Format)?;
        let agent_radius = *SIZE_CLASSES.get(class as usize).ok_or(NavmeshAssetError::Format)?;
        Ok(LevelCollision { collision: level_collision(octree, level)?, agent_radius })
    }
}

/// The navmesh bake's input: the level's solid collision surfaces, plus its
/// collidable props in their rest pose as obstacles. Doorways closed by doors
/// become gaps, joined by links while the doors are open.
pub fn level_collision(octree: &[u8], level: &[u8]) -> Result<CollisionMesh, PvlError> {
    let mut collision = read_collision(octree)?.solid_mesh();
    let geometry = read_level_geometry(level)?;
    let sections = read_sections(&mut Bytes::new(&level[geometry.end..]))?;
    for prop in sections.props.iter().filter(|p| p.flags & PROP_COLLIDABLE != 0) {
        for (part, (transform, _)) in prop.detailed_parts().iter().zip(part_transforms(prop)) {
            let record = &part.mesh.frames[part.mesh.first_frame];
            collision.append(&record_collision(record), &transform, false);
        }
    }
    Ok(collision)
}

#[derive(Default, TypePath)]
pub struct BakeNavmesh;

impl AssetTransformer for BakeNavmesh {
    type AssetInput = LevelCollision;
    type AssetOutput = BakedNavmesh;
    type Settings = NavmeshBakeSettings;
    type Error = NavmeshAssetError;

    async fn transform<'a>(
        &'a self,
        asset: TransformedAsset<LevelCollision>,
        settings: &'a NavmeshBakeSettings,
    ) -> Result<TransformedAsset<BakedNavmesh>, NavmeshAssetError> {
        let settings = NavmeshBakeSettings { agent_radius: asset.agent_radius, ..settings.clone() };
        let (mesh, report) = bake(&asset.collision, &settings).map_err(NavmeshAssetError::Bake)?;
        if report.inverted_polygons > 0 {
            warn!("navmesh dropped folded polygons: {report:?}");
        } else if report != Default::default() {
            info!("navmesh repaired: {report:?}");
        }
        Ok(asset.replace_asset(BakedNavmesh(mesh)))
    }
}

// The processed file: a magic number, a format version, then the mesh as
// little-endian counts and values.
const MAGIC: &[u8; 4] = b"RZNM";
const VERSION: u32 = 1;

fn encode(mesh: &NavigationMesh3d) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let u32 = |out: &mut Vec<u8>, v: usize| out.extend((v as u32).to_le_bytes());
    let vec3s = |out: &mut Vec<u8>, vs: &[Vec3]| {
        u32(out, vs.len());
        for v in vs {
            for c in v.to_array() {
                out.extend(c.to_le_bytes());
            }
        }
    };
    out.extend(VERSION.to_le_bytes());
    vec3s(&mut out, &mesh.vertices);
    u32(&mut out, mesh.polygons.len());
    for (polygon, &kind) in mesh.polygons.iter().zip(&mesh.polygon_type_indices) {
        u32(&mut out, kind);
        u32(&mut out, polygon.len());
        for &v in polygon {
            u32(&mut out, v);
        }
    }
    match &mesh.height_mesh {
        None => out.push(0),
        Some(height) => {
            out.push(1);
            vec3s(&mut out, &height.vertices);
            u32(&mut out, height.polygons.len());
            for p in &height.polygons {
                for v in
                    [p.base_vertex_index, p.vertex_count, p.base_triangle_index, p.triangle_count]
                {
                    out.extend(v.to_le_bytes());
                }
            }
            u32(&mut out, height.triangles.len());
            for t in &height.triangles {
                out.extend(t);
            }
        }
    }
    out
}

fn decode(data: &[u8]) -> Result<NavigationMesh3d, NavmeshAssetError> {
    let mut bytes = Bytes::new(data);
    if &bytes.array::<4>()? != MAGIC || bytes.u32()? != VERSION {
        return Err(NavmeshAssetError::Format);
    }
    let vec3s = |bytes: &mut Bytes| -> Result<Vec<Vec3>, Truncated> {
        let count = bytes.u32()? as usize;
        bytes.records(count, 12, |b| Ok(Vec3::from_array(b.f32s()?)))
    };
    let vertices = vec3s(&mut bytes)?;
    let count = bytes.u32()? as usize;
    let mut polygons = Vec::with_capacity(count.min(bytes.remaining() / 8));
    let mut polygon_type_indices = Vec::with_capacity(polygons.capacity());
    for _ in 0..count {
        polygon_type_indices.push(bytes.u32()? as usize);
        let corners = bytes.u32()? as usize;
        polygons.push(bytes.records(corners, 4, |b| Ok(b.u32()? as usize))?);
    }
    let height_mesh = match bytes.u8()? {
        0 => None,
        _ => {
            let vertices = vec3s(&mut bytes)?;
            let count = bytes.u32()? as usize;
            let polygons = bytes.records(count, 16, |b| {
                Ok(HeightPolygon {
                    base_vertex_index: b.u32()?,
                    vertex_count: b.u32()?,
                    base_triangle_index: b.u32()?,
                    triangle_count: b.u32()?,
                })
            })?;
            let count = bytes.u32()? as usize;
            let triangles = bytes.records(count, 3, |b| b.array::<3>())?;
            Some(HeightNavigationMesh3d { polygons, triangles, vertices })
        }
    };
    Ok(NavigationMesh3d { vertices, polygons, polygon_type_indices, height_mesh })
}

#[derive(Default, TypePath)]
pub struct NavmeshSaver;

impl AssetSaver for NavmeshSaver {
    type Asset = BakedNavmesh;
    type Settings = ();
    type OutputLoader = NavmeshLoader;
    type Error = NavmeshAssetError;

    async fn save(
        &self,
        writer: &mut Writer,
        asset: SavedAsset<'_, '_, BakedNavmesh>,
        _: &(),
        _: AssetPath<'_>,
    ) -> Result<(), NavmeshAssetError> {
        writer.write_all(&encode(&asset.0)).await?;
        Ok(())
    }
}

/// Loads a processed navmesh, validated for Landmass.
#[derive(Default, TypePath)]
pub struct NavmeshLoader;

impl AssetLoader for NavmeshLoader {
    type Asset = NavMesh3d;
    type Settings = ();
    type Error = NavmeshAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<NavMesh3d, NavmeshAssetError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let mesh = decode(&data)?;
        let valid = mesh.validate().map_err(|e| NavmeshAssetError::Invalid(format!("{e:?}")))?;
        Ok(NavMesh3d { nav_mesh: Arc::new(valid) })
    }

    fn extensions(&self) -> &[&str] {
        &[NAVMESH_EXTENSION]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_format_round_trips() {
        let mesh = NavigationMesh3d {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::NEG_Z],
            polygons: vec![vec![0, 1, 2]],
            polygon_type_indices: vec![255],
            height_mesh: Some(HeightNavigationMesh3d {
                polygons: vec![HeightPolygon {
                    base_vertex_index: 0,
                    vertex_count: 3,
                    base_triangle_index: 0,
                    triangle_count: 1,
                }],
                triangles: vec![[0, 1, 2]],
                vertices: vec![Vec3::ZERO, Vec3::X, Vec3::NEG_Z],
            }),
        };
        let decoded = decode(&encode(&mesh)).unwrap();
        assert_eq!(decoded.vertices, mesh.vertices);
        assert_eq!(decoded.polygons, mesh.polygons);
        assert_eq!(decoded.polygon_type_indices, mesh.polygon_type_indices);
        let (a, b) = (decoded.height_mesh.unwrap(), mesh.height_mesh.unwrap());
        assert_eq!((a.vertices, a.triangles), (b.vertices, b.triangles));
        assert!(decode(b"RZNM\x02\0\0\0").is_err());
    }
}
