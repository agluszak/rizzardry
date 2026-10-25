//! Navmeshes through Bevy's asset processor.
//!
//! Each retail level has a `navmesh://<level>.navmesh` input (its `.pvl`
//! bytes, see [`crate::retail::source`]). The processor bakes it once with
//! [`NavmeshProcess`] and caches the result under `imported_assets/navmesh`,
//! re-baking only when the level or the [`NavmeshBakeSettings`] in its meta
//! change. The game loads the processed file as a Landmass [`NavMesh3d`].

use std::sync::Arc;

use bevy::asset::io::{Reader, Writer};
use bevy::asset::processor::LoadTransformAndSave;
use bevy::asset::saver::{AssetSaver, SavedAsset};
use bevy::asset::transformer::{AssetTransformer, TransformedAsset};
use bevy::asset::{AssetLoader, AssetPath, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::tasks::futures_lite::AsyncWriteExt;
use bevy_landmass::{HeightNavigationMesh3d, HeightPolygon, NavMesh3d, NavigationMesh3d};

use super::bake::{NavmeshBakeSettings, bake};
use crate::retail::bytes::{Bytes, Truncated};
use crate::retail::level_loader::collision_triangles;
use crate::retail::pvl::{PvlError, read_level_geometry};
use crate::retail::source::{NAVMESH_EXTENSION, NAVMESH_SOURCE};

/// The import-time processor for `.navmesh` inputs.
pub type NavmeshProcess = LoadTransformAndSave<LevelCollisionLoader, BakeNavmesh, NavmeshSaver>;

/// The navmesh path for a retail level path, e.g. `levels/x/y.pvl`.
pub fn navmesh_path(level: &str) -> AssetPath<'static> {
    let stem = level.strip_suffix(".pvl").unwrap_or(level);
    AssetPath::from(format!("{stem}.{NAVMESH_EXTENSION}")).with_source(NAVMESH_SOURCE)
}

/// A level's solid triangles: the bake's input.
#[derive(Asset, TypePath, Debug)]
pub struct LevelCollision {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
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

/// Reads a level's collision triangles from its `.pvl` bytes.
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
        let (vertices, triangles) = collision_triangles(&read_level_geometry(&data)?);
        Ok(LevelCollision { vertices, triangles })
    }
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
        let (mesh, report) =
            bake(&asset.vertices, &asset.triangles, settings).map_err(NavmeshAssetError::Bake)?;
        if report != Default::default() {
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
