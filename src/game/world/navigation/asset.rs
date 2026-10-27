//! Navmeshes, baked when a level asks for them and cached on disk.
//!
//! Each retail level has a `navmesh://` input per walker size class: the
//! class and the level's collision files. [`NavmeshLoader`] keys the cache
//! by those bytes, the bake settings and this format's version; on a miss
//! it decodes the collision (through a decoder the app supplies, since the
//! retail formats are not the game's to read), bakes off the main thread
//! and writes the result whole, so concurrent games and tests can share
//! the cache. Only the levels entered are baked.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy_landmass::{HeightNavigationMesh3d, HeightPolygon, NavMesh3d, NavigationMesh3d};

use super::bake::{NavmeshBakeSettings, SIZE_CLASSES, bake};
use crate::bytes::{Bytes, Truncated};
use crate::content::navmesh::{LevelCollision, NAVMESH_EXTENSION};

#[derive(Debug, thiserror::Error)]
pub enum NavmeshAssetError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Truncated(#[from] Truncated),
    #[error("navmesh input: {0}")]
    Input(String),
    #[error("navmesh bake failed: {0}")]
    Bake(String),
    #[error("not a navmesh file of format version {VERSION}")]
    Format,
    #[error("invalid navmesh: {0}")]
    Invalid(String),
}

// The cached file: a magic number, a format version, then the mesh as
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

/// Where baked navmeshes are cached, under the asset base path.
pub const CACHE: &str = "imported_assets/navmesh";

/// Loads a level's navmesh for one size class from its input, baking and
/// caching it on a miss, validated for Landmass.
#[derive(TypePath)]
pub struct NavmeshLoader {
    /// Where baked navmeshes are kept, by input path and key.
    pub cache: PathBuf,
    /// Reads a navmesh input's collision.
    pub decode: fn(&[u8]) -> Result<LevelCollision, String>,
}

impl NavmeshLoader {
    /// The cached file for an input: a re-bake follows any change to the
    /// level, the settings or the format. (A changed hasher only re-bakes.)
    /// The navmesh of an input (at `input_path` under `navmesh://`): from
    /// the cache, or baked and cached, replacing what the input's earlier
    /// versions left there.
    pub fn navmesh(
        &self,
        input_path: &Path,
        input: &[u8],
    ) -> Result<NavigationMesh3d, NavmeshAssetError> {
        let cached = self.cached(input_path, input);
        if let Some(mesh) = std::fs::read(&cached).ok().and_then(|bytes| decode(&bytes).ok()) {
            return Ok(mesh);
        }
        let mesh = self.bake(input)?;
        match publish(&cached, &encode(&mesh)) {
            Ok(()) => remove_superseded(&cached),
            Err(error) => warn!("could not cache {}: {error}", cached.display()),
        }
        Ok(mesh)
    }

    /// The cached file for an input: a re-bake follows any change to the
    /// level, the settings or the format. (A changed hasher only re-bakes.)
    fn cached(&self, input_path: &Path, input: &[u8]) -> PathBuf {
        let mut hasher = std::hash::DefaultHasher::new();
        (input, format!("{:?}", NavmeshBakeSettings::default()), VERSION).hash(&mut hasher);
        self.cache
            .join(input_path)
            .with_extension(format!("{:016x}.{NAVMESH_EXTENSION}", hasher.finish()))
    }

    fn bake(&self, input: &[u8]) -> Result<NavigationMesh3d, NavmeshAssetError> {
        let level = (self.decode)(input).map_err(NavmeshAssetError::Input)?;
        let agent_radius = *SIZE_CLASSES.get(level.class).ok_or(NavmeshAssetError::Format)?;
        let settings = NavmeshBakeSettings { agent_radius, ..default() };
        let (mesh, report) = bake(&level.collision, &settings).map_err(NavmeshAssetError::Bake)?;
        if report.inverted_polygons > 0 {
            warn!("navmesh dropped folded polygons: {report:?}");
        } else if report != Default::default() {
            info!("navmesh repaired: {report:?}");
        }
        Ok(mesh)
    }
}

/// Write `bytes` to `path` whole: to a temporary file, then renamed over it.
fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a cached file lies in a directory"))?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

/// Remove the same input's files under other keys (`<input>.<key>.navmesh`).
/// Another game still using an old key only re-bakes.
fn remove_superseded(current: &Path) {
    let (Some(dir), Some(name)) = (current.parent(), current.file_name().and_then(|n| n.to_str()))
    else {
        return;
    };
    let Some((input, _)) = name.split_once('.') else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let other = entry.file_name();
        let Some(other) = other.to_str() else { continue };
        if other != name
            && other.strip_prefix(input).is_some_and(|rest| rest.starts_with('.'))
            && other.ends_with(&format!(".{NAVMESH_EXTENSION}"))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

impl AssetLoader for NavmeshLoader {
    type Asset = NavMesh3d;
    type Settings = ();
    type Error = NavmeshAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<NavMesh3d, NavmeshAssetError> {
        let mut input = Vec::new();
        reader.read_to_end(&mut input).await?;
        let mesh = self.navmesh(context.path().path(), &input)?;
        let valid = mesh.validate().map_err(|e| NavmeshAssetError::Invalid(format!("{e:?}")))?;
        Ok(NavMesh3d { nav_mesh: Arc::new(valid) })
    }

    fn extensions(&self) -> &[&str] {
        &[NAVMESH_EXTENSION]
    }
}
