//! Native text editing with imported retail glyphs. Bevy owns text, caret,
//! selection, pointer hit-testing, clipboard and IME.
use super::*;
use bevy::input::keyboard::Key;
use bevy::input_focus::{FocusCause, FocusedInput, InputFocus};
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle};
use bevy::ui_widgets::TextInput;

#[derive(Component)]
pub(crate) struct NoteInput;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinishNote {
    Accept,
    Cancel,
}

pub(crate) fn sync_note_input(
    mut commands: Commands,
    map: Option<Single<&Automap>>,
    viewport: Query<Entity, With<MapViewport>>,
    editors: Query<Entity, With<NoteInput>>,
    assets: Res<AssetServer>,
    mut focus: ResMut<InputFocus>,
) {
    let draft = map.as_ref().and_then(|map| map.editing.as_ref());
    let Some(draft) = draft else {
        for entity in &editors {
            commands.entity(entity).despawn();
        }
        return;
    };
    if !editors.is_empty() {
        return;
    }
    let Ok(parent) = viewport.single() else { return };
    let entity = commands
        .spawn((
            NoteInput,
            TextInput,
            TextLayout::no_wrap(),
            TabIndex(0),
            EditableText {
                max_characters: Some(crate::game::automap::MAX_NOTE_LENGTH),
                allow_newlines: false,
                ..EditableText::new(&draft.note.text)
            },
            EditableTextFilter::new(|c| crate::content::ui::glyph_index(c).is_some()),
            TextFont {
                font: assets.load("retail://data/fonts/wiz_text_font.sti#font").into(),
                font_size: 11.0.into(),
                ..default()
            },
            TextColor(Color::WHITE),
            TextCursorStyle {
                color: EDIT_BOX,
                selection_color: Color::srgba(0.8, 0.5, 0.1, 0.5),
                ..default()
            },
            Node { position_type: PositionType::Absolute, border: UiRect::all(px(1)), ..default() },
            BorderColor::all(EDIT_BOX),
            Pickable::default(),
            ChildOf(parent),
        ))
        .observe(finish)
        .id();
    focus.set(entity, FocusCause::Navigated);
}

fn finish(
    mut event: On<FocusedInput<KeyboardInput>>,
    inputs: Query<&EditableText, With<NoteInput>>,
    mut commands: Commands,
) {
    let Ok(input) = inputs.get(event.original_event_target()) else { return };
    if !event.input.state.is_pressed() || input.is_composing() {
        return;
    }
    if !matches!(event.input.logical_key, Key::Enter | Key::Escape) {
        return;
    }
    event.propagate(false);
    let outcome =
        if event.input.logical_key == Key::Enter { FinishNote::Accept } else { FinishNote::Cancel };
    commands.entity(event.original_event_target()).insert((outcome, InteractionDisabled));
}

pub(super) fn cancel_from_controls(
    mut event: On<FocusedInput<KeyboardInput>>,
    inputs: Query<Entity, With<NoteInput>>,
    mut commands: Commands,
) {
    if event.input.state.is_pressed()
        && event.input.logical_key == Key::Escape
        && !inputs.is_empty()
    {
        event.propagate(false);
        for entity in &inputs {
            commands.entity(entity).insert((FinishNote::Cancel, InteractionDisabled));
        }
    }
}

/// The editor owns the text until then: the draft keeps the text it started
/// with. Native edits (including same-frame text/IME and asynchronous paste)
/// settle before Enter commits. Cancellation also waits until after input dispatch,
/// so the same raw Escape cannot re-enable and close the underlying map.
/// A cancelled or replaced editor cannot commit later.
pub(crate) fn finish_note_input(
    inputs: Query<(Entity, &EditableText, &FinishNote)>,
    map: Option<Single<&mut Automap>>,
    mut commands: Commands,
    mut notes: MessageWriter<crate::game::automap::CommitNote>,
) {
    let Some(mut map) = map else { return };
    for (entity, input, outcome) in &inputs {
        if *outcome == FinishNote::Accept
            && (!input.pending_edits.is_empty() || input.pending_paste.is_some())
        {
            continue;
        }
        if let Some(mut draft) = map.editing.take()
            && *outcome == FinishNote::Accept
        {
            draft.note.text = input.value().to_string();
            notes.write(crate::game::automap::CommitNote(draft));
        }
        commands.entity(entity).despawn();
    }
}

pub(crate) fn draw_note_input(
    map: Option<Single<&Automap>>,
    art: Res<AutomapArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut inputs: Query<(&EditableText, &mut Node), With<NoteInput>>,
) {
    let (Some(map), Some(font)) = (map, sheets.get(&art.font)) else { return };
    let Some(draft) = &map.editing else { return };
    for (input, mut node) in &mut inputs {
        let height = font
            .frames
            .first()
            .map_or(11., |f| (i32::from(f.offset[1]) + f.height as i32).max(1) as f32);
        let size =
            Vec2::new(font.text_width(input.editor.raw_text()).max(20) as f32 + 2., height + 2.);
        let at = map.project(draft.note.position) + Vec2::new(MARKER / 2. + 1., -size.y / 2.);
        node.set_if_neq(Node { border: UiRect::all(px(1)), ..absolute(at, size) });
    }
}
