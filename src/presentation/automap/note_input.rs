//! Native text editing with retail bitmap presentation. Bevy owns the buffer,
//! selection, clipboard and IME; this module only maps bitmap pixels to that editor.
use super::*;
use crate::presentation::screen_art::BitmapText;
use bevy::input::keyboard::Key;
use bevy::input_focus::{FocusCause, FocusedInput, InputFocus};
use bevy::text::{
    EditableText, EditableTextFilter, FontCx, LayoutCx, TextCursorStyle, TextEditChange,
};
use bevy::ui_widgets::TextInput;

#[derive(Component)]
pub(crate) struct NoteInput;
#[derive(Component)]
pub(crate) struct HitBox;
#[derive(Component)]
pub(crate) struct Selection;
#[derive(Component)]
pub(crate) struct Caret;
#[derive(Component)]
pub(crate) struct InputLabel;
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
    art: Res<AutomapArt>,
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
            TextFont { font_size: 12.0.into(), ..default() },
            TextColor(Color::NONE),
            TextCursorStyle {
                color: Color::NONE,
                selection_color: Color::NONE,
                unfocused_selection_color: Color::NONE,
                selected_text_color: Some(Color::NONE),
                ..default()
            },
            Node { position_type: PositionType::Absolute, border: UiRect::all(px(1)), ..default() },
            BorderColor::all(EDIT_BOX),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .observe(changed)
        .observe(finish)
        .id();
    let hit = commands
        .spawn((
            HitBox,
            absolute(Vec2::ZERO, Vec2::new(20.0, 14.0)),
            Pickable::default(),
            ChildOf(entity),
        ))
        .observe(press)
        .observe(drag)
        .observe(|mut click: On<PointerClick>| {
            if click.button == bevy::picking::pointer::PointerButton::Primary {
                click.propagate(false);
            }
        })
        .id();
    commands.spawn((
        Selection,
        Node::default(),
        BackgroundColor(Color::srgba(0.8, 0.5, 0.1, 0.5)),
        Pickable::IGNORE,
        ChildOf(hit),
    ));
    commands.spawn((
        InputLabel,
        BitmapText {
            font: art.font.clone(),
            at: Vec2::ONE,
            justify: bevy::text::Justify::Left,
            ..default()
        },
        Node::default(),
        Pickable::IGNORE,
        ChildOf(hit),
    ));
    commands.spawn((
        Caret,
        Node::default(),
        BackgroundColor(EDIT_BOX),
        Pickable::IGNORE,
        ChildOf(hit),
    ));
    focus.set(entity, FocusCause::Navigated);
}

fn changed(
    event: On<TextEditChange>,
    inputs: Query<&EditableText, With<NoteInput>>,
    map: Option<Single<&mut Automap>>,
) {
    let (Ok(input), Some(mut map)) = (inputs.get(event.event_target()), map) else { return };
    if let Some(draft) = map.editing.as_mut() {
        let value = input.value().to_string();
        if draft.note.text != value {
            draft.note.text = value;
        }
    }
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

/// Native edits (including same-frame text/IME and asynchronous paste) settle
/// before Enter commits. Cancellation also waits until after input dispatch,
/// so the same raw Escape cannot re-enable and close the underlying map.
/// A cancelled or replaced editor cannot commit later.
pub(crate) fn finish_note_input(
    inputs: Query<(Entity, &EditableText, &FinishNote)>,
    map: Option<Single<&mut Automap>>,
    mut commands: Commands,
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
            commands.queue(move |world: &mut World| {
                let _ = crate::game::automap::commit_note(world, draft);
            });
        }
        commands.entity(entity).despawn();
    }
}

fn nearest_byte(font: &SpriteSheet, value: &str, x: f32) -> usize {
    value
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(value.len()))
        .min_by(|a, b| {
            ((font.text_width(&value[..*a]) as f32) - x)
                .abs()
                .total_cmp(&((font.text_width(&value[..*b]) as f32) - x).abs())
        })
        .unwrap_or(0)
}

fn press(
    mut event: On<PointerPress>,
    hits: Query<&ChildOf, With<HitBox>>,
    mut editors: Query<&mut EditableText, With<NoteInput>>,
    art: Res<AutomapArt>,
    sheets: Res<Assets<SpriteSheet>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform, &ComputedUiRenderTargetInfo)>,
    scale: Res<UiScale>,
    keys: Res<ButtonInput<Key>>,
    mut fonts: ResMut<FontCx>,
    mut layouts: ResMut<LayoutCx>,
    mut focus: ResMut<InputFocus>,
) {
    if event.button != bevy::picking::pointer::PointerButton::Primary {
        return;
    }
    event.propagate(false);
    let Ok(parent) = hits.get(event.entity) else { return };
    let Ok(mut input) = editors.get_mut(parent.parent()) else { return };
    if input.is_composing() {
        return;
    }
    let (Some(font), Ok((node, transform, target))) =
        (sheets.get(&art.font), nodes.get(event.entity))
    else {
        return;
    };
    let Some(inverse) = transform.try_inverse() else { return };
    let x = inverse.transform_point2(event.pointer.position * target.scale_factor() / scale.0).x
        - node.content_box().min.x
        - 1.0;
    let index = nearest_byte(font, input.editor.raw_text(), x);
    let mut driver = input.editor.driver(&mut fonts, &mut layouts.0);
    if event.count == 2 {
        driver.move_to_byte(index);
        if let Some(caret) = driver.editor.cursor_geometry(1.0) {
            driver.select_word_at_point(caret.x0 as f32, ((caret.y0 + caret.y1) / 2.0) as f32);
        }
    } else if event.count > 2 {
        driver.select_all();
    } else if keys.pressed(Key::Shift) {
        driver.extend_selection_to_byte(index);
    } else {
        driver.move_to_byte(index);
    }
    focus.set(parent.parent(), FocusCause::Pressed);
}

