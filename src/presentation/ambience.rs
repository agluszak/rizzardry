//! Sounds places make (Whizzardry `W8AmbientSound`): looping sounds play
//! while the party is within their radius; one-shot sounds play now and
//! then, two times in three on coming into range. Loudness is full within
//! 40% of the radius and fades to nothing at its edge. Retail positions them
//! in stereo; here only their loudness follows the distance.

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;
use rand::Rng;

use crate::presentation::PresentationRng;

use super::environment::LevelAppearance;
use crate::content::level::{AmbientSoundModel, LevelGeometry};
use crate::presentation::party_view::PartyCamera;

/// The inner part of the radius heard at full loudness.
const FULL_WITHIN: f32 = 0.4;

/// One of the level's ambient sounds, by its index in the owning level.
/// Playback lives on this entity; REMOVE completion leaves its timing intact.
#[derive(Component, Debug)]
pub struct AmbientEmitter {
    index: usize,
    inside: bool,
    /// Until it may play again.
    wait: Timer,
}

/// The level's ambient sounds have been placed.
#[derive(Component, Debug)]
pub struct AmbiencePlaced;

pub fn place_ambient_sounds(
    mut commands: Commands,
    levels: Query<(Entity, &LevelAppearance), Without<AmbiencePlaced>>,
    geometry: Res<Assets<LevelGeometry>>,
) {
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.0) else { continue };
        for (index, sound) in geometry.ambient_sounds.iter().enumerate() {
            commands.spawn((
                Name::new(format!("Ambience {}", sound.path)),
                AmbientEmitter { index, inside: false, wait: Timer::default() },
                Transform::from_translation(sound.position),
                ChildOf(root),
            ));
        }
        commands.entity(root).insert(AmbiencePlaced);
    }
}

fn gain(sound: &AmbientSoundModel, distance: f32) -> f32 {
    let full = sound.radius * FULL_WITHIN;
    if distance <= full { 1.0 } else { (1.0 - (distance - full) / (sound.radius - full)).max(0.0) }
}

pub fn play_ambient_sounds(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    listener: Query<&GlobalTransform, With<PartyCamera>>,
    levels: Query<(&LevelAppearance, Option<&crate::game::world::triggers::LevelState>)>,
    geometry: Res<Assets<LevelGeometry>>,
    mut emitters: Query<(
        Entity,
        &mut AmbientEmitter,
        &Transform,
        &ChildOf,
        Has<AudioPlayer>,
        Option<&mut AudioSink>,
        Option<&mut PlaybackSettings>,
    )>,
    master: Option<Res<bevy::audio::GlobalVolume>>,
    mut rng: ResMut<PresentationRng>,
) {
    let Ok(listener) = listener.single() else { return };
    let ear = listener.translation();
    for (entity, mut emitter, at, owner, playing, sink, settings) in &mut emitters {
        let Ok((appearance, state)) = levels.get(owner.parent()) else { continue };
        let Some(level) = geometry.get(&appearance.0) else { continue };
        let Some(sound) = level.ambient_sounds.get(emitter.index) else { continue };
        let distance = ear.distance(at.translation);
        let inside =
            distance < sound.radius && state.is_none_or(|s| s.ambient_sound_enabled(&sound.name));
        let entering = inside && !emitter.inside;
        emitter.inside = inside;
        if !inside {
            // Removing the player also cancels audio still waiting for its asset.
            if playing {
                commands.entity(entity).remove::<(AudioPlayer, AudioSink, PlaybackSettings)>();
            }
            continue;
        }
        let loudness = gain(sound, distance);
        if sound.looping {
            let volume = Volume::Linear(sound.volume.end() * loudness);
            if let Some(mut sink) = sink {
                sink.set_volume(volume * master.as_ref().map_or(Volume::Linear(1.), |v| v.volume));
            } else if let Some(mut settings) = settings {
                // The listener can move while decoding audio. Its first sample
                // must use the current gain, not the gain at the initial request.
                settings.volume = volume;
            }
            if !playing {
                debug!("ambience: {} at {loudness:.2}", sound.path);
                commands.entity(entity).insert((
                    AudioPlayer::new(assets.load(crate::content::paths::retail(&sound.path))),
                    PlaybackSettings::LOOP
                        .with_volume(Volume::Linear(sound.volume.end() * loudness)),
                ));
            }
            continue;
        }
        let due = if entering {
            rng.0.random_bool(2.0 / 3.0)
        } else {
            emitter.wait.tick(time.delta()).is_finished()
        };
        if entering {
            emitter.wait =
                Timer::from_seconds(rng.0.random_range(sound.interval.clone()), TimerMode::Once);
        }
        if due && !playing {
            let volume = rng.0.random_range(sound.volume.clone()) * loudness;
            commands.entity(entity).insert((
                AudioPlayer::new(assets.load(crate::content::paths::retail(&sound.path))),
                PlaybackSettings::REMOVE.with_volume(Volume::Linear(volume)),
            ));
            emitter.wait =
                Timer::from_seconds(rng.0.random_range(sound.interval.clone()), TimerMode::Once);
        }
    }
}
