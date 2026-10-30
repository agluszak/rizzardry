//! Fog, clear colour and sky light from the level environment and the time
//! of day.

use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::time::GameClock;
use bevy::asset::LoadState;

use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::body::PartyBody;
use crate::game::world::level::{Destination, Level, LevelEntities};
use crate::game::world::props::Prop;
use crate::game::world::props::PropPart;
use crate::presentation::billboards::FaceCamera;
use crate::presentation::party_view::PartyCamera;

/// The party's torch, as every lit view reads it: the Light spell's
/// [`spell_light`] at the party's eye.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PartyTorch<'w, 's> {
    party: Query<
        'w,
        's,
        (&'static avian3d::prelude::Position, &'static PartyEffects),
        With<PartyBody>,
    >,
}

impl PartyTorch<'_, '_> {
    pub fn light(&self) -> Option<crate::content::level::PointLightSource> {
        let (at, effects) = self.party.single().ok()?;
        spell_light(PartyBody::eye(at), effects)
    }
}

/// The sky's light on a level now, by the time of day's phase.
pub(super) fn sky_now(level: &LevelGeometry, clock: &GameClock) -> Vec3 {
    level.environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(clock.phase()))
}

/// Retail camera light: warm, 4000 retail units, twice adjusted power plus two.
fn spell_light(
    eye: Vec3,
    effects: &PartyEffects,
) -> Option<crate::content::level::PointLightSource> {
    let effect = effects.0.get(crate::content::spells::SpellId::LIGHT)?;
    let amount = (effect.adjusted(u32::from(effect.power)) as u8).wrapping_mul(2);
    Some(crate::content::level::PointLightSource {
        name: String::new(),
        runtime: true,
        enabled: true,
        position: eye,
        colour: Vec3::new(1.0, 0.85, 0.39) * (f32::from(amount) + 2.0),
        range: 4000.0 / crate::content::UNITS_PER_METRE,
    })
}

/// Current fixed lighting sampled for actor materials. Meshes own no
/// rest-pose point-light term; this calculation also needs no signed inverse.
pub(super) fn actor_light(
    lighting: &crate::content::level::LevelLighting,
    state: Option<&crate::game::world::triggers::LightActivation>,
    at: Vec3,
) -> Vec3 {
    lighting.offset
        + lighting
            .lights
            .iter()
            .filter(|light| {
                state.map_or(light.enabled, |state| state.light_enabled(&light.name, light.enabled))
            })
            .map(|light| light.at(at))
            .sum::<Vec3>()
}

/// Presentation owns the optional render payload for an active level.
#[derive(Component, Debug)]
pub struct LevelAppearance(pub Handle<LevelGeometry>);

/// On an entry being prepared: its appearance, loading alongside it.
#[derive(Component, Debug)]
pub struct PreparedAppearance(Handle<LevelGeometry>);

/// Start loading an entry's appearance while gameplay prepares it, so it
/// is likely ready when the level commits. It never holds or fails the
/// entry: a level whose looks are late shows them when they arrive, and
/// one whose looks fail plays on unseen ([`attach_level_visuals`]).
pub fn prepare_appearance(
    mut commands: Commands,
    entries: Query<(Entity, &Destination), Without<PreparedAppearance>>,
    assets: Res<AssetServer>,
) {
    for (entity, destination) in &entries {
        let handle = assets.load(crate::content::paths::retail(&destination.level.path));
        commands.entity(entity).insert(PreparedAppearance(handle));
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
    commands.entity(camera).insert(AppliedEnvironment(level.0.id(), phase));
    let Some(environment) = &level_geometry.environment else {
        commands.entity(camera).remove::<DistanceFog>();
        return;
    };

    let fog = environment.fog_colours[phase as usize];
    clear.0 = Color::srgb(fog.x, fog.y, fog.z);
    let far = environment.view_distance;
    if let Some(perspective) = super::water_view::perspective_mut(&mut projection) {
        perspective.far = far;
    }
    if environment.fog_enabled {
        commands.entity(camera).insert(DistanceFog {
            color: Color::srgb(fog.x, fog.y, fog.z),
            falloff: FogFalloff::Linear {
                start: far * environment.fog_range.0,
                end: far * environment.fog_range.1,
            },
            ..default()
        });
    } else {
        commands.entity(camera).remove::<DistanceFog>();
    }

    let sky = environment.sky_light(phase);
    for part in level_geometry.parts.iter().chain(
        level_geometry.props.iter().flat_map(|prop| &prop.parts).flat_map(|part| &part.meshes),
    ) {
        if let Some(mut mesh) = meshes.get_mut(&part.mesh) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, part.lighting.colours(sky));
        }
    }
}

/// The level's meshes have been attached, or its appearance failed and
/// none will be.
#[derive(Component, Debug)]
pub struct LevelVisuals;

