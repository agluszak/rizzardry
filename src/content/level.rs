//! A level's static geometry as native Bevy assets.

use bevy::prelude::*;

/// One renderable piece of the static level: a compiled octree mesh's
/// polygons that share a material.
#[derive(Clone, Debug, Reflect)]
pub struct LevelPart {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// Immutable static geometry shared by every visit to a level.
#[derive(Asset, Clone, Debug, Reflect)]
pub struct LevelGeometry {
    pub parts: Vec<LevelPart>,
    /// Solid triangles in world space (metres), for building the collider.
    pub collision_vertices: Vec<Vec3>,
    pub collision_triangles: Vec<[u32; 3]>,
    pub bounds: (Vec3, Vec3),
}
