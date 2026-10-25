//! Level music from retail playlists (Whizzardry `music_playlist.cpp`): a
//! weighted song, faded in over five seconds; after each, sometimes a
//! silence; another level's playlist when the party travels.

use bevy::asset::LoadState;
use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::content::levels::LevelInfo;
use crate::content::music::Playlist;
use crate::game::world::level::Level;

const VOLUME: f32 = 0.6;
const FADE_IN_SECONDS: f32 = 5.0;
/// Levels without a playlist of their own play this one.
const FALLBACK: &str = "retail://data/music/adventure.mpl";

#[derive(Resource, Debug, Default)]
pub struct Music {
    /// The level the music is for, and its playlist.
    level: Option<String>,
    playlist: Option<Handle<Playlist>>,
    /// No new song before this time.
    quiet_until: f32,
    /// A song has played since the level's music began, so the next gap
    /// may be a silence.
    played: bool,
    /// Presentation randomness, apart from gameplay's.
    noise: u32,
}

impl Music {
    fn random(&mut self, below: u32) -> u32 {
        let mut x = self.noise.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x;
        if below == 0 { 0 } else { x % below }
    }
}

/// The song playing.
#[derive(Component, Debug)]
pub struct Song {
    started: f32,
}

pub fn play_music(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    levels: Query<&Level>,
    playlists: Res<Assets<Playlist>>,
    mut music: ResMut<Music>,
    mut songs: Query<(Entity, &Song, Option<&mut AudioSink>)>,
) {
    let now = time.elapsed_secs();
    let Ok(level) = levels.single() else { return };
    if music.level.as_deref() != Some(level.path.as_str()) {
        let file = LevelInfo::for_path(&level.path).map(|info| info.file.to_lowercase());
        music.playlist = Some(match file {
            Some(file) => assets.load(format!("retail://data/music/{file}.mpl")),
            None => assets.load(FALLBACK),
        });
        music.level = Some(level.path.clone());
        for (song, _, _) in &songs {
            commands.entity(song).despawn();
        }
        music.quiet_until = now;
        music.played = false;
        return;
    }
    let Some(handle) = music.playlist.clone() else { return };
    if matches!(assets.load_state(&handle), LoadState::Failed(_)) {
        music.playlist = Some(assets.load(FALLBACK));
        return;
    }
    let Some(playlist) = playlists.get(&handle) else { return };
    if let Some((_, song, sink)) = songs.iter_mut().next() {
        if let Some(mut sink) = sink {
            let faded = ((now - song.started) / FADE_IN_SECONDS).clamp(0.0, 1.0);
            sink.set_volume(Volume::Linear(VOLUME * faded));
        }
        return;
    }
    if now < music.quiet_until || playlist.songs.is_empty() {
        return;
    }
    // A song just ended: perhaps a silence first.
    if music.played && music.random(100) < playlist.pause_chance {
        let span = playlist.pause_max.saturating_sub(playlist.pause_min);
        music.quiet_until = now + (playlist.pause_min + music.random(span + 1)) as f32;
        music.played = false;
        return;
    }
    let weights: Vec<u32> = if playlist.songs.iter().any(|(_, w)| w.is_none()) {
        vec![1; playlist.songs.len()]
    } else {
        playlist.songs.iter().map(|(_, w)| w.unwrap_or(1)).collect()
    };
    let mut roll = music.random(weights.iter().sum::<u32>().max(1));
    let mut pick = 0;
    for (index, &weight) in weights.iter().enumerate() {
        if roll < weight {
            pick = index;
            break;
        }
        roll -= weight;
    }
    let file = &playlist.songs[pick].0;
    info!("music: {file}");
    commands.spawn((
        Name::new(format!("Song {file}")),
        Song { started: now },
        AudioPlayer::new(assets.load(format!("retail://data/music/{file}"))),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0)),
    ));
    music.played = true;
}