/// Give an active level its looks: the static geometry under its root, each
/// prop part's meshes under the part, and sprites turning to the view.
pub fn attach_level_visuals(
    mut commands: Commands,
    levels: Query<(Entity, &LevelAppearance, Option<&LevelEntities>), Without<LevelVisuals>>,
    geometry: Res<Assets<LevelGeometry>>,
    source_materials: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<super::lighting::SceneMaterial>>,
    lights: Res<super::lighting::SceneLights>,
    assets: Res<AssetServer>,
    props: Query<(&Prop, &Children)>,
    parts: Query<&PropPart>,
) {
    for (root, level, children) in &levels {
        let Some(geometry) = geometry.get(&level.0) else {
            if let Some(LoadState::Failed(error)) = assets.get_load_state(&level.0) {
                error!("the level's appearance failed to load; it plays unseen: {error}");
                commands.entity(root).insert(LevelVisuals);
            }
            continue;
        };
        // Do not mark a partially attached scene complete while material
        // definitions are still arriving; retry attachment as one operation.
        if geometry
            .parts
            .iter()
            .chain(geometry.props.iter().flat_map(|prop| &prop.parts).flat_map(|part| &part.meshes))
            .any(|part| source_materials.get(&part.surface.material).is_none())
        {
            continue;
        }
        commands.entity(root).insert((LevelVisuals, Visibility::default()));
        let mut instances = bevy::platform::collections::HashMap::new();
        for part in &geometry.parts {
            if let Some(surface) = part.surface.dynamic_instance(
                true,
                &lights.0,
                &mut instances,
                &source_materials,
                &mut materials,
            ) {
                commands.spawn((Mesh3d(part.mesh.clone()), surface, ChildOf(root)));
            }
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
                    if let Some(surface) = mesh.surface.dynamic_instance(
                        false,
                        &lights.0,
                        &mut instances,
                        &source_materials,
                        &mut materials,
                    ) {
                        commands.spawn((Mesh3d(mesh.mesh.clone()), surface, ChildOf(visual)));
                    }
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
                fog_enabled: true,
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

        // Changing levels must remove fog even while daylight stays unchanged.
        let mut disabled = app
            .world()
            .resource::<Assets<LevelGeometry>>()
            .get(&geometry)
            .unwrap()
            .environment
            .clone()
            .unwrap();
        disabled.fog_enabled = false;
        let off = app
            .world_mut()
            .resource_mut::<Assets<LevelGeometry>>()
            .add(LevelGeometry { environment: Some(disabled), ..default() });
        let level = app
            .world_mut()
            .query_filtered::<Entity, With<LevelAppearance>>()
            .single(app.world())
            .unwrap();
        for (handle, enabled) in [(&off, false), (&geometry, true), (&off, false)] {
            app.world_mut().entity_mut(level).insert(LevelAppearance(handle.clone()));
            app.update();
            assert_eq!(app.world().get::<DistanceFog>(camera).is_some(), enabled);
            assert_eq!(far(&app), 120.0);
        }
        app.world_mut().despawn(camera);
        let camera = app.world_mut().spawn((PartyCamera, Projection::default())).id();
        app.update();
        assert!(app.world().get::<DistanceFog>(camera).is_none());

        app.world_mut().entity_mut(level).insert(LevelAppearance(geometry));
        app.update();
        assert!(app.world().get::<DistanceFog>(camera).is_some());
        let absent =
            app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry::default());
        app.world_mut().entity_mut(level).insert(LevelAppearance(absent));
        app.update();
        assert!(app.world().get::<DistanceFog>(camera).is_none());
    }

    /// A level whose looks cannot load plays on unseen; presentation gives
    /// up on them without touching the level.
    #[test]
    fn a_level_whose_appearance_fails_plays_on_unseen() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<LevelGeometry>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<super::super::lighting::SceneMaterial>>()
            .insert_resource(super::super::lighting::SceneLights(default()))
            .add_systems(Update, attach_level_visuals);
        // No loader takes this file, so its load fails.
        let handle = app.world().resource::<AssetServer>().load("missing.appearance");
        let root = app.world_mut().spawn(LevelAppearance(handle)).id();
        for _ in 0..100 {
            app.update();
            if app.world().get::<LevelVisuals>(root).is_some() {
                break;
            }
        }
        assert!(app.world().get::<LevelVisuals>(root).is_some());
        assert!(app.world().get::<LevelAppearance>(root).is_some());
    }

    #[test]
    fn billboards_follow_the_camera_without_changing_gameplay_poses() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, AssetPlugin::default()))
            .init_asset::<LevelGeometry>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<super::super::lighting::SceneMaterial>>()
            .insert_resource(super::super::lighting::SceneLights(default()))
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
                Prop { definition: 0 },
                PropMotion::default(),
                Transform::default(),
                crate::game::world::level::LevelOwned(root),
            ))
            .id();
        let part = app.world_mut().spawn((PropPart { part: 0 }, authored, ChildOf(prop))).id();
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
