//! Fog, clear colour and sky light from the level environment and the time
//! of day.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::time::GameClock;
use bevy::asset::{LoadState, RecursiveDependencyLoadState};

use crate::game::world::level::{Destination, Level, LevelEntities, PresentationReadiness};
use crate::game::world::props::Prop;
use crate::game::world::props::PropPart;
use crate::presentation::billboards::FaceCamera;
use crate::presentation::party_view::PartyCamera;

/// Presentation owns the optional render payload for an active level.
#[derive(Component, Debug)]
pub struct LevelAppearance(pub Handle<LevelGeometry>);

/// On an entry being prepared: its appearance, loading before it commits.
#[derive(Component, Debug)]
pub struct PreparedAppearance(Handle<LevelGeometry>);

/// Load an entry's appearance as part of its preparation, holding its
/// commit until the appearance and what it uses have loaded. If the level
/// itself cannot be shown the entry fails; a missing texture or model only
/// shows as missing, as before.
pub fn prepare_appearance(
    mut commands: Commands,
    entries: Query<(Entity, &Destination, Option<&PreparedAppearance>)>,
    assets: Res<AssetServer>,
) {
    for (entity, destination, prepared) in &entries {
        let Some(PreparedAppearance(handle)) = prepared else {
            let handle = assets.load(crate::content::paths::retail(&destination.level.path));
            commands
                .entity(entity)
                .insert((PreparedAppearance(handle), PresentationReadiness::Waiting));
            continue;
        };
        let readiness = match assets.get_load_states(handle) {
            Some((LoadState::Failed(error), ..)) => {
                Some(PresentationReadiness::Failed(format!("its appearance failed: {error}")))
            }
            Some((
                LoadState::Loaded,
                _,
                RecursiveDependencyLoadState::Loaded | RecursiveDependencyLoadState::Failed(_),
            )) => None,
            _ => Some(PresentationReadiness::Waiting),
        };
        match readiness {
            Some(PresentationReadiness::Waiting) => {}
            Some(failed) => {
                commands.entity(entity).insert(failed);
            }
            None => {
                commands.entity(entity).remove::<PresentationReadiness>();
            }
        }
    }
}

/// Request an active level's render payload as it becomes active: the
/// appearance its entry prepared, or (for a level entered without a
/// renderer's preparation) loading it now. Building it waits for the asset
/// ([`attach_level_visuals`]).
pub fn request_level_visuals(
    add: On<Add<Level>>,
    mut commands: Commands,
    levels: Query<(&Level, Option<&PreparedAppearance>)>,
    assets: Res<AssetServer>,
) {
    let Ok((level, prepared)) = levels.get(add.entity) else { return };
    let handle = prepared.map_or_else(
        || assets.load(crate::content::paths::retail(&level.path)),
        |prepared| prepared.0.clone(),
    );
    commands
        .entity(add.entity)
        .remove::<PreparedAppearance>()
        .insert((LevelAppearance(handle), Visibility::default()));
}

/// The render inputs applied to this camera. A replacement camera needs its
/// own fog setup even when the level and daylight phase have not changed.
#[derive(Component)]
pub struct AppliedEnvironment(AssetId<LevelGeometry>, u8);

/// Apply the environment when the level or the time-of-day phase changes:
/// linear fog toward the phase's fog colour (which is also the clear
/// colour), the far clip at the view distance, and vertex colours relit
/// with the phase's sky light.
pub fn apply_environment(
    clock: Res<GameClock>,
    levels: Query<Ref<LevelAppearance>>,
    geometry: Res<Assets<LevelGeometry>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clear: ResMut<ClearColor>,
    mut camera: Query<(Entity, &mut Projection, Option<&AppliedEnvironment>), With<PartyCamera>>,
    mut commands: Commands,
) {
    let Some(level) = levels.iter().next() else { return };
    let Some(level_geometry) = geometry.get(&level.0) else { return };
    let Ok((camera, mut projection, applied)) = camera.single_mut() else { return };
    let phase = clock.phase();
    if applied.is_some_and(|applied| applied.0 == level.0.id() && applied.1 == phase)
        && !level.is_changed()
        && !geometry.is_changed()
    {
        return;
    }
    let Some(environment) = &level_geometry.environment else { return };

    let fog = environment.fog_colours[phase as usize];
    clear.0 = Color::srgb(fog.x, fog.y, fog.z);
    let far = environment.view_distance;
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.far = far;
    }
    commands.entity(camera).insert((
        AppliedEnvironment(level.0.id(), phase),
        DistanceFog {
            color: Color::srgb(fog.x, fog.y, fog.z),
            falloff: FogFalloff::Linear {
                start: far * environment.fog_range.0,
                end: far * environment.fog_range.1,
            },
            ..default()
        },
    ));

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
    levels: Query<(Entity, &LevelAppearance, Option<&LevelEntities>), Without<LevelVisuals>>,
    geometry: Res<Assets<LevelGeometry>>,
    props: Query<(&Prop, &Children)>,
    parts: Query<&PropPart>,
) {
    for (root, level, children) in &levels {
        let Some(geometry) = geometry.get(&level.0) else { continue };
        commands.entity(root).insert((LevelVisuals, Visibility::default()));
        for part in &geometry.parts {
            commands.spawn((Mesh3d(part.mesh.clone()), part.surface.render(), ChildOf(root)));
        }
        for &child in children.into_iter().flat_map(LevelEntities::entities) {
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
                        mesh.surface.render(),
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
    fn replacing_a_camera_and_reloading_environment_reapply_fog_at_the_same_phase() {
        use crate::content::level::LevelEnvironment;
        let mut app = App::new();
        app.init_resource::<Assets<LevelGeometry>>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ClearColor>()
            .init_resource::<GameClock>()
            .add_systems(Update, apply_environment);
        let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            environment: Some(LevelEnvironment {
                fog_range: (0.2, 1.0),
                view_distance: 80.0,
                intensity: 1.0,
                fog_colours: vec![Vec3::ONE; 256],
                ambient_colours: vec![Vec3::ONE; 256],
            }),
            ..default()
        });
        app.world_mut().spawn(LevelAppearance(geometry.clone()));
        let camera = app.world_mut().spawn((PartyCamera, Projection::default())).id();
        app.update();
        assert!(app.world().get::<DistanceFog>(camera).is_some());
        app.world_mut().despawn(camera);
        let camera = app.world_mut().spawn((PartyCamera, Projection::default())).id();
        app.update();
        assert!(app.world().get::<DistanceFog>(camera).is_some());
        let far = |app: &App| match app.world().get::<Projection>(camera).unwrap() {
            Projection::Perspective(p) => p.far,
            _ => panic!("expected perspective camera"),
        };
        assert_eq!(far(&app), 80.0);
        app.world_mut()
            .resource_mut::<Assets<LevelGeometry>>()
            .get_mut(&geometry)
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .view_distance = 120.0;
        app.update();
        assert_eq!(far(&app), 120.0);
    }

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
                    frames_per_second: 1.0,
                    last_frame: 0,
                    wraps: false,
                    slots: vec![],
                },
                PropMotion::default(),
                Transform::default(),
                crate::game::world::level::LevelOwned(root),
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
