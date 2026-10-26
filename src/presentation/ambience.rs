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
    levels: Query<&LevelAppearance>,
    geometry: Res<Assets<LevelGeometry>>,
    mut emitters: Query<(
        Entity,
        &mut AmbientEmitter,
        &Transform,
        &ChildOf,
        Has<AudioPlayer>,
        Option<&mut AudioSink>,
    )>,
    mut rng: ResMut<PresentationRng>,
) {
    let Ok(listener) = listener.single() else { return };
    let ear = listener.translation();
    for (entity, mut emitter, at, owner, playing, sink) in &mut emitters {
        let Some(level) = levels.get(owner.parent()).ok().and_then(|l| geometry.get(&l.0)) else {
            continue;
        };
        let Some(sound) = level.ambient_sounds.get(emitter.index) else { continue };
        let distance = ear.distance(at.translation);
        let inside = distance < sound.radius;
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
            if let Some(mut sink) = sink {
                sink.set_volume(Volume::Linear(sound.volume.end() * loudness));
            }
            if !playing {
                debug!("ambience: {} at {loudness:.2}", sound.path);
                commands.entity(entity).insert((
                    AudioPlayer::new(assets.load(format!("retail://{}", sound.path))),
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
                AudioPlayer::new(assets.load(format!("retail://{}", sound.path))),
                PlaybackSettings::REMOVE.with_volume(Volume::Linear(volume)),
            ));
            emitter.wait =
                Timer::from_seconds(rng.0.random_range(sound.interval.clone()), TimerMode::Once);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::io::{AssetSourceBuilder, memory::MemoryAssetReader};
    use bevy::audio::PlaybackMode;

    fn app(looping: bool) -> (App, Entity, Entity, Entity) {
        let mut app = App::new();
        app.register_asset_source(
            "retail",
            AssetSourceBuilder::new(|| Box::new(MemoryAssetReader::default())),
        );
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<AudioSource>()
            .init_asset::<LevelGeometry>()
            .init_resource::<PresentationRng>()
            .add_systems(Update, (place_ambient_sounds, play_ambient_sounds).chain());
        let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            ambient_sounds: vec![AmbientSoundModel {
                path: "test.wav".into(),
                position: Vec3::ZERO,
                radius: 10.0,
                looping,
                volume: 0.5..=0.5,
                interval: 0.0..=0.0,
            }],
            ..default()
        });
        let level = app.world_mut().spawn(LevelAppearance(geometry)).id();
        let listener = app.world_mut().spawn((PartyCamera, GlobalTransform::default())).id();
        app.update();
        app.update(); // A one-shot's zero interval is due regardless of its entry roll.
        let emitter = app
            .world_mut()
            .query_filtered::<Entity, With<AmbientEmitter>>()
            .single(app.world())
            .unwrap();
        (app, level, listener, emitter)
    }

    #[test]
    fn pending_audio_is_owned_once_and_cancelled_on_exit_or_travel() {
        for looping in [false, true] {
            let (mut app, level, listener, emitter) = app(looping);
            assert!(app.world().get::<AudioPlayer>(emitter).is_some());
            assert!(app.world().get::<AudioSink>(emitter).is_none());
            // Observe replacement, even if it uses the same asset handle.
            app.world_mut().clear_trackers();
            for _ in 0..3 {
                app.update();
            }
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, Changed<AudioPlayer>>()
                    .iter(app.world())
                    .count(),
                0
            );
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, With<AudioPlayer>>()
                    .iter(app.world())
                    .count(),
                1
            );
            app.world_mut()
                .entity_mut(listener)
                .insert(GlobalTransform::from_translation(Vec3::X * 20.0));
            app.update();
            assert!(app.world().get::<AudioPlayer>(emitter).is_none());
            assert!(app.world().get::<PlaybackSettings>(emitter).is_none());
            assert!(app.world().get::<AmbientEmitter>(emitter).is_some());
            app.world_mut().entity_mut(listener).insert(GlobalTransform::default());
            app.update();
            app.update();
            assert!(app.world().get::<AudioPlayer>(emitter).is_some());
            app.world_mut().despawn(level);
            assert!(app.world().get_entity(emitter).is_err());
        }
    }

    #[test]
    fn one_shot_completion_keeps_emitter_available_for_next_play() {
        let (mut app, _, _, emitter) = app(false);
        assert!(matches!(
            app.world().get::<PlaybackSettings>(emitter).unwrap().mode,
            PlaybackMode::Remove
        ));
        // The audio backend removes playback components when the sink is empty.
        // No hardware/output device is needed to exercise the next scheduling step.
        app.world_mut().entity_mut(emitter).remove::<(AudioPlayer, AudioSink, PlaybackSettings)>();
        app.update();
        assert!(app.world().get::<AmbientEmitter>(emitter).is_some());
        assert!(app.world().get::<AudioPlayer>(emitter).is_some());
    }

    #[test]
    fn emitters_read_their_own_levels_sound_definitions() {
        let (mut app, _, _, _) = app(true);
        let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            ambient_sounds: vec![AmbientSoundModel {
                path: "distant.wav".into(),
                position: Vec3::X * 5.0,
                radius: 1.0,
                looping: true,
                volume: 1.0..=1.0,
                interval: 0.0..=0.0,
            }],
            ..default()
        });
        let level = app.world_mut().spawn(LevelAppearance(geometry)).id();
        app.update();
        let emitter = app.world().get::<Children>(level).unwrap()[0];
        assert!(app.world().get::<AmbientEmitter>(emitter).is_some());
        assert!(app.world().get::<AudioPlayer>(emitter).is_none());
    }
}
