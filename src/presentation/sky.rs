//! The sky camera: it sits at the sky world's origin and turns with the
//! party camera, so the sky has no parallax (Whizzardry `world.cpp`). It
//! draws first and clears the frame; the party camera draws the level over
//! it.

use bevy::camera::visibility::RenderLayers;
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::content::level::LevelGeometry;
use crate::game::time::GameClock;
use crate::game::world::level::Level;
use crate::presentation::billboards::FaceCamera;
use crate::presentation::party_view::PartyCamera;

#[derive(Component, Debug)]
pub struct SkyCamera;

pub fn follow_with_sky_camera(
    mut commands: Commands,
    party: Query<(Entity, &Projection), (With<PartyCamera>, Without<SkyCamera>)>,
    mut transforms: ParamSet<(
        bevy::transform::helper::TransformHelper,
        Query<(&mut Transform, &mut Projection), With<SkyCamera>>,
    )>,
) {
    let Ok((party, party_projection)) = party.single() else { return };
    // Read the current local hierarchy before engine propagation, not last
    // frame's GlobalTransform. The sky then propagates in the same engine pass.
    let Ok(current) = transforms.p0().compute_global_transform(party) else { return };
    let rotation = current.rotation();
    let mut sky = transforms.p1();
    match sky.single_mut() {
        Ok((mut transform, mut projection)) => {
            if transform.rotation != rotation {
                transform.rotation = rotation;
            }
            if let (Projection::Perspective(sky), Projection::Perspective(level)) =
                (projection.as_ref(), party_projection)
                && sky.fov != level.fov
                && let Projection::Perspective(sky) = projection.as_mut()
            {
                sky.fov = level.fov;
            }
        }
        Err(_) => {
            let mut projection = party_projection.clone();
            if let Projection::Perspective(perspective) = &mut projection {
                perspective.near = 0.1;
                perspective.far = 10_000.0;
            }
            commands.spawn((
                Name::new("Sky camera"),
                SkyCamera,
                Camera3d::default(),
                Camera { order: -1, ..default() },
                projection,
                Transform::from_rotation(rotation),
                RenderLayers::layer(SKY_LAYER),
            ));
        }
    }
}

/// Retail gives every `CloudsN` prop one shared material: brightness
/// `b = 1 - |t - noon| / noon`, alpha `0.75 b + 0.25`, and textures
/// scrolling along U at 0.002 per second.
#[derive(Component, Debug)]
pub struct Cloud;

pub fn shade_clouds(
    time: Res<Time<Virtual>>,
    clock: Res<GameClock>,
    clouds: Query<&MeshMaterial3d<StandardMaterial>, With<Cloud>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    const NOON_MS: f32 = 43_200_000.0;
    let t = (clock.ms % 86_400_000) as f32;
    let brightness = 1.0 - (t - NOON_MS).abs() / NOON_MS;
    let offset = (time.elapsed_secs() * 0.002).fract();
    for material in &clouds {
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.base_color =
                Color::linear_rgba(brightness, brightness, brightness, 0.75 * brightness + 0.25);
            material.uv_transform = Affine2::from_translation(Vec2::new(offset, 0.0));
        }
    }
}

/// A sky body's authored position, and which one it is.
#[derive(Component, Debug)]
pub struct Celestial {
    pub sun: bool,
    pub authored: Vec3,
}

/// Move the Sun by day and the Moon by night around the midpoint of their
/// authored positions (Whizzardry `AdvanceEnvironmentTime`); the body not
/// in the sky waits at the centre.
pub fn orbit_sun_and_moon(clock: Res<GameClock>, mut bodies: Query<(&Celestial, &mut Transform)>) {
    let (mut sun, mut moon) = (None, None);
    for (body, _) in &bodies {
        if body.sun { sun = Some(body.authored) } else { moon = Some(body.authored) }
    }
    let (Some(sun), Some(moon)) = (sun, moon) else { return };
    let centre = (sun + moon) / 2.0;
    let radius = sun.distance(moon) / 2.0;
    let t = (clock.ms % 86_400_000) as f32;
    let day = (18_000_000.0..=79_200_000.0).contains(&t) && t > 18_000_000.0;
    let degrees = if day {
        (t - 18_000_000.0) * 80.0 * 1.634e-8
    } else if t <= 18_000_000.0 {
        (t + 7_200_000.0) * 80.0 * 3.968e-8
    } else {
        (t - 79_200_000.0) * 80.0 * 3.968e-8
    };
    // A rotation about Z keeps its sense under the Z mirror.
    let orbit = centre + Quat::from_rotation_z((degrees - 40.0).to_radians()) * Vec3::Y * radius;
    for (body, mut transform) in &mut bodies {
        transform.translation = if body.sun == day { orbit } else { centre };
    }
}

/// Optional sky content belongs to presentation, including its loading lifetime.
#[derive(Component, Debug)]
pub struct LevelSky(pub Option<Handle<LevelGeometry>>);

/// Request an active level's sky as it becomes active; [`build_sky`] waits
/// for it.
pub fn request_skies(
    add: On<Add<Level>>,
    mut commands: Commands,
    levels: Query<&Level>,
    assets: Res<AssetServer>,
) {
    let Ok(level) = levels.get(add.entity) else { return };
    let sky = crate::content::levels::LevelInfo::for_path(&level.path)
        .and_then(crate::content::levels::LevelInfo::sky_path)
        .map(|path| assets.load(format!("retail://{path}")));
    commands.entity(add.entity).insert(LevelSky(sky));
}

/// The level's sky has been built (or it has none).
#[derive(Component, Debug)]
pub struct SkyBuilt;

/// The render layer sky worlds are drawn on, by their own camera.
pub const SKY_LAYER: usize = 1;

/// Build the level's sky world under its root, on the sky's render layer,
/// once it has loaded. Cosmetic: the level is played without waiting.
pub fn build_sky(
    mut commands: Commands,
    levels: Query<(Entity, &LevelSky), Without<SkyBuilt>>,
    geometry: Res<Assets<LevelGeometry>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (root, level) in &levels {
        let Some(sky) = &level.0 else {
            commands.entity(root).insert(SkyBuilt);
            continue;
        };
        let Some(sky) = geometry.get(sky) else { continue };
        commands.entity(root).insert(SkyBuilt);
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::transform::TransformSystems;

    #[test]
    fn sky_uses_current_parented_camera_pose_in_the_same_propagation_pass() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .add_systems(PostUpdate, follow_with_sky_camera.before(TransformSystems::Propagate));
        let parent = app.world_mut().spawn(Transform::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                PartyCamera,
                Transform::default(),
                Projection::Perspective(default()),
                ChildOf(parent),
            ))
            .id();
        let sky = app
            .world_mut()
            .spawn((SkyCamera, Transform::default(), Projection::Perspective(default())))
            .id();
        app.update();
        for angle in [0.7, -1.3] {
            let yaw = Quat::from_rotation_y(angle);
            let pitch = Quat::from_rotation_x(0.2);
            app.world_mut().get_mut::<Transform>(parent).unwrap().rotation = yaw;
            app.world_mut().get_mut::<Transform>(camera).unwrap().rotation = pitch;
            // Neither cached global pose has propagated this new orientation.
            app.update();
            assert!(
                app.world()
                    .get::<GlobalTransform>(sky)
                    .unwrap()
                    .rotation()
                    .abs_diff_eq(yaw * pitch, 1e-5)
            );
        }
    }
}
