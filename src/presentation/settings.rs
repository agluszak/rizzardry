//! Settings are committed only after their file has been replaced successfully.
use crate::game::difficulty::Difficulty;
use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, SpatialAudioSink};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub volume: u8,
    pub invert_mouse_y: bool,
    pub fullscreen: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { volume: 100, invert_mouse_y: false, fullscreen: false }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSettings {
    pub preferences: Preferences,
    pub difficulty: Difficulty,
    pub continuous: bool,
}
#[derive(Resource)]
pub struct SettingsFile(pub PathBuf);
impl Default for SettingsFile {
    fn default() -> Self {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self(base.join("rizzardry/settings.ron"))
    }
}
impl SavedSettings {
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if self.preferences.volume > 100 {
            return Err("Invalid volume.".into());
        }
        let text =
            ron::ser::to_string_pretty(self, Default::default()).map_err(|e| e.to_string())?;
        let parent = path.parent().ok_or("Invalid settings location.")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temporary = path.with_extension("ron.tmp");
        std::fs::write(&temporary, text).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    }
}
pub fn load(
    file: Res<SettingsFile>,
    mut preferences: ResMut<Preferences>,
    mut difficulty: ResMut<Difficulty>,
    mut combat: ResMut<crate::game::combat::CombatSettings>,
) {
    match std::fs::read_to_string(&file.0) {
        Ok(text) => match ron::from_str::<SavedSettings>(&text) {
            Ok(saved) if saved.preferences.volume <= 100 => {
                *preferences = saved.preferences;
                *difficulty = saved.difficulty;
                combat.continuous = saved.continuous;
            }
            _ => warn!("Cannot read settings; using defaults"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!("Cannot read settings: {e}"),
    }
}
pub fn apply(
    preferences: Res<Preferences>,
    mut volume: Option<ResMut<bevy::audio::GlobalVolume>>,
    mut windows: Query<&mut Window>,
    mut sinks: Query<(&PlaybackSettings, &mut AudioSink)>,
    mut spatial: Query<(&PlaybackSettings, &mut SpatialAudioSink)>,
) {
    if !preferences.is_changed() || windows.is_empty() {
        return;
    }
    let master = bevy::audio::Volume::Linear(f32::from(preferences.volume) / 100.);
    for (settings, mut sink) in &mut sinks {
        sink.set_volume(settings.volume * master);
    }
    for (settings, mut sink) in &mut spatial {
        sink.set_volume(settings.volume * master);
    }
    if let Some(ref mut volume) = volume {
        volume.volume = master;
    }
    for mut window in &mut windows {
        window.mode = if preferences.fullscreen {
            bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
        } else {
            bevy::window::WindowMode::Windowed
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_and_failed_replace_preserves_previous_file() {
        let directory =
            std::env::temp_dir().join(format!("rizzardry-settings-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.ron");
        let saved = SavedSettings {
            preferences: Preferences { volume: 30, invert_mouse_y: true, fullscreen: false },
            difficulty: Difficulty::Expert,
            continuous: true,
        };
        saved.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let loaded: SavedSettings = ron::from_str(&text).unwrap();
        assert_eq!(loaded.preferences, saved.preferences);
        assert_eq!(loaded.difficulty, Difficulty::Expert);
        assert!(loaded.continuous);
        // A failed temporary-file write must not truncate the accepted settings.
        std::fs::create_dir(path.with_extension("ron.tmp")).unwrap();
        assert!(saved.save(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
