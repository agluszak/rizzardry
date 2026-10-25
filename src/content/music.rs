//! Level music: which songs play, and how often silence falls between them.

use bevy::prelude::*;

#[derive(Asset, TypePath, Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    /// Song files under `data/music`, with their weights; any song without a
    /// weight makes the choice uniform.
    pub songs: Vec<(String, Option<u32>)>,
    /// After a song, silence for `pause_min..=pause_max` seconds this often
    /// (percent).
    pub pause_min: u32,
    pub pause_max: u32,
    pub pause_chance: u32,
}
