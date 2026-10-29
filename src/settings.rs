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
    pub view_tracking: crate::presentation::party_view::ViewTracking,
    pub view_turning: crate::presentation::party_view::ViewTurning,
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
    });
    app.insert_resource(file)
        .insert_resource(saved.preferences)
        .insert_resource(saved.difficulty)
        .insert_resource(crate::game::combat::CombatSettings {
            continuous: continuous || saved.continuous,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_loads_without_presentation_and_cli_wins_before_startup() {
        let directory =
            std::env::temp_dir().join(format!("rizzardry-bootstrap-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.ron");
        let saved = SavedSettings {
            preferences: Preferences { volume: 30, ..default() },
            difficulty: Difficulty::Expert,
            continuous: false,
        };
        saved.save(&path).unwrap();
        for explicit in [false, true] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            initialize(&mut app, SettingsFile(path.clone()), explicit);
            assert_eq!(*app.world().resource::<Difficulty>(), Difficulty::Expert);
            assert_eq!(app.world().resource::<Preferences>(), &saved.preferences);
            assert_eq!(
                app.world().resource::<crate::game::combat::CombatSettings>().continuous,
                explicit
            );
            app.update();
            assert_eq!(
                app.world().resource::<crate::game::combat::CombatSettings>().continuous,
                explicit
            );
            assert!(app.world_mut().query::<&Window>().iter(app.world()).next().is_none());
        }
        // A rejected file cannot apply only its gameplay fields; absent and
        // invalid files both leave the same defaults plus the explicit override.
        for contents in [Some("(preferences:(volume:255),difficulty:Expert,continuous:true)"), None]
        {
            if let Some(text) = contents {
                std::fs::write(&path, text).unwrap();
            } else {
                std::fs::remove_file(&path).unwrap();
            }
            let mut app = App::new();
            initialize(&mut app, SettingsFile(path.clone()), true);
            assert_eq!(*app.world().resource::<Difficulty>(), Difficulty::Normal);
            assert_eq!(*app.world().resource::<Preferences>(), Preferences::default());
            assert!(app.world().resource::<crate::game::combat::CombatSettings>().continuous);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn settings_round_trip_and_failed_replace_preserves_previous_file() {
        let directory =
            std::env::temp_dir().join(format!("rizzardry-settings-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.ron");
        let saved = SavedSettings {
            preferences: Preferences { volume: 30, invert_mouse_y: true, fullscreen: false, ..default() },
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
