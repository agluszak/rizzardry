//! Fog, clear colour and sky light from the level environment and the time
//! of day.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::party::body::PartyCamera;
use crate::game::time::GameClock;
use crate::game::world::level::{Level, LevelPhase, PropPart};
use crate::game::world::props::Prop;
use crate::presentation::billboards::FaceCamera;

/// The level and phase the presentation was last updated for.
#[derive(Default)]
pub struct Shown(Option<(AssetId<LevelGeometry>, u8)>);

/// Apply the environment when the level or the time-of-day phase changes:
/// linear fog toward the phase's fog colour (which is also the clear
/// colour), the far clip at the view distance, and vertex colours relit
/// with the phase's sky light.
pub fn apply_environment(
    mut shown: Local<Shown>,
    clock: Res<GameClock>,
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clear: ResMut<ClearColor>,
    mut camera: Query<(Entity, &mut Projection), With<PartyCamera>>,
    mut commands: Commands,
) {
    let Some(level) = levels.iter().next() else { return };
    let Some(level_geometry) = geometry.get(&level.geometry) else { return };
    let Ok((camera, mut projection)) = camera.single_mut() else { return };
    let phase = clock.phase();
    let key = (level.geometry.id(), phase);
    if shown.0 == Some(key) {
        return;
    }
    shown.0 = Some(key);
    let Some(environment) = &level_geometry.environment else { return };

    let fog = environment.fog_colours[phase as usize];
    clear.0 = Color::srgb(fog.x, fog.y, fog.z);
    let far = environment.view_distance;
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.far = far;
    }
    commands.entity(camera).insert(DistanceFog {
        color: Color::srgb(fog.x, fog.y, fog.z),
        falloff: FogFalloff::Linear {
            start: far * environment.fog_range.0,
            end: far * environment.fog_range.1,
        },
        ..default()
    });

    let sky = environment.sky_light(phase);
    let parts = level_geometry
        .parts
        .iter()
        .chain(level_geometry.props.iter().flat_map(|p| &p.parts).flat_map(|p| &p.meshes));
    for part in parts {
        if let Some(mut mesh) = meshes.get_mut(&part.mesh) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, part.lighting.colours(sky));
        }
    }
}

/// The level's meshes have been attached.
#[derive(Component, Debug)]
pub struct LevelVisuals;

/// Give an active level its looks: the static geometry under its root, each
/// prop part's meshes under the part, and sprites turning to the view.
pub fn attach_level_visuals(
    mut commands: Commands,
    levels: Query<(Entity, &Level, &LevelPhase, Option<&Children>), Without<LevelVisuals>>,
    geometry: Res<Assets<LevelGeometry>>,
    props: Query<(&Prop, &Children)>,
    parts: Query<&PropPart>,
) {
    for (root, level, phase, children) in &levels {
        let (LevelPhase::Active, Some(geometry)) = (phase, geometry.get(&level.geometry)) else {
            continue;
        };
        commands.entity(root).insert(LevelVisuals);
        for part in &geometry.parts {
            commands.spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                ChildOf(root),
            ));
        }
        for &child in children.into_iter().flatten() {
            let Ok((prop, prop_parts)) = props.get(child) else { continue };
            let Some(model) = geometry.props.get(prop.definition) else { continue };
            for &entity in prop_parts {
                let Some(part) = parts.get(entity).ok().and_then(|p| model.parts.get(p.part))
                else {
                    continue;
                };
                if model.face_camera {
                    commands.entity(entity).insert(FaceCamera);
                }
                for mesh in &part.meshes {
                    commands.spawn((
                        Mesh3d(mesh.mesh.clone()),
                        MeshMaterial3d(mesh.material.clone()),
                        ChildOf(entity),
                    ));
                }
            }
        }
    }
}
