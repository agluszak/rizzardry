//! Music playlists (`data/music/<name>.mpl`, Whizzardry
//! `MusicPlaylist::Parse`): songs with weights, then pause settings.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;

pub use crate::content::music::Playlist;

#[derive(Default, TypePath)]
pub struct PlaylistLoader;

impl AssetLoader for PlaylistLoader {
    type Asset = Playlist;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Playlist, std::io::Error> {
        let mut text = Vec::new();
        reader.read_to_end(&mut text).await?;
        Ok(read_playlist(&String::from_utf8_lossy(&text)))
    }

    fn extensions(&self) -> &[&str] {
        &["mpl"]
    }
}

pub fn read_playlist(text: &str) -> Playlist {
    let mut playlist = Playlist::default();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        if let Some(setting) = line.strip_prefix('#') {
            let Some((key, value)) = setting.split_once('=') else { continue };
            let value = value.trim().parse().unwrap_or(0);
            match key.trim().to_uppercase().as_str() {
                "PAUSEMIN" => playlist.pause_min = value,
                "PAUSEMAX" => playlist.pause_max = value,
                "PAUSECHANCE" => playlist.pause_chance = value,
                _ => {}
            }
            continue;
        }
        let (file, weight) = match line.split_once('(') {
            Some((file, rest)) => (file, rest.trim_end_matches(')').trim().parse().ok()),
            None => (line, None),
        };
        playlist.songs.push((file.trim().to_lowercase(), weight));
    }
    playlist
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monastery_playlist_reads_as_authored() {
        let text = "*comment\n\nUnderground.MP3(70)\nOutdoor Area.mp3(30) \n* vars\n#PAUSEMIN=50\n#PAUSEMAX=140\n#PAUSECHANCE=80\n";
        let playlist = read_playlist(text);
        assert_eq!(
            playlist.songs,
            [("underground.mp3".to_owned(), Some(70)), ("outdoor area.mp3".to_owned(), Some(30))]
        );
        assert_eq!((playlist.pause_min, playlist.pause_max, playlist.pause_chance), (50, 140, 80));
    }
}
