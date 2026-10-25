//! Sounds places make (Whizzardry `W8AmbientSound`): looping sounds play
//! while the party is within their radius; one-shot sounds play now and
//! then, two times in three on coming into range. Loudness is full within
//! 40% of the radius and fades to nothing at its edge. Retail positions them
//! in stereo; here only their loudness follows the distance.

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;
use rand::Rng;

use crate::presentation::PresentationRng;

use crate::content::level::{AmbientSoundModel, LevelGeometry};
use crate::game::world::level::Level;
use crate::presentation::party_view::PartyCamera;

/// The inner part of the radius heard at full loudness.
const FULL_WITHIN: f32 = 0.4;

/// One of the level's ambient sounds, by its index in the level.
#[derive(Component, Debug)]
pub struct AmbientEmitter {
    index: usize,
    inside: bool,
    playing: Option<Entity>,
    /// Until it may play again.
    wait: Timer,
}

/// The level's ambient sounds have been placed.
#[derive(Component, Debug)]
pub struct AmbiencePlaced;

pub fn place_ambient_sounds(
    mut commands: Commands,
    levels: Query<(Entity, &Level), Without<AmbiencePlaced>>,
    geometry: Res<Assets<LevelGeometry>>,
) {
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        for (index, sound) in geometry.ambient_sounds.iter().enumerate() {
            commands.spawn((
                Name::new(format!("Ambience {}", sound.path)),
                AmbientEmitter { index, inside: false, playing: None, wait: Timer::default() },
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
    levels: Query<&Level>,
    geometry: Res<Assets<LevelGeometry>>,
    mut emitters: Query<(Entity, &mut AmbientEmitter, &Transform)>,
    mut sinks: Query<&mut AudioSink>,
    mut rng: ResMut<PresentationRng>,
) {
    let (Ok(listener), Some(level)) =
        (listener.single(), levels.iter().find_map(|l| geometry.get(&l.geometry)))
    else {
        return;
    };
    let ear = listener.translation();
    for (entity, mut emitter, at) in &mut emitters {
        let Some(sound) = level.ambient_sounds.get(emitter.index) else { continue };
        let distance = ear.distance(at.translation);
        let inside = distance < sound.radius;
        let entering = inside && !emitter.inside;
        emitter.inside = inside;
        // A one-shot that finished despawned itself.
        if emitter.playing.is_some_and(|p| sinks.get(p).is_err() && !entering) && !sound.looping {
            emitter.playing = None;
        }
        if !inside {
            if let Some(playing) = emitter.playing.take() {
                commands.entity(playing).despawn();
            }
            continue;
        }
        let loudness = gain(sound, distance);
        if sound.looping {
            match emitter.playing {
                Some(playing) => {
                    if let Ok(mut sink) = sinks.get_mut(playing) {
                        sink.set_volume(Volume::Linear(sound.volume.end() * loudness));
                    }
                }
                None => {
                    debug!("ambience: {} at {loudness:.2}", sound.path);
                    let playing = commands
                        .spawn((
                            AudioPlayer::new(assets.load(format!("retail://{}", sound.path))),
                            PlaybackSettings::LOOP
                                .with_volume(Volume::Linear(sound.volume.end() * loudness)),
                            ChildOf(entity),
                        ))
                        .id();
                    emitter.playing = Some(playing);
                }
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
        if due && emitter.playing.is_none() {
            let volume = rng.0.random_range(sound.volume.clone()) * loudness;
            let playing = commands
                .spawn((
                    AudioPlayer::new(assets.load(format!("retail://{}", sound.path))),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
                    ChildOf(entity),
                ))
                .id();
            emitter.playing = Some(playing);
            emitter.wait =
                Timer::from_seconds(rng.0.random_range(sound.interval.clone()), TimerMode::Once);
        }
    }
}
