//! The sky camera: it sits at the sky world's origin and turns with the
//! party camera, so the sky has no parallax (Whizzardry `world.cpp`). It
//! draws first and clears the frame; the party camera draws the level over
//! it.

use bevy::camera::visibility::RenderLayers;
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::game::party::body::PartyCamera;
use crate::game::time::GameClock;
use crate::game::world::level::SKY_LAYER;

#[derive(Component, Debug)]
pub struct SkyCamera;

pub fn follow_with_sky_camera(
    mut commands: Commands,
    party: Query<(&GlobalTransform, &Projection), (With<PartyCamera>, Without<SkyCamera>)>,
    mut sky: Query<(&mut Transform, &mut Projection), With<SkyCamera>>,
) {
    let Ok((party, party_projection)) = party.single() else { return };
    let rotation = party.rotation();
    match sky.single_mut() {
        Ok((mut transform, mut projection)) => {
            transform.rotation = rotation;
            if let (Projection::Perspective(sky), Projection::Perspective(level)) =
                (projection.as_mut(), party_projection)
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
