//! Showing what triggers did: their sounds and camera shakes.

use bevy::audio::{AudioPlayer, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::content::UNITS_PER_METRE;
use crate::game::party::body::EYE_ABOVE_CENTRE;
use crate::game::world::triggers::TriggerEffect;
use crate::presentation::PresentationRng;
use crate::presentation::party_view::PartyCamera;
use rand::Rng;

/// The party camera shaking (Whizzardry `ShakeSystem`).
#[derive(Resource, Debug, Default)]
pub struct CameraShake {
    /// How long it lasts; none when the camera is still.
    timer: Option<Timer>,
    /// Metres.
    amplitude: f32,
    fade_out: bool,
}

pub fn play_trigger_effects(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut effects: MessageReader<TriggerEffect>,
    mut shake: ResMut<CameraShake>,
) {
    for effect in effects.read() {
        let sound = match effect {
            TriggerEffect::Sound { path, volume, at } => {
                Some((format!("retail://{path}"), *volume, *at))
            }
            TriggerEffect::FountainUsed { at } => {
                Some(("retail://data/sound/misc/fountain_magic.wav".to_owned(), 1.0, *at))
            }
            TriggerEffect::BellRang(number) => {
                Some((format!("retail://data/sound/ambients/mon2bell{number}.wav"), 1.0, None))
            }
            TriggerEffect::Gold(_) => {
                Some(("retail://data/sound/misc/chaching.wav".to_owned(), 1.0, None))
            }
            TriggerEffect::Shake { seconds, intensity, fade_out } => {
                // Retail offsets the view by up to 250 units at full
                // strength. Whizzardry's reconstruction clamps the authored
                // intensity so that it never matters, which looks like a
                // min/max slip; here intensity / 250 scales the offset.
                *shake = CameraShake {
                    timer: Some(Timer::from_seconds(*seconds, TimerMode::Once)),
                    amplitude: (intensity / 250.0).min(1.0) * 250.0 / UNITS_PER_METRE,
                    fade_out: *fade_out,
                };
                None
            }
            // Shown as notices.
            TriggerEffect::Hurt { member, amount } => {
                info!("{member} takes {amount} damage");
                None
            }
            TriggerEffect::Message(_)
            | TriggerEffect::NothingHappened
            | TriggerEffect::Locked
            | TriggerEffect::PartyTakes(_) => None,
        };
        if let Some((path, volume, at)) = sound {
            let settings = PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(volume))
                .with_spatial(at.is_some());
            commands.spawn((
                Name::new(format!("Sound {path}")),
                AudioPlayer::new(assets.load(path)),
                settings,
                Transform::from_translation(at.unwrap_or_default()),
            ));
        }
    }
}

/// Jitter the camera while a shake lasts, fading out if it does.
pub fn shake_camera(
    time: Res<Time>,
    mut shake: ResMut<CameraShake>,
    mut rng: ResMut<PresentationRng>,
    mut camera: Query<&mut Transform, With<PartyCamera>>,
) {
    let Ok(mut camera) = camera.single_mut() else { return };
    let rest = Vec3::Y * EYE_ABOVE_CENTRE;
    let (amplitude, fade_out) = (shake.amplitude, shake.fade_out);
    let Some(timer) = shake.timer.as_mut().filter(|t| !t.is_finished()) else {
        camera.translation = rest;
        return;
    };
    let strength = if fade_out { timer.fraction_remaining() } else { 1.0 };
    let offset = Vec3::from_array(std::array::from_fn(|_| rng.0.random_range(-1.0..1.0)));
    camera.translation = rest + offset * amplitude * strength;
    timer.tick(time.delta());
}
