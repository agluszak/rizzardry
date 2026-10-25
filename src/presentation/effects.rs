//! Showing what triggers did: their sounds and camera shakes.

use bevy::audio::{AudioPlayer, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::game::party::body::{EYE_ABOVE_CENTRE, PartyCamera};
use crate::game::world::triggers::TriggerEffect;
use crate::retail::space::UNITS_PER_METRE;

/// The party camera shaking (Whizzardry `ShakeSystem`).
#[derive(Resource, Debug, Default)]
pub struct CameraShake {
    remaining: f32,
    duration: f32,
    /// Metres.
    amplitude: f32,
    fade_out: bool,
    /// Presentation randomness, apart from gameplay's.
    noise: u32,
}

pub fn play_trigger_effects(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut effects: MessageReader<TriggerEffect>,
    mut shake: ResMut<CameraShake>,
) {
    for effect in effects.read() {
        match effect {
            TriggerEffect::Sound { path, volume, at } => {
                let settings = PlaybackSettings::DESPAWN
                    .with_volume(Volume::Linear(*volume))
                    .with_spatial(at.is_some());
                commands.spawn((
                    Name::new(format!("Sound {path}")),
                    AudioPlayer::new(assets.load(format!("retail://{path}"))),
                    settings,
                    Transform::from_translation(at.unwrap_or_default()),
                ));
            }
            TriggerEffect::Shake { seconds, intensity, fade_out } => {
                // Retail offsets the view by up to 250 units at full
                // strength. Whizzardry's reconstruction clamps the authored
                // intensity so that it never matters, which looks like a
                // min/max slip; here intensity / 250 scales the offset.
                *shake = CameraShake {
                    remaining: *seconds,
                    duration: *seconds,
                    amplitude: (intensity / 250.0).min(1.0) * 250.0 / UNITS_PER_METRE,
                    fade_out: *fade_out,
                    noise: shake.noise,
                };
            }
            // Shown as notices.
            TriggerEffect::Message(_) | TriggerEffect::NothingHappened | TriggerEffect::Locked => {}
        }
    }
}

/// Jitter the camera while a shake lasts, fading out if it does.
pub fn shake_camera(
    time: Res<Time>,
    mut shake: ResMut<CameraShake>,
    mut camera: Query<&mut Transform, With<PartyCamera>>,
) {
    let Ok(mut camera) = camera.single_mut() else { return };
    let rest = Vec3::Y * EYE_ABOVE_CENTRE;
    if shake.remaining <= 0.0 {
        camera.translation = rest;
        return;
    }
    let strength = if shake.fade_out { shake.remaining / shake.duration } else { 1.0 };
    let mut next = || {
        // xorshift32
        let mut x = shake.noise.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        shake.noise = x;
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let offset = Vec3::new(next(), next(), next());
    camera.translation = rest + offset * shake.amplitude * strength;
    shake.remaining -= time.delta_secs();
}
