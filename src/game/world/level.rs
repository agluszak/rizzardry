//! The active level: one root entity owning everything level-specific.
//!
//! The root carries [`Level`]; its geometry, collider and navigation are
//! built under it as their assets load. Despawning the root unloads the
//! level. The party is not level-owned and survives travel.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy_landmass::NavMesh3d;

use super::navigation::asset::navmesh_path;
use super::navigation::bake::SIZE_CLASSES;
use super::props::{Prop, PropMotion, pose};
use super::scripts::LevelScript;
use super::triggers::LevelState;
use crate::content::level::LevelGeometry;
use crate::content::levels::LevelInfo;
use crate::content::ui::TextTable;
use crate::presentation::billboards::FaceCamera;
use crate::presentation::sky::{Celestial, Cloud};

/// The root of the level the party is in.
#[derive(Component, Debug)]
#[require(Transform, Visibility, AwaitingGeometry, AwaitingNavmesh, AwaitingLinks, AwaitingSky)]
pub struct Level {
    /// The retail path of its `.pvl`, which identifies it.
    pub path: String,
    pub geometry: Handle<LevelGeometry>,
    /// One navmesh per walker size class (`navigation::bake::SIZE_CLASSES`).
    pub navmeshes: Vec<Handle<NavMesh3d>>,
    /// The level's sky world, drawn behind it; none for levels without one.
    pub sky: Option<Handle<LevelGeometry>>,
    pub script: LevelScript,
    /// The messages its triggers show.
    pub messages: Option<Handle<TextTable>>,
}

impl Level {
    /// The level at a retail `.pvl` path, with its navmeshes and sky.
    pub fn load(path: &str, assets: &AssetServer) -> Self {
        Level {
            path: path.to_owned(),
            geometry: assets.load(format!("retail://{path}")),
            navmeshes: (0..SIZE_CLASSES.len())
                .map(|class| assets.load(navmesh_path(path, class)))
                .collect(),
            sky: LevelInfo::for_path(path)
                .and_then(LevelInfo::sky_path)
                .map(|sky| assets.load(format!("retail://{sky}"))),
            script: LevelInfo::for_path(path)
                .map_or_else(LevelScript::default, |info| LevelScript::for_location(info.location)),
            messages: LevelInfo::for_path(path).map(|info| {
                assets.load(format!("retail://data/messages/{}.msg", info.folder.to_lowercase()))
            }),
        }
    }
}

/// Leave the level for one of another level's entrances, e.g. `MO201`.
#[derive(Message, Debug, Clone)]
pub struct Travel {
    pub entrance: String,
}

/// Replace the level root with the entrance's level. The party stays and is
/// placed at the entrance once the new level is ready (retail
/// `TravelToLevel`). Retail first asks "Enter <level>?" when the trigger
/// says so; that question is not asked yet.
pub fn travel(
    mut commands: Commands,
    mut requests: MessageReader<Travel>,
    levels: Query<(Entity, &Level, Option<&LevelState>, &Children)>,
    props: Query<&PropMotion>,
    mut memory: ResMut<LevelMemory>,
    assets: Res<AssetServer>,
    mut start: ResMut<crate::game::party::body::PartyStart>,
) {
    let Some(request) = requests.read().last() else { return };
    let Some(info) = LevelInfo::for_entrance(&request.entrance) else {
        warn!("no level has entrance {}", request.entrance);
        return;
    };
    for (entity, level, state, children) in &levels {
        // Remember the level as the party leaves it; moving props are kept
        // where they were heading.
        if let Some(state) = state {
            let prop_frames =
                children.iter().filter_map(|c| props.get(c).ok()).map(|m| m.target).collect();
            memory.0.insert(level.path.clone(), LevelRecord { state: state.clone(), prop_frames });
        }
        commands.entity(entity).despawn();
    }
    let path = info.path();
    info!("travelling to {path} at {}", request.entrance);
    commands.spawn((Name::new(path.clone()), Level::load(&path, &assets)));
    *start = crate::game::party::body::PartyStart {
        entrance: Some(request.entrance.clone()),
        ..default()
    };
}

/// Levels the party has left, as they were when it left (props by their
/// order in the level). Not yet saved with the game.
#[derive(Resource, Debug, Default)]
pub struct LevelMemory(HashMap<String, LevelRecord>);

#[derive(Debug)]
pub struct LevelRecord {
    state: LevelState,
    prop_frames: Vec<f32>,
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
            let celestial = match prop.name.to_lowercase().as_str() {
                "sun" => Some(true),
                "moon" => Some(false),
                _ => None,
            };
            for part in &prop.parts {
                let part_entity = commands
                    .spawn((
                        Name::new(prop.name.clone()),
                        part.transform,
                        Visibility::default(),
                        ChildOf(sky_root),
                    ))
                    .id();
                if prop.face_camera {
                    commands.entity(part_entity).insert(FaceCamera);
                }
                if let Some(sun) = celestial {
                    commands
                        .entity(part_entity)
                        .insert(Celestial { sun, authored: part.transform.translation });
                }
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
    memory: Res<LevelMemory>,
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
        let remembered = memory.0.get(&level.path);
        let mut root = commands.entity(root);
        root.remove::<AwaitingGeometry>().insert((
            RigidBody::Static,
            collider,
            remembered
                .map_or_else(|| LevelState::new(geometry, level.script), |r| r.state.revisited()),
        ));
        root.with_children(|parent| {
            for part in &geometry.parts {
                parent.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(part.material.clone())));
            }
            for (index, prop) in geometry.props.iter().enumerate() {
                let frame =
                    remembered.and_then(|r| r.prop_frames.get(index)).copied().unwrap_or(0.0);
                let last_frame =
                    prop.parts.iter().map(|p| p.keys.len().saturating_sub(1)).max().unwrap_or(0);
                parent
                    .spawn((
                        Name::new(prop.name.clone()),
                        Prop {
                            trigger: prop.trigger.clone(),
                            door: prop.trigger.as_ref().is_some_and(|name| {
                                geometry
                                    .triggers
                                    .iter()
                                    .any(|t| &t.name == name && t.door.is_some())
                            }),
                            frames_per_second: prop.frames_per_second,
                            last_frame,
                            wraps: prop.wraps,
                            slots: prop.slots.clone(),
                        },
                        PropMotion { frame, target: frame },
                        Transform::default(),
                        Visibility::default(),
                    ))
                    .with_children(|prop_entity| {
                        for part in &prop.parts {
                            let at = if part.keys.is_empty() {
                                part.transform
                            } else {
                                pose(&part.keys, frame)
                            };
                            let mut part_entity = prop_entity.spawn((
                                PropPart { keys: part.keys.clone() },
                                at,
                                Visibility::default(),
                            ));
                            if prop.face_camera {
                                // Sprites turn with the view and do not collide.
                                part_entity.insert(FaceCamera);
                            } else {
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
