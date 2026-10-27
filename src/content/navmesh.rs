//! What the navmesh bake reads: a level's collision for one walker size
//! class, under the names the `navmesh://` asset source serves.

use bevy::asset::AssetPath;
use bevy::prelude::*;

use super::level::CollisionMesh;

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
