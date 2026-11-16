//! Application configuration: load before gameplay starts, independently of rendering.
//! Accepted option changes replace the file before changing the live owners.
use crate::game::difficulty::Difficulty;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub volume: u8,
    pub invert_mouse_y: bool,
    pub fullscreen: bool,
    pub view_tracking: ViewTracking,
    pub view_turning: ViewTurning,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            volume: 100,
            invert_mouse_y: false,
            fullscreen: false,
            view_tracking: default(),
            view_turning: default(),
        }
    }
}
/// Whom the view turns to in a fight (retail `camera_rotation_mode`; new
/// games track every target, `Gameplay Init.cpp`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewTracking {
    Off,
    /// Only a monster going for the selected member.
    SelectedMember,
    #[default]
    AllTargets,
}

/// How the view turns (retail `camera_rotation_style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewTurning {
    #[default]
    Smooth,
    Snap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSettings {
    pub preferences: Preferences,
    pub difficulty: Difficulty,
    pub continuous: bool,
    /// A member who finds nobody to strike tries their other weapon set
    /// (retail `autoswap_weapons`, on in a new game).
    #[serde(default = "on")]
    pub autoswap_weapons: bool,
}

fn on() -> bool {
    true
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
/// Load once before Startup in both rendered and headless apps. Explicit
/// command-line overrides win over the file; a new-game choice may change
/// difficulty later through its normal gameplay operation.
pub fn initialize(app: &mut App, file: SettingsFile, continuous: bool) {
    let saved = match std::fs::read_to_string(&file.0) {
        Ok(text) => match ron::from_str::<SavedSettings>(&text) {
            Ok(saved) if saved.preferences.volume <= 100 => Some(saved),
            _ => {
                warn!("Cannot read settings; using defaults");
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("Cannot read settings: {e}");
            None
        }
    };
    let saved = saved.unwrap_or(SavedSettings {
        preferences: Preferences::default(),
        difficulty: Difficulty::default(),
        continuous: false,
        autoswap_weapons: true,
    });
    app.insert_resource(file)
        .insert_resource(saved.preferences)
        .insert_resource(saved.difficulty)
        .insert_resource(crate::game::combat::CombatSettings {
            continuous: continuous || saved.continuous,
            autoswap_weapons: saved.autoswap_weapons,
        });
}
