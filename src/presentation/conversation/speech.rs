//! Speech pacing lives on the session's line entity, independently of its UI.
use crate::game::npcs::conversation::{
    ConversationRequest, ConversationSpeech, update_conversation,
};
use bevy::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (play_lines, watch_playback, advance_lines).chain().after(update_conversation),
    );
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

fn play_lines(
    mut commands: Commands,
    lines: Query<(Entity, &ConversationSpeech), Added<ConversationSpeech>>,
    assets: Res<AssetServer>,
) {
    for (e, line) in &lines {
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
            requests.write(ConversationRequest::Advance { conversation: parent.parent(), speech });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A said line whose speech never plays shows for its text's time, then
    /// the view advances the conversation past it, once, by the ordinary
    /// request.
    #[test]
    fn a_silent_line_is_advanced_after_its_text_time() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<bevy::audio::AudioSource>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .add_message::<ConversationRequest>()
            .add_systems(Update, (play_lines, watch_playback, advance_lines).chain());
        let conversation = app.world_mut().spawn_empty().id();
        let speech = app
            .world_mut()
            .spawn((
                ConversationSpeech {
                    voice: Some("npc_test_000".into()),
                    text: "Hello".into(),
                    alert: false,
                },
                ChildOf(conversation),
            ))
            .id();
        let mut advanced = Vec::new();
        for frame in 0..40 {
            app.update();
            let requests: Vec<_> =
                app.world_mut().resource_mut::<Messages<ConversationRequest>>().drain().collect();
            advanced.extend(requests.into_iter().map(|request| (frame, request)));
        }
        // 2.3 seconds for five characters; the first frame has no time in it.
        assert_eq!(advanced.len(), 1, "{advanced:?}");
        let (frame, request) = &advanced[0];
        assert!((23..=25).contains(frame), "{frame}");
        assert!(matches!(
            request,
            ConversationRequest::Advance { conversation: c, speech: s }
                if *c == conversation && *s == speech
        ));
    }
}
