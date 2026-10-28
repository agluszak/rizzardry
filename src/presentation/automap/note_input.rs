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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::text::{FontCx, LayoutCx};
    #[test]
    fn escape_cancels_the_editor_before_a_second_escape_closes_the_map() {
        use crate::presentation::test_input::{input_app, tap};
        use bevy::input_focus::{InputDispatchPlugin, InputFocusSystems};
        use bevy::window::PrimaryWindow;
        let mut app = input_app();
        app.add_plugins(InputDispatchPlugin)
            .init_state::<Screen>()
            .init_asset::<crate::content::level::LevelDefinition>()
            .add_input_context::<AutomapScreen>()
            .add_systems(OnEnter(Screen::Automap), enter_automap)
            .add_systems(
                PreUpdate,
                route_map_keys
                    .after(InputFocusSystems::Dispatch)
                    .before(EnhancedInputSystems::Update),
            )
            .add_message::<crate::game::automap::CommitNote>()
            .add_systems(PostUpdate, finish_note_input);
        let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        app.world_mut().spawn((PartyBody::default(), Transform::default()));
        let level = app.world_mut().spawn(Level::test(default())).id();
        app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Automap);
        app.finish();
        app.cleanup();
        app.update();
        app.update();
        let root =
            app.world_mut().query_filtered::<Entity, With<Automap>>().single(app.world()).unwrap();
        app.world_mut().get_mut::<Automap>(root).unwrap().editing = Some(NoteDraft {
            level,
            original: None,
            note: MapNote { position: Vec2::ZERO, layer: 0, text: "draft".into() },
        });
        let input = app
            .world_mut()
            .spawn((NoteInput, EditableText::new("draft"), ChildOf(root)))
            .observe(finish)
            .id();
        app.insert_resource(InputFocus::from_entity(input));
        tap(&mut app, window, KeyCode::Escape);
        assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Automap);
        assert!(app.world().get::<Automap>(root).unwrap().editing.is_none());
        assert!(app.world().get_entity(input).is_err());
        tap(&mut app, window, KeyCode::Escape);
        assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::World);
    }

    #[test]
    fn native_note_input_commits_or_cancels_without_mutating_saved_notes_early() {
        for accept in [false, true] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<FontCx>()
                .init_resource::<LayoutCx>()
                .init_resource::<bevy::clipboard::Clipboard>()
                .add_message::<crate::game::automap::CommitNote>()
                .add_systems(Update, crate::game::automap::commit_notes)
                .add_systems(PostUpdate, (bevy::text::apply_text_edits, finish_note_input).chain());
            app.world_mut().resource_mut::<FontCx>().collection.register_fonts(
                bevy::text::Font::from_bytes(bevy::text::DEFAULT_FONT_DATA.to_vec()).data,
                None,
            );
            let level = app.world_mut().spawn((Level::test(default()), MapNotes::default())).id();
            let mut map = Automap::new(None, &[], Vec3::ZERO);
            map.level = level;
            map.editing = Some(NoteDraft {
                level,
                original: None,
                note: MapNote { position: Vec2::ZERO, layer: 0, text: "old".into() },
            });
            let root = app.world_mut().spawn(map).id();
            let input = app
                .world_mut()
                .spawn((NoteInput, EditableText::new("edited")))
                .observe(finish)
                .id();
            app.world_mut()
                .get_mut::<EditableText>(input)
                .unwrap()
                .queue_edit(bevy::text::TextEdit::Insert("!".into()));
            assert!(app.world().get::<MapNotes>(level).unwrap().notes.is_empty());
            app.world_mut().trigger(FocusedInput::new(
                input,
                KeyboardInput {
                    key_code: if accept { KeyCode::Enter } else { KeyCode::Escape },
                    logical_key: if accept { Key::Enter } else { Key::Escape },
                    state: bevy::input::ButtonState::Pressed,
                    text: None,
                    repeat: false,
                    window: Entity::PLACEHOLDER,
                },
                Entity::PLACEHOLDER,
            ));
            app.update();
            assert!(app.world().get::<Automap>(root).unwrap().editing.is_none());
            // The accepted draft is committed by the game on the next frame.
            app.update();
            let notes = &app.world().get::<MapNotes>(level).unwrap().notes;
            assert_eq!(notes.len(), usize::from(accept));
            if accept {
                assert_eq!(notes[0].text, "edited!");
            }
            assert!(app.world().get_entity(input).is_err());
        }
    }
}
