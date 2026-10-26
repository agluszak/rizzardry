//! Fog, clear colour and sky light from the level environment and the time
//! of day.

use bevy::prelude::*;

use crate::content::level::{LevelDefinition, LevelGeometry};
use crate::game::time::GameClock;
use crate::game::world::level::{Level, PropPart};
use crate::game::world::props::Prop;
use crate::presentation::billboards::FaceCamera;
use crate::presentation::party_view::PartyCamera;

/// Apply the environment when the level or the time-of-day phase changes:
/// linear fog toward the phase's fog colour (which is also the clear
/// colour), the far clip at the view distance, and vertex colours relit
/// with the phase's sky light.
pub fn apply_environment(
    mut shown: Local<Option<(Entity, bool, u8)>>,
    clock: Res<GameClock>,
    levels: Query<(Entity, &Level, &LevelVisuals)>,
    definitions: Res<Assets<LevelDefinition>>,
    geometry: Res<Assets<LevelGeometry>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clear: ResMut<ClearColor>,
    mut camera: Query<(Entity, &mut Projection), With<PartyCamera>>,
    mut commands: Commands,
) {
    let Ok((root, level, visual)) = levels.single() else { return };
    let Some(definition) = definitions.get(&level.geometry) else { return };
    let level_geometry = geometry.get(&visual.geometry);
    let Ok((camera, mut projection)) = camera.single_mut() else { return };
    let phase = clock.phase();
    let key = (root, level_geometry.is_some(), phase);
    if *shown == Some(key) {
        return;
    }
    *shown = Some(key);
    let Some(environment) = &definition.environment else { return };

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
    let Some(level_geometry) = level_geometry else { return };
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

/// Presentation owns optional render handles; a live level needs neither.
#[derive(Component, Debug)]
pub struct LevelVisuals {
    pub geometry: Handle<LevelGeometry>,
    pub sky: Option<Handle<LevelGeometry>>,
}

pub fn request_level_visuals(
    mut commands: Commands,
    assets: Res<AssetServer>,
    levels: Query<(Entity, &Level), Without<LevelVisuals>>,
) {
    for (root, level) in &levels {
        commands.entity(root).insert(LevelVisuals {
            geometry: assets.load(format!("retail://{}", level.path)),
            sky: crate::content::levels::LevelInfo::for_path(&level.path)
                .and_then(crate::content::levels::LevelInfo::sky_path)
                .map(|path| assets.load(format!("retail://{path}"))),
        });
    }
}

#[derive(Component, Debug)]
pub struct VisualsAttached;

/// Give an active level its looks: the static geometry under its root, each
/// prop part's meshes under the part, and sprites turning to the view.
pub fn attach_level_visuals(
    mut commands: Commands,
    levels: Query<(Entity, &LevelVisuals, Option<&Children>), Without<VisualsAttached>>,
    geometry: Res<Assets<LevelGeometry>>,
    props: Query<(&Prop, &Children)>,
    parts: Query<&PropPart>,
) {
    for (root, level, children) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        commands.entity(root).insert(VisualsAttached);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_visuals_receive_the_current_light_without_a_clock_change() {
        let mut app = App::new();
        app.init_resource::<GameClock>()
            .init_resource::<ClearColor>()
            .init_resource::<Assets<LevelDefinition>>()
            .init_resource::<Assets<LevelGeometry>>()
            .init_resource::<Assets<Mesh>>()
            .add_systems(Update, apply_environment);
        let definition =
            app.world_mut().resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition {
                environment: Some(crate::content::level::LevelEnvironment {
                    fog_range: (0.0, 1.0),
                    view_distance: 50.0,
                    intensity: 1.0,
                    fog_colours: vec![Vec3::ZERO; 256],
                    ambient_colours: vec![Vec3::splat(0.25); 256],
                }),
                ..default()
            });
        let visual = app.world_mut().resource_mut::<Assets<LevelGeometry>>().reserve_handle();
        app.world_mut().spawn((
            Level {
                path: "test".into(),
                geometry: definition,
                navmeshes: vec![],
                script: default(),
                messages: None,
            },
            LevelVisuals { geometry: visual.clone(), sky: None },
        ));
        app.world_mut().spawn((PartyCamera, Projection::Perspective(default())));
        app.update();
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        ));
        app.world_mut()
            .resource_mut::<Assets<LevelGeometry>>()
            .insert(
                &visual,
                LevelGeometry {
                    parts: vec![crate::content::level::LevelPart {
                        mesh: mesh.clone(),
                        material: default(),
                        lighting: crate::content::level::VertexLighting {
                            fixed: vec![Vec3::ZERO],
                            sun: vec![1.0],
                            reflectance: Vec3::ONE,
                        },
                    }],
                    ..default()
                },
            )
            .unwrap();
        app.update();
        let mesh = app.world().resource::<Assets<Mesh>>().get(&mesh).unwrap();
        assert!(matches!(mesh.attribute(Mesh::ATTRIBUTE_COLOR),
            Some(bevy::mesh::VertexAttributeValues::Float32x4(colours)) if colours == &[[0.25, 0.25, 0.25, 1.0]]));
    }
}
