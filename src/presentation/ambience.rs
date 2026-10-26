//! Sounds places make (Whizzardry `W8AmbientSound`): looping sounds play
//! while the party is within their radius; one-shot sounds play now and
//! then, two times in three on coming into range. Loudness is full within
//! 40% of the radius and fades to nothing at its edge. Retail positions them
//! in stereo; here only their loudness follows the distance.

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;
use rand::Rng;

use crate::presentation::PresentationRng;

use crate::content::level::{AmbientSoundModel, LevelDefinition};
use crate::game::world::level::Level;
use crate::presentation::party_view::PartyCamera;

/// The inner part of the radius heard at full loudness.
const FULL_WITHIN: f32 = 0.4;

/// One of the level's ambient sounds, by its index in the level.
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
    levels: Query<(Entity, &Level), Without<AmbiencePlaced>>,
    geometry: Res<Assets<LevelDefinition>>,
) {
    for (root, level) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
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
    levels: Query<&Level>,
    geometry: Res<Assets<LevelDefinition>>,
    mut emitters: Query<(Entity, &mut AmbientEmitter, &Transform, Option<&Children>)>,
    mut sinks: Query<&mut AudioSink>,
    mut rng: ResMut<PresentationRng>,
) {
    let (Ok(listener), Some(level)) =
        (listener.single(), levels.iter().find_map(|l| geometry.get(&l.geometry)))
    else {
        return;
    };
    let ear = listener.translation();
    for (entity, mut emitter, at, children) in &mut emitters {
        let Some(sound) = level.ambient_sounds.get(emitter.index) else { continue };
        // The emitter owns only its sound. The child exists while loading or
        // playing; DESPAWN playback removes it when the one-shot finishes.
        let playing = children.and_then(|children| children.first()).copied();
        let distance = ear.distance(at.translation);
        let inside = distance < sound.radius;
        let entering = inside && !emitter.inside;
        emitter.inside = inside;
        if !inside {
            if let Some(playing) = playing {
                commands.entity(playing).despawn();
            }
            continue;
        }
        let loudness = gain(sound, distance);
        if sound.looping {
            match playing {
                Some(playing) => {
                    if let Ok(mut sink) = sinks.get_mut(playing) {
                        sink.set_volume(Volume::Linear(sound.volume.end() * loudness));
                    }
                }
                None => {
                    debug!("ambience: {} at {loudness:.2}", sound.path);
                    commands.spawn((
                        AudioPlayer::new(assets.load(format!("retail://{}", sound.path))),
                        PlaybackSettings::LOOP
                            .with_volume(Volume::Linear(sound.volume.end() * loudness)),
                        ChildOf(entity),
                    ));
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
        if due && playing.is_none() {
            let volume = rng.0.random_range(sound.volume.clone()) * loudness;
            commands.spawn((
                AudioPlayer::new(assets.load(format!("retail://{}", sound.path))),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
                ChildOf(entity),
            ));
            emitter.wait =
                Timer::from_seconds(rng.0.random_range(sound.interval.clone()), TimerMode::Once);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_sound_is_not_started_again_before_it_finishes() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<LevelDefinition>()
            .init_asset::<bevy::audio::AudioSource>()
            .init_resource::<PresentationRng>()
            .add_systems(Update, play_ambient_sounds);
        let geometry =
            app.world_mut().resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition {
                ambient_sounds: vec![AmbientSoundModel {
                    path: "pending.wav".into(),
                    position: Vec3::ZERO,
                    radius: 10.0,
                    looping: false,
                    volume: 1.0..=1.0,
                    interval: 0.0..=0.0,
                }],
                ..default()
            });
        app.world_mut().spawn(Level {
            path: "test".into(),
            geometry,
            navmeshes: vec![],
            script: default(),
            messages: None,
        });
        let listener = app.world_mut().spawn((PartyCamera, GlobalTransform::default())).id();
        let emitter = app
            .world_mut()
            .spawn((
                AmbientEmitter { index: 0, inside: true, wait: Timer::default() },
                Transform::default(),
            ))
            .id();

        // No audio backend is installed: the player remains pending without a sink.
        app.update();
        let sound = app.world().get::<Children>(emitter).unwrap()[0];
        app.update();
        assert!(app.world().get::<AudioSink>(sound).is_none());
        assert_eq!(&**app.world().get::<Children>(emitter).unwrap(), &[sound]);

        app.world_mut().despawn(sound);
        app.update();
        let next = app.world().get::<Children>(emitter).unwrap()[0];
        assert_ne!(sound, next, "finished playback permits the next sound");

        app.world_mut()
            .entity_mut(listener)
            .insert(GlobalTransform::from_translation(Vec3::X * 20.0));
        app.update();
        assert!(app.world().get_entity(next).is_err(), "leaving cancels pending playback");
    }
}
