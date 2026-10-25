//! The active level: one root entity owning everything level-specific.
//!
//! The root carries [`Level`]; its geometry, collider and navigation are
//! built under it as their assets load. Despawning the root unloads the
//! level. The party is not level-owned and survives travel.

use avian3d::prelude::*;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy_landmass::NavMesh3d;

use super::props::{Prop, PropMotion};
use crate::content::level::LevelGeometry;
use crate::presentation::sky::Cloud;

/// The root of the level the party is in.
#[derive(Component, Debug)]
#[require(Transform, Visibility, AwaitingGeometry, AwaitingNavmesh, AwaitingLinks, AwaitingSky)]
pub struct Level {
    pub geometry: Handle<LevelGeometry>,
    pub navmesh: Handle<NavMesh3d>,
    /// The level's sky world, drawn behind it; none for levels without one.
    pub sky: Option<Handle<LevelGeometry>>,
}

/// The level's sky has not been built yet.
#[derive(Component, Debug, Default)]
pub struct AwaitingSky;

/// The render layer sky worlds are drawn on, by their own camera.
pub const SKY_LAYER: usize = 1;

/// Build the sky world under the level root, on the sky's render layer.
pub fn build_sky(
    mut commands: Commands,
    levels: Query<(Entity, &Level), With<AwaitingSky>>,
    geometry: Res<Assets<LevelGeometry>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (root, level) in &levels {
        let Some(sky) = &level.sky else {
            commands.entity(root).remove::<AwaitingSky>();
            continue;
        };
        let Some(sky) = geometry.get(sky) else { continue };
        commands.entity(root).remove::<AwaitingSky>();
        info!(
            "sky ready: {} parts, {} props, {} animated textures",
            sky.parts.len(),
            sky.props.len(),
            sky.animated_textures.len()
        );
        let layer = RenderLayers::layer(SKY_LAYER);
        let sky_root = commands
            .spawn((Name::new("Sky"), Transform::default(), Visibility::default(), ChildOf(root)))
            .id();
        for part in &sky.parts {
            commands.spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                layer.clone(),
                ChildOf(sky_root),
            ));
        }
        for prop in &sky.props {
            let cloud = prop.name.to_lowercase().starts_with("clouds");
            for part in &prop.parts {
                let part_entity = commands
                    .spawn((
                        Name::new(prop.name.clone()),
                        part.transform,
                        Visibility::default(),
                        ChildOf(sky_root),
                    ))
                    .id();
                for mesh in &part.meshes {
                    let mut entity = commands.spawn((
                        Mesh3d(mesh.mesh.clone()),
                        layer.clone(),
                        ChildOf(part_entity),
                    ));
                    if cloud {
                        // Clouds share retail's animated cloud material; keep
                        // each one's texture.
                        let texture = materials
                            .get(&mesh.material)
                            .and_then(|m| m.base_color_texture.clone());
                        let material = materials.add(StandardMaterial {
                            base_color_texture: texture,
                            unlit: true,
                            double_sided: true,
                            cull_mode: None,
                            alpha_mode: AlphaMode::Blend,
                            ..default()
                        });
                        entity.insert((MeshMaterial3d(material), Cloud));
                    } else {
                        entity.insert(MeshMaterial3d(mesh.material.clone()));
                    }
                }
            }
        }
    }
}

/// The level's geometry and collider have not been built yet.
#[derive(Component, Debug, Default)]
pub struct AwaitingGeometry;

/// The level's navigation has not been installed yet.
#[derive(Component, Debug, Default)]
pub struct AwaitingNavmesh;

/// One rigid part of a placed prop, with its animation poses.
#[derive(Component, Debug)]
pub struct PropPart {
    /// Poses along the part's path, starting with the rest pose; empty if
    /// the part does not move.
    pub keys: Vec<Transform>,
}

/// The level's navigation links have not been spawned yet.
#[derive(Component, Debug, Default)]
pub struct AwaitingLinks;

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
        let collider = match Collider::try_trimesh_with_config(
            geometry.collision.vertices.clone(),
            geometry.collision.triangles.clone(),
            TrimeshFlags::MERGE_DUPLICATE_VERTICES
                | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES
                | TrimeshFlags::FIX_INTERNAL_EDGES,
        ) {
            Ok(collider) => collider,
            Err(error) => {
                error!("level collider: {error:?}");
                commands.entity(root).remove::<AwaitingGeometry>();
                continue;
            }
        };
        let mut root = commands.entity(root);
        root.remove::<AwaitingGeometry>().insert((RigidBody::Static, collider));
        root.with_children(|parent| {
            for part in &geometry.parts {
                parent.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(part.material.clone())));
            }
            for prop in &geometry.props {
                let last_frame =
                    prop.parts.iter().map(|p| p.keys.len().saturating_sub(1)).max().unwrap_or(0);
                parent
                    .spawn((
                        Name::new(prop.name.clone()),
                        Prop::new(prop.interaction.clone(), prop.frames_per_second, last_frame),
                        PropMotion::default(),
                        Transform::default(),
                        Visibility::default(),
                    ))
                    .with_children(|prop_entity| {
                        for part in &prop.parts {
                            let mut part_entity = prop_entity.spawn((
                                PropPart { keys: part.keys.clone() },
                                part.transform,
                                Visibility::default(),
                            ));
                            match Collider::try_trimesh_with_config(
                                part.collision.vertices.clone(),
                                part.collision.triangles.clone(),
                                TrimeshFlags::MERGE_DUPLICATE_VERTICES
                                    | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES,
                            ) {
                                Ok(collider) => {
                                    // Parts with a path can move (doors, lifts).
                                    let body = if part.keys.is_empty() {
                                        RigidBody::Static
                                    } else {
                                        RigidBody::Kinematic
                                    };
                                    part_entity.insert((body, collider));
                                    // Props the party walks through can still be used.
                                    if !prop.collidable {
                                        part_entity.insert(Sensor);
                                    }
                                }
                                Err(error) => warn!("prop {} collider: {error:?}", prop.name),
                            }
                            part_entity.with_children(|meshes| {
                                for mesh in &part.meshes {
                                    meshes.spawn((
                                        Mesh3d(mesh.mesh.clone()),
                                        MeshMaterial3d(mesh.material.clone()),
                                    ));
                                }
                            });
                        }
                    });
            }
        });
        info!(
            "level ready: {} parts, {} props, {} collision triangles",
            geometry.parts.len(),
            geometry.props.len(),
            geometry.collision.triangles.len()
        );
        ready.write(LevelReady);
    }
}
