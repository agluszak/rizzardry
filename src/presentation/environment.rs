//! Fog, clear colour and sky light from the level environment and the time
//! of day.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::time::GameClock;
use crate::game::world::level::{Level, PropPart};
use crate::game::world::props::Prop;
use crate::presentation::billboards::FaceCamera;
use crate::presentation::party_view::PartyCamera;

/// Presentation owns the optional render payload for an active level.
#[derive(Component, Debug)]
pub struct LevelAppearance(pub Handle<LevelGeometry>);

pub fn request_level_visuals(
    mut commands: Commands,
    levels: Query<(Entity, &Level), Without<LevelAppearance>>,
    assets: Res<AssetServer>,
) {
    for (entity, level) in &levels {
        commands.entity(entity).insert((
            LevelAppearance(assets.load(format!("retail://{}", level.path))),
            Visibility::default(),
        ));
    }
}

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
    levels: Query<&LevelAppearance>,
    geometry: Res<Assets<LevelGeometry>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clear: ResMut<ClearColor>,
    mut camera: Query<(Entity, &mut Projection), With<PartyCamera>>,
    mut commands: Commands,
) {
    let Some(level) = levels.iter().next() else { return };
    let Some(level_geometry) = geometry.get(&level.0) else { return };
    let Ok((camera, mut projection)) = camera.single_mut() else { return };
    let phase = clock.phase();
    let key = (level.0.id(), phase);
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
    levels: Query<(Entity, &LevelAppearance, Option<&Children>), Without<LevelVisuals>>,
    geometry: Res<Assets<LevelGeometry>>,
    props: Query<(&Prop, &Children)>,
    parts: Query<&PropPart>,
) {
    for (root, level, children) in &levels {
        let Some(geometry) = geometry.get(&level.0) else { continue };
        commands.entity(root).insert((LevelVisuals, Visibility::default()));
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
            commands.entity(child).insert(Visibility::default());
            for &entity in prop_parts {
                let Some(part) = parts.get(entity).ok().and_then(|p| model.parts.get(p.part))
                else {
                    continue;
                };
                commands.entity(entity).insert(Visibility::default());
                // Gameplay owns the part's authored pose, including rotation.
                // Billboard only its visual child, cancelling that rotation.
                let visual = if model.face_camera {
                    commands
                        .spawn((
                            FaceCamera,
                            Transform::default(),
                            Visibility::default(),
                            ChildOf(entity),
                        ))
                        .id()
                } else {
                    entity
                };
                for mesh in &part.meshes {
                    commands.spawn((
                        Mesh3d(mesh.mesh.clone()),
                        MeshMaterial3d(mesh.material.clone()),
                        ChildOf(visual),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::level::{PropVisual, PropVisualPart};
    use crate::game::world::props::PropMotion;

    #[test]
    fn billboards_follow_the_camera_without_changing_gameplay_poses() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .init_resource::<Assets<LevelGeometry>>()
            .add_systems(Update, attach_level_visuals)
            .add_systems(Update, crate::presentation::billboards::face_camera);
        let authored = Transform::from_xyz(3.0, 4.0, 5.0).with_rotation(Quat::from_rotation_y(0.7));
        let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            props: vec![PropVisual {
                name: "sprite".into(),
                face_camera: true,
                parts: vec![PropVisualPart { transform: authored, meshes: vec![] }],
            }],
            ..default()
        });
        let root = app.world_mut().spawn((LevelAppearance(geometry), Transform::default())).id();
        let prop = app
            .world_mut()
            .spawn((
                Prop {
                    definition: 0,
                    centre: authored.translation,
                    door: false,
                    frames_per_second: 1.0,
                    last_frame: 0,
                    wraps: false,
                    slots: vec![],
                },
                PropMotion::default(),
                Transform::default(),
                ChildOf(root),
            ))
            .id();
        let part = app
            .world_mut()
            .spawn((PropPart { part: 0, keys: vec![] }, authored, ChildOf(prop)))
            .id();
        let camera_rotation = Quat::from_euler(EulerRot::YXZ, -0.4, 0.2, 0.0);
        app.world_mut().spawn((PartyCamera, Transform::from_rotation(camera_rotation)));
        // Attachment and presentation can run independently on the first frame.
        app.update();
        app.update();
        let visual = app
            .world_mut()
            .query_filtered::<Entity, With<FaceCamera>>()
            .single(app.world())
            .unwrap();
        assert_eq!(*app.world().get::<Transform>(part).unwrap(), authored);
        assert!(
            app.world()
                .get::<GlobalTransform>(visual)
                .unwrap()
                .rotation()
                .abs_diff_eq(camera_rotation, 1e-5)
        );
        let moved = authored.with_rotation(Quat::from_rotation_x(1.2));
        *app.world_mut().get_mut::<Transform>(part).unwrap() = moved;
        app.update();
        assert_eq!(*app.world().get::<Transform>(part).unwrap(), moved);
        let shown = app.world().get::<GlobalTransform>(visual).unwrap();
        assert!(shown.rotation().abs_diff_eq(camera_rotation, 1e-5));
        assert!(shown.translation().abs_diff_eq(moved.translation, 1e-5));
        app.world_mut().despawn(root);
        assert!(app.world().get_entity(visual).is_err());
    }
}
