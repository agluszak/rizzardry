//! Speech pacing lives on the session's line entity, independently of its UI.
use crate::game::npcs::conversation::{
    ConversationRequest, ConversationSpeech, Talk, update_conversation,
};
use bevy::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_observer(play_line)
        .add_systems(Update, (watch_playback, advance_lines).chain().after(update_conversation));
}

/// How long a said line shows before the conversation is advanced past
/// it: the text's own time, or its speech while it plays and a short hold
/// after. Only presentation paces lines; the script waits for the advance.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
enum LinePace {
    Text { remaining: f32 },
    Speech,
    Hold { remaining: f32 },
    Said,
}

/// The time a line shows without its speech: 2 seconds and 0.06 a
/// character.
fn text_seconds(text: &str) -> f32 {
    text.encode_utf16().count() as f32 * 0.06 + 2.0
}

fn play_line(
    event: On<Add<ConversationSpeech>>,
    mut commands: Commands,
    lines: Query<&ConversationSpeech>,
    assets: Res<AssetServer>,
) {
    let e = event.entity;
    let Ok(line) = lines.get(e) else { return };
    commands.entity(e).insert(LinePace::Text { remaining: text_seconds(&line.text) });
    if let Some(voice) = &line.voice {
        commands.entity(e).insert((
            bevy::audio::AudioPlayer::new(
                assets.load(crate::presentation::voice::voice_path(voice)),
            ),
            bevy::audio::PlaybackSettings::ONCE,
        ));
    }
    // Retail `SpeakNpcSubquote`: the alert chime, while the line waits
    // out its text, not the sound.
    if line.alert {
        commands.spawn((
            bevy::audio::AudioPlayer::new(
                assets.load("retail://data/sound/npcs/dialogue alert.wav"),
            ),
            bevy::audio::PlaybackSettings::DESPAWN,
        ));
    }
}
/// A line whose speech plays shows while it does, then holds briefly; one
/// whose speech never plays shows for its text's time.
fn watch_playback(mut lines: Query<(&bevy::audio::AudioSink, &mut LinePace)>) {
    use bevy::audio::AudioSinkPlayback;
    for (sink, mut pace) in &mut lines {
        if matches!(*pace, LinePace::Text { .. }) {
            *pace = LinePace::Speech;
        }
        if *pace == LinePace::Speech && sink.empty() {
            *pace = LinePace::Hold { remaining: 0.7 };
        }
    }
}
/// Advance the conversation past each line once it has shown, by the same
/// request any caller makes.
fn advance_lines(
    time: Res<Time<Real>>,
    mut lines: Query<(Entity, &mut LinePace, &ChildOf), With<ConversationSpeech>>,
    mut requests: MessageWriter<ConversationRequest>,
) {
    for (speech, mut pace, parent) in &mut lines {
        let (LinePace::Text { remaining } | LinePace::Hold { remaining }) = &mut *pace else {
            continue;
        };
        *remaining -= time.delta_secs();
        if *remaining <= 0.0 {
            *pace = LinePace::Said;
            requests.write(ConversationRequest::new(parent.parent(), Talk::Advance { speech }));
        }
    }
}