fn drag(
    mut event: On<PointerDrag>,
    hits: Query<&ChildOf, With<HitBox>>,
    mut editors: Query<&mut EditableText, With<NoteInput>>,
    art: Res<AutomapArt>,
    sheets: Res<Assets<SpriteSheet>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform, &ComputedUiRenderTargetInfo)>,
    scale: Res<UiScale>,
    mut fonts: ResMut<FontCx>,
    mut layouts: ResMut<LayoutCx>,
) {
    if event.button != bevy::picking::pointer::PointerButton::Primary {
        return;
    }
    event.propagate(false);
    let Ok(parent) = hits.get(event.entity) else { return };
    let Ok(mut input) = editors.get_mut(parent.parent()) else { return };
    if input.is_composing() {
        return;
    }
    let (Some(font), Ok((node, transform, target))) =
        (sheets.get(&art.font), nodes.get(event.entity))
    else {
        return;
    };
    let Some(inverse) = transform.try_inverse() else { return };
    let x = inverse.transform_point2(event.pointer.position * target.scale_factor() / scale.0).x
        - node.content_box().min.x
        - 1.0;
    let index = nearest_byte(font, input.editor.raw_text(), x);
    input.editor.driver(&mut fonts, &mut layouts.0).extend_selection_to_byte(index);
}

pub(crate) fn draw_note_input(
    map: Option<Single<&Automap>>,
    art: Res<AutomapArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut inputs: Query<(Entity, &EditableText, &mut Node), With<NoteInput>>,
    mut hits: Query<
        &mut Node,
        (With<HitBox>, Without<NoteInput>, Without<Selection>, Without<Caret>),
    >,
    mut labels: Query<&mut BitmapText, With<InputLabel>>,
    mut marks: Query<
        (&mut Node, Has<Caret>),
        (Or<(With<Selection>, With<Caret>)>, Without<NoteInput>, Without<HitBox>),
    >,
    focus: Res<InputFocus>,
    time: Res<Time<Real>>,
) {
    let (Some(map), Some(font)) = (map, sheets.get(&art.font)) else { return };
    let Some(draft) = &map.editing else { return };
    for (entity, input, mut node) in &mut inputs {
        let value = input.editor.raw_text();
        let height = font
            .frames
            .first()
            .map_or(12.0, |f| (i32::from(f.offset[1]) + f.height as i32).max(1) as f32);
        let size = Vec2::new(font.text_width(value).max(20) as f32 + 2.0, height + 2.0);
        let at = map.project(draft.note.position) + Vec2::new(MARKER / 2.0 + 1.0, -size.y / 2.0);
        node.set_if_neq(Node { border: UiRect::all(px(1)), ..absolute(at, size) });
        for mut hit in &mut hits {
            hit.set_if_neq(absolute(Vec2::ZERO, size - Vec2::splat(2.0)));
        }
        for mut label in &mut labels {
            if label.font == art.font && label.value != value {
                label.value = value.to_owned();
            }
        }
        let selection = input.editor.raw_selection();
        let range = selection.text_range();
        let prefix = |i| font.text_width(value.get(..i).unwrap_or(value)) as f32 + 1.0;
        for (mut mark, caret) in &mut marks {
            let (left, width) = if caret {
                (prefix(selection.focus().index()), 1.0)
            } else {
                (prefix(range.start), prefix(range.end) - prefix(range.start))
            };
            let mut desired = absolute(Vec2::new(left, 1.0), Vec2::new(width, height));
            desired.display =
                if focus.get() == Some(entity) && (!caret || (time.elapsed_secs() % 1.0) < 0.5) {
                    Display::Flex
                } else {
                    Display::None
                };
            mark.set_if_neq(desired);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_cancels_the_editor_before_a_second_escape_closes_the_map() {
        use crate::presentation::test_input::{input_app, tap};
        use bevy::input_focus::{InputDispatchPlugin, InputFocusSystems};
        use bevy::window::PrimaryWindow;
        let mut app = input_app();
        app.add_plugins(InputDispatchPlugin)
            .init_state::<Screen>()
            .init_asset::<LevelDefinition>()
            .add_input_context::<AutomapScreen>()
            .add_systems(OnEnter(Screen::Automap), enter_automap)
            .add_systems(
                PreUpdate,
                route_map_keys
                    .after(InputFocusSystems::Dispatch)
                    .before(EnhancedInputSystems::Update),
            )
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
            let notes = &app.world().get::<MapNotes>(level).unwrap().notes;
            assert_eq!(notes.len(), usize::from(accept));
            if accept {
                assert_eq!(notes[0].text, "edited!");
            }
            assert!(app.world().get_entity(input).is_err());
        }
    }
}
