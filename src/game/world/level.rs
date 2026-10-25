//! The active level: one root entity owning everything level-specific.
//!
//! The root carries [`Level`]; its geometry, collider and navigation are
//! built under it as their assets load. Despawning the root unloads the
//! level. The party is not level-owned and survives travel.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::NavMesh3d;

use crate::content::level::LevelGeometry;

/// The root of the level the party is in.
#[derive(Component, Debug)]
#[require(Transform, Visibility, AwaitingGeometry, AwaitingNavmesh)]
pub struct Level {
    pub geometry: Handle<LevelGeometry>,
    pub navmesh: Handle<NavMesh3d>,
}

/// The level's geometry and collider have not been built yet.
#[derive(Component, Debug, Default)]
pub struct AwaitingGeometry;

/// The level's navigation has not been installed yet.
#[derive(Component, Debug, Default)]
pub struct AwaitingNavmesh;

/// Sent once a level's geometry and collider exist.
#[derive(Message, Debug, Clone, Copy)]
pub struct LevelReady;

/// Build the static geometry and collider under the level root once the
/// geometry has loaded.
pub fn build_level_geometry(
    mut commands: Commands,
    levels: Query<(Entity, &Level), With<AwaitingGeometry>>,
    geometry: Res<Assets<LevelGeometry>>,
    mut ready: MessageWriter<LevelReady>,
) {
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        let mut root = commands.entity(root);
        root.remove::<AwaitingGeometry>();
        match Collider::try_trimesh_with_config(
            geometry.collision.vertices.clone(),
            geometry.collision.triangles.clone(),
            TrimeshFlags::MERGE_DUPLICATE_VERTICES
                | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES
                | TrimeshFlags::FIX_INTERNAL_EDGES,
        ) {
            Ok(collider) => {
                root.insert((RigidBody::Static, collider));
            }
            Err(error) => error!("level collider: {error:?}"),
        }
        root.with_children(|parent| {
            for part in &geometry.parts {
                parent.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(part.material.clone())));
            }
        });
        info!(
            "level ready: {} parts, {} collision triangles",
            geometry.parts.len(),
            geometry.collision.triangles.len()
        );
        ready.write(LevelReady);
    }
}
