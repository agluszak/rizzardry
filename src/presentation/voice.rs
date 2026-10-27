//! Voice lines with identified completion.
//!
//! Dialogue owns what a line means; playback owns its progress. A spoken
//! line is an entity carrying [`Speech`], and [`SpeechFinished`] names that
//! entity when its audio has played out, so a conversation waits for its own
//! line, not whatever sound ended last.

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings};
use bevy::prelude::*;

/// A voice line being spoken.
#[derive(Component, Debug)]
pub struct Speech {
    /// The retail line name, e.g. `npc_aletheides_000`.
    pub line: String,
}

#[derive(Message, Debug, Clone)]
pub struct SpeechFinished {
    pub speech: Entity,
    pub line: String,
}

/// The retail file of a voice line: `data/sound/npcs/<speaker>/<line>.mp3`,
/// where the speaker is the line name without its quote number. Retail
/// scripts ask for `.wav` and fall back to the shipped `.mp3`.
pub fn voice_path(line: &str) -> String {
    let line = line.to_lowercase();
    let speaker = line.rsplit_once('_').map_or(line.as_str(), |(speaker, _)| speaker);
    format!("retail://data/sound/npcs/{speaker}/{line}.mp3")
}

/// Start speaking a line; returns the entity to wait on.
pub fn speak(commands: &mut Commands, assets: &AssetServer, line: &str) -> Entity {
    commands
        .spawn((
            Name::new(format!("Speech {line}")),
            Speech { line: line.to_owned() },
            AudioPlayer::new(assets.load(voice_path(line))),
            PlaybackSettings::ONCE,
        ))
        .id()
}

/// Report and remove lines whose audio has played out.
pub fn finish_speech(
    mut commands: Commands,
    speech: Query<(Entity, &Speech, &AudioSink)>,
    mut finished: MessageWriter<SpeechFinished>,
) {
    for (entity, speech, sink) in &speech {
        if sink.empty() {
            finished.write(SpeechFinished { speech: entity, line: speech.line.clone() });
            commands.entity(entity).despawn();
        }
    }
}
