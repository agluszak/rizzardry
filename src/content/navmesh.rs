//! What the navmesh bake reads: a level's collision for one walker size
//! class, under the names the `navmesh://` asset source serves.

use bevy::asset::AssetPath;
use bevy::prelude::*;

use super::level::CollisionMesh;
use crate::bytes::{Bytes, DecodeError};

pub const NAVMESH_SOURCE: &str = "navmesh";
pub const NAVMESH_EXTENSION: &str = "navmesh";

/// The navmesh input for `level` (`levels/x/y.pvl`) and size class.
pub fn navmesh_input_path(level: &str, class: usize) -> String {
    let stem = level.strip_suffix(".pvl").unwrap_or(level);
    // No dot before the class: Bevy treats everything after the first dot
    // of a file name as its extension.
    format!("{stem}_class{class}.{NAVMESH_EXTENSION}")
}

/// The baked navmesh of `level` for size class `class`.
pub fn navmesh_path(level: &str, class: usize) -> AssetPath<'static> {
    AssetPath::from(navmesh_input_path(level, class)).with_source(NAVMESH_SOURCE)
}

/// A level's collision, as walkers of one size class meet it: its solid
/// surfaces and its collidable props at rest (doorways closed by doors are
/// gaps, joined by links while the doors are open).
#[derive(Debug)]
pub struct LevelCollision {
    pub collision: CollisionMesh,
    pub class: usize,
}

const MAGIC: &[u8; 4] = b"RZN3";

impl LevelCollision {
    /// The navmesh input's bytes: little-endian counts and values.
    pub fn encode(&self) -> Vec<u8> {
        let mesh = &self.collision;
        let mut out = MAGIC.to_vec();
        out.extend((self.class as u32).to_le_bytes());
        out.extend((mesh.vertices.len() as u32).to_le_bytes());
        for vertex in &mesh.vertices {
            out.extend(vertex.to_array().into_iter().flat_map(f32::to_le_bytes));
        }
        out.extend((mesh.triangles.len() as u32).to_le_bytes());
        for (triangle, &walkable) in mesh.triangles.iter().zip(&mesh.walkable) {
            out.extend(triangle.iter().flat_map(|v| v.to_le_bytes()));
            out.push(u8::from(walkable));
        }
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        let mut bytes = Bytes::new(data);
        if &bytes.array::<4>()? != MAGIC {
            return Err(bytes.invalid("not a navmesh input"));
        }
        let class = bytes.u32()? as usize;
        let count = bytes.u32()? as usize;
        let vertices = bytes.records(count, 12, |b| Ok(Vec3::from_array(b.f32s()?)))?;
        let count = bytes.u32()? as usize;
        let triangles = bytes.records(count, 13, |b| Ok((b.array::<12>()?, b.u8()? != 0)))?;
        let mut collision = CollisionMesh::default();
        for (triangle, walkable) in triangles {
            let corners: [u32; 3] = std::array::from_fn(|i| {
                u32::from_le_bytes(triangle[i * 4..i * 4 + 4].try_into().unwrap())
            });
            if corners.iter().any(|&v| v as usize >= vertices.len()) {
                return Err(bytes.invalid("triangle corner out of range"));
            }
            collision.triangles.push(corners);
            collision.walkable.push(walkable);
        }
        collision.vertices = vertices;
        Ok(LevelCollision { collision, class })
    }
}
