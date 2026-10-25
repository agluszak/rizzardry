//! Level music from retail playlists (Whizzardry `music_playlist.cpp`): a
//! weighted song, faded in over five seconds; after each, sometimes a
//! silence; another level's playlist when the party travels, and the
//! camping playlist while it camps.

use bevy::asset::LoadState;
use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::content::levels::LevelInfo;
use crate::content::music::Playlist;
use crate::game::camp::Camp;
use crate::game::world::level::Level;
use crate::presentation::PresentationRng;
use rand::Rng;
use rand::seq::IndexedRandom;

const VOLUME: f32 = 0.6;
const FADE_IN_SECONDS: f32 = 5.0;
/// Levels without a playlist of their own play this one.
const FALLBACK: &str = "retail://data/music/adventure.mpl";
const CAMPING: &str = "retail://data/music/camping.mpl";

#[derive(Resource, Debug, Default)]
pub struct Music {
    /// The playlist wanted (the level's or camping's), and the one loaded
    /// for it, which is the fallback when a level has none.
    wanted: Option<String>,
    playlist: Option<Handle<Playlist>>,
    /// A silence between songs, while it lasts.
    silence: Option<Timer>,
    /// A song has played since the level's music began, so the next gap
    /// may be a silence.
    played: bool,
}

/// The song playing, fading in.
#[derive(Component, Debug)]
pub struct Song {
    fade_in: Timer,
}

pub fn play_music(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    levels: Query<&Level>,
    camp: Option<Res<Camp>>,
    playlists: Res<Assets<Playlist>>,
    mut music: ResMut<Music>,
    mut rng: ResMut<PresentationRng>,
    mut songs: Query<(Entity, &mut Song, Option<&mut AudioSink>)>,
) {
    let Ok(level) = levels.single() else { return };
    let wanted = if camp.is_some() {
        CAMPING.to_owned()
    } else {
        LevelInfo::for_path(&level.path).map_or(FALLBACK.to_owned(), |info| {
            format!("retail://data/music/{}.mpl", info.file.to_lowercase())
        })
    };
    if music.wanted.as_ref() != Some(&wanted) {
        music.playlist = Some(assets.load(&wanted));
        music.wanted = Some(wanted);
        for (song, ..) in &songs {
            commands.entity(song).despawn();
        }
        music.silence = None;
        music.played = false;
        return;
    }
    let Some(handle) = music.playlist.clone() else { return };
    if matches!(assets.load_state(&handle), LoadState::Failed(_)) {
        music.playlist = Some(assets.load(FALLBACK));
        return;
    }
    let Some(playlist) = playlists.get(&handle) else { return };
    if let Some((_, mut song, sink)) = songs.iter_mut().next() {
        song.fade_in.tick(time.delta());
        if let Some(mut sink) = sink {
            sink.set_volume(Volume::Linear(VOLUME * song.fade_in.fraction()));
        }
        return;
    }
    if let Some(silence) = &mut music.silence {
        if !silence.tick(time.delta()).is_finished() {
            return;
        }
        music.silence = None;
    }
    // A song just ended: perhaps a silence first.
    if music.played && rng.0.random_range(0..100) < playlist.pause_chance {
        let low = playlist.pause_min;
        let seconds = rng.0.random_range(low..=playlist.pause_max.max(low));
        music.silence = Some(Timer::from_seconds(seconds as f32, TimerMode::Once));
        music.played = false;
        return;
    }
    // Weighted, unless any song has no weight: then all alike.
    let weighted = playlist.songs.iter().all(|(_, w)| w.is_some());
    let Ok((file, _)) = playlist
        .songs
        .choose_weighted(&mut rng.0, |(_, w)| if weighted { w.unwrap_or(1) } else { 1 })
    else {
        return;
    };
    info!("music: {file}");
    commands.spawn((
        Name::new(format!("Song {file}")),
        Song { fade_in: Timer::from_seconds(FADE_IN_SECONDS, TimerMode::Once) },
        AudioPlayer::new(assets.load(format!("retail://data/music/{file}"))),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0)),
    ));
    music.played = true;
}
