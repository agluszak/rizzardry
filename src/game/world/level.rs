//! The active level's static geometry and collider.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::content::level::LevelGeometry;

/// The level the session is in, and whether its world has been built.
#[derive(Resource, Debug)]
pub struct ActiveLevel {
    pub geometry: Handle<LevelGeometry>,
    pub navmesh: Handle<bevy_landmass::NavMesh3d>,
    pub spawned: bool,
}

/// Root of the static level; despawning it removes the level's geometry.
#[derive(Component, Debug)]
pub struct LevelRoot;

/// Sent once the active level's geometry and collider exist.
#[derive(Message, Debug, Clone, Copy)]
pub struct LevelReady;

pub fn spawn_active_level(
    mut commands: Commands,
    mut active: ResMut<ActiveLevel>,
    levels: Res<Assets<LevelGeometry>>,
    mut ready: MessageWriter<LevelReady>,
) {
    if active.spawned {
        return;
    }
    let Some(level) = levels.get(&active.geometry) else { return };
    active.spawned = true;

    let collider = Collider::try_trimesh_with_config(
        level.collision.vertices.clone(),
        level.collision.triangles.clone(),
        TrimeshFlags::MERGE_DUPLICATE_VERTICES
            | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES
            | TrimeshFlags::FIX_INTERNAL_EDGES,
    );
    let mut root = commands.spawn((
        LevelRoot,
        Name::new("Level"),
        Transform::default(),
        Visibility::default(),
    ));
    match collider {
        Ok(collider) => {
            root.insert((RigidBody::Static, collider));
        }
        Err(error) => error!("level collider: {error:?}"),
    }
    root.with_children(|parent| {
        for part in &level.parts {
            parent.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(part.material.clone())));
        }
    });
    info!(
        "level ready: {} parts, {} collision triangles",
        level.parts.len(),
        level.collision.triangles.len()
    );
    ready.write(LevelReady);
}
