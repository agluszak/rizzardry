//! Apply presentation preferences to the window and audio outputs.
use crate::settings::Preferences;
use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, SpatialAudioSink};
use bevy::prelude::*;

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
