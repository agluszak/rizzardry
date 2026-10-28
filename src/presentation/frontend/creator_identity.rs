//! Retail portrait, personality and name page.
use super::{
    Action, Creation, NameInput, NicknameInput, PORTRAITS, Page, absolute, creator_stats,
    finish_name, update_name,
};
use crate::content::characters::{Personality, Voice};
use crate::content::ui::SpriteSheet;
use crate::content::ui::TextTable;
use crate::presentation::retail_controls::{ControlSkin, FocusHighlight};
use crate::presentation::screen_art::{BitmapText, Picture, spawn_picture};
use bevy::audio::{AudioPlayer, PlaybackSettings};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{
    FocusCause, FocusedInput, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::picking::{
    events::{PointerClick, PointerDrag, PointerPress},
    hover::Hovered,
    pointer::PointerButton,
};
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, FontCx, LayoutCx, TextCursorStyle};
use bevy::ui::{Checked, ComputedUiRenderTargetInfo, Pressed, UiGlobalTransform};
use bevy::ui_widgets::{Activate, Button, ModalDialog};
use bevy::ui_widgets::{RadioButton, RadioGroup, TextInput, ValueChange};

// Retail portrait defaults by gender, playable race, and profession.
const DEFAULT_PORTRAITS: [[[usize; 15]; 11]; 2] = [
    [
        [3, 1, 0, 0, 3, 56, 4, 2, 1, 2, 5, 4, 24, 4, 5],
        [13, 12, 0, 12, 13, 56, 13, 12, 12, 12, 14, 14, 14, 14, 14],
        [19, 19, 0, 19, 19, 56, 20, 20, 20, 20, 18, 18, 18, 18, 18],
        [24, 24, 0, 24, 24, 56, 24, 24, 25, 25, 25, 25, 25, 25, 25],
        [28, 28, 0, 28, 28, 56, 28, 28, 29, 29, 29, 29, 29, 29, 29],
        [32, 32, 0, 32, 32, 56, 32, 32, 33, 33, 33, 33, 33, 33, 33],
        [36, 36, 0, 36, 36, 56, 36, 36, 37, 37, 37, 37, 37, 37, 37],
        [40, 40, 0, 40, 40, 56, 41, 40, 41, 40, 41, 41, 41, 41, 41],
        [45, 45, 0, 45, 45, 56, 44, 44, 44, 44, 44, 44, 44, 44, 44],
        [48, 48, 0, 48, 48, 56, 48, 48, 49, 49, 49, 49, 49, 49, 49],
        [52, 52, 0, 52, 52, 56, 52, 52, 53, 53, 53, 53, 53, 53, 53],
    ],
    [
        [9, 11, 9, 10, 9, 57, 10, 11, 7, 7, 7, 8, 8, 8, 8],
        [15, 15, 15, 17, 15, 57, 17, 17, 16, 16, 17, 16, 16, 17, 16],
        [22, 21, 22, 22, 22, 57, 22, 21, 21, 23, 23, 23, 23, 23, 23],
        [26, 26, 26, 26, 26, 57, 26, 26, 27, 27, 27, 27, 27, 27, 27],
        [30, 30, 30, 30, 30, 57, 30, 30, 31, 31, 31, 31, 31, 31, 31],
        [35, 35, 35, 35, 35, 57, 35, 35, 34, 34, 34, 34, 34, 34, 34],
        [38, 38, 38, 38, 38, 57, 38, 38, 39, 39, 39, 39, 39, 39, 39],
        [42, 42, 42, 42, 42, 57, 42, 42, 43, 43, 43, 43, 43, 43, 43],
        [47, 47, 47, 47, 47, 57, 47, 47, 46, 46, 46, 46, 46, 46, 46],
        [50, 50, 50, 50, 50, 57, 50, 50, 51, 51, 51, 51, 51, 51, 51],
        [55, 55, 55, 55, 55, 57, 55, 55, 52, 52, 52, 52, 52, 52, 52],
    ],
];

const GROUPS: [&[usize]; 12] = [
    &[0, 1, 2, 3, 76, 4, 5, 6, 7, 8, 9, 77, 10, 11],
    &[12, 13, 14, 78, 15, 16, 17, 79],
    &[18, 19, 20, 21, 22, 23],
    &[24, 25, 26, 27],
    &[28, 29, 30, 31],
    &[32, 33, 34, 35],
    &[36, 37, 38, 39],
    &[40, 41, 42, 43],
    &[44, 45, 46, 47],
    &[48, 49, 50, 51],
    &[52, 53, 54, 55],
    &[56, 57],
];

/// Retail derives the initial portrait, personality and voice from the recipe.
pub(super) fn initialize(draft: &mut super::CharacterDraft) {
    use crate::content::characters::{Gender, Profession, Race};
    let portraits = DEFAULT_PORTRAITS[draft.gender() as usize][draft.race() as usize];
    draft.portrait = PORTRAITS[portraits[draft.profession() as usize]].into();
    let class = if draft.gender() == Gender::Male
        && matches!(
            draft.profession(),
            Profession::Fighter | Profession::Lord | Profession::Ranger | Profession::Rogue
        ) {
        match draft.race() {
            Race::Dwarf => 16,
            Race::Lizardman => 15,
            _ => draft.profession() as usize,
        }
    } else {
        draft.profession() as usize
    };
    let table = [
        (0, 2),
        (0, 1),
        (6, 1),
        (0, 1),
        (0, 1),
        (2, 2),
        (0, 1),
        (8, 1),
        (7, 2),
        (7, 2),
        (7, 2),
        (7, 2),
        (7, 2),
        (7, 2),
        (4, 1),
        (4, 1),
        (1, 1),
        (5, 2),
        (7, 1),
        (6, 2),
        (6, 2),
        (6, 1),
        (3, 2),
        (3, 2),
        (5, 1),
        (1, 1),
        (1, 2),
        (5, 2),
        (1, 1),
        (1, 2),
        (2, 1),
        (2, 1),
        (2, 2),
        (2, 2),
    ];
    let (personality, voice) = table[class * 2 + draft.gender() as usize];
    let choices = [
        Personality::Aggressive,
        Personality::Intellectual,
        Personality::Burly,
        Personality::Chaotic,
        Personality::Cunning,
        Personality::Eccentric,
        Personality::Kindly,
        Personality::Laidback,
        Personality::Loner,
    ];
    draft.personality = choices[personality];
    draft.voice = if voice == 1 { Voice::First } else { Voice::Second };
}
fn position(current: &str) -> (usize, usize) {
    let index = PORTRAITS.iter().position(|&p| p == current).unwrap_or(0);
    GROUPS
        .iter()
        .enumerate()
        .find_map(|(group, items)| items.iter().position(|&i| i == index).map(|i| (group, i)))
        .unwrap_or((0, 0))
}
pub(super) fn next_portrait(current: &str, direction: i32) -> &'static str {
    let (group, index) = position(current);
    let items = GROUPS[group];
    PORTRAITS[items[(index as i32 + direction).rem_euclid(items.len() as i32) as usize]]
}
pub(super) fn next_portrait_group(current: &str, direction: i32) -> &'static str {
    let (group, _) = position(current);
    let group = (group as i32 + direction).rem_euclid(GROUPS.len() as i32) as usize;
    PORTRAITS[GROUPS[group][0]]
}
#[derive(Component, Default)]
pub(super) struct QuoteSource {
    key: Option<(crate::content::characters::Gender, Personality, Voice)>,
    table: Handle<TextTable>,
}
#[derive(Component)]
pub(super) struct QuoteLine(usize);

#[derive(Component)]
pub(super) struct Portrait;
#[derive(Component, Clone, Copy)]
pub(super) enum Choice {
    Personality(Personality),
    Voice(Voice),
}
#[derive(Component)]
pub(super) struct InputHit;
#[derive(Component)]
pub(super) struct InputText;
#[derive(Component)]
pub(super) struct InputCaret;
#[derive(Component)]
pub(super) struct InputSelection;
#[derive(Component)]
pub(super) struct BitmapInput {
    font: Handle<SpriteSheet>,
}

fn input(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    y: f32,
    nickname: bool,
) {
    let filter = EditableTextFilter::new(|c| {
        !c.is_control()
            && !"\\/:*?\"<>|".contains(c)
            && crate::content::ui::glyph_index(c).is_some()
    });
    let mut entity = commands.spawn((
        TextInput,
        TextLayout::no_wrap(),
        TabIndex(0),
        EditableText {
            max_characters: Some(if nickname { 9 } else { 39 }),
            allow_newlines: false,
            ..EditableText::new("")
        },
        filter,
        TextFont { font_size: 11.0.into(), ..default() },
        TextColor(Color::NONE),
        TextCursorStyle {
            color: Color::NONE,
            selection_color: Color::NONE,
            unfocused_selection_color: Color::NONE,
            selected_text_color: Some(Color::NONE),
            ..default()
        },
        BitmapInput { font: font.clone() },
        absolute(Vec2::new(346., y), Vec2::new(264., 16.)),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
    if nickname {
        entity.insert(NicknameInput);
    } else {
        entity.insert(NameInput);
    }
    let input = entity.observe(update_name).observe(finish_name).id();
    let hit = commands
        .spawn((
            InputHit,
            absolute(Vec2::ZERO, Vec2::new(264., 16.)),
            BackgroundColor(Color::NONE),
            Pickable::default(),
            ChildOf(input),
        ))
        .observe(press_input)
        .observe(drag_input)
        .observe(|mut event: On<PointerClick>| {
            if event.button == PointerButton::Primary {
                event.propagate(false);
            }
        })
        .id();
    commands.spawn((
        InputSelection,
        Node::default(),
        BackgroundColor(Color::srgba(0.45, 0.55, 0.9, 0.5)),
        Pickable::IGNORE,
        ChildOf(hit),
    ));
    commands.spawn((
        InputText,
        BitmapText { font, at: Vec2::new(2., 2.), ..default() },
        ChildOf(hit),
    ));
    commands.spawn((
        InputCaret,
        Node::default(),
        BackgroundColor(Color::WHITE),
        Pickable::IGNORE,
        ChildOf(hit),
    ));
}
fn choice_group(commands: &mut Commands, root: Entity, assets: &AssetServer, personality: bool) {
    let font: Handle<SpriteSheet> = assets.load("retail://data/fonts/wiz_text_font.sti");
    let icons: Handle<SpriteSheet> =
        assets.load("retail://data/char generation/cg_personality.sti");
    let group = commands
        .spawn((
            RadioGroup,
            TabIndex(0),
            crate::presentation::retail_controls::FocusRing,
            absolute(Vec2::ZERO, Vec2::new(640., 450.)),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .observe(select)
        .id();
    let personalities = [
        Personality::Aggressive,
        Personality::Intellectual,
        Personality::Burly,
        Personality::Chaotic,
        Personality::Cunning,
        Personality::Eccentric,
        Personality::Kindly,
        Personality::Laidback,
        Personality::Loner,
    ];
    let count = if personality { 9 } else { 2 };
    for (i, personality_value) in personalities.iter().copied().take(count).enumerate() {
        let (x, y, choice, id, width) = if personality {
            (
                231. + (i % 3) as f32 * 128.,
                306. + (i / 3) as f32 * 14.,
                Choice::Personality(personality_value),
                0x3ad + i,
                127.,
            )
        } else {
            (
                228.,
                363. + i as f32 * 29.,
                Choice::Voice(if i == 0 { Voice::First } else { Voice::Second }),
                0x8d + i,
                72.,
            )
        };
        let row = commands
            .spawn((
                RadioButton,
                choice,
                Hovered::default(),
                absolute(Vec2::new(x, y), Vec2::new(width, 14.)),
                Pickable::default(),
                ChildOf(group),
            ))
            .id();
        spawn_picture(commands, row, icons.clone(), 5, Vec2::new(0., 1.));
        creator_stats::label(
            commands,
            row,
            font.clone(),
            Vec2::new(11., 1.),
            width - 11.,
            bevy::text::Justify::Left,
            creator_stats::Value::String(id),
        );
    }
}
fn select(
    event: On<ValueChange<Entity>>,
    choices: Query<&Choice>,
    mut sessions: Query<&mut Creation>,
) {
    let (Ok(choice), Ok(mut session)) = (choices.get(event.value), sessions.single_mut()) else {
        return;
    };
    let Some(draft) = session.mode.draft_mut() else { return };
    match choice {
        Choice::Personality(value) => draft.personality = *value,
        Choice::Voice(value) => draft.voice = *value,
    }
}

pub(super) fn build(commands: &mut Commands, root: Entity, assets: &AssetServer) {
    creator_stats::frame(commands, root, assets, Page::Review);
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let art = load("char generation/cg_personality");
    let buttons = load("char generation/cg_buttons");
    let bottom = load("char generation/cg_bottombuttons");
    let font = load("fonts/wiz_text_font");
    spawn_picture(commands, root, art, 0, Vec2::new(195., 43.));
    commands.entity(root).insert(QuoteSource::default());
    for line in 0..4 {
        commands.spawn((
            QuoteLine(line),
            BitmapText {
                font: font.clone(),
                at: Vec2::new(307., 361. + line as f32 * 11.),
                width: 224.,
                justify: bevy::text::Justify::Center,
                ..default()
            },
            ChildOf(root),
        ));
    }
    let portrait = spawn_picture(
        commands,
        root,
        assets.load("retail://data/portraits/large/lhummf.sti"),
        0,
        Vec2::new(329., 48.),
    );
    commands.entity(portrait).insert(Portrait);
    for (id, x, y) in [(0xee, 239., 69.), (0xef, 547., 69.), (0xf0, 239., 147.), (0xf1, 547., 147.)]
    {
        for line in 0..2 {
            creator_stats::label(
                commands,
                root,
                font.clone(),
                Vec2::new(x, y + line as f32 * 11.),
                56.,
                bevy::text::Justify::Center,
                creator_stats::Value::Wrapped(id, line, 56),
            );
        }
    }
    for (id, x, y, w) in
        [(0x84, 231., 215., 110.), (0x85, 231., 243., 110.), (0x86, 219., 281., 400.)]
    {
        creator_stats::label(
            commands,
            root,
            font.clone(),
            Vec2::new(x, y),
            w,
            bevy::text::Justify::Center,
            creator_stats::Value::String(id),
        );
    }
    for (x, y, frame, action) in [
        (295., 68., 10, Action::PortraitGroup(-1)),
        (519., 68., 15, Action::PortraitGroup(1)),
        (295., 146., 10, Action::Portrait(-1)),
        (519., 146., 15, Action::Portrait(1)),
    ] {
        // These controls use five frames with their pressed and hover order reversed.
        commands.spawn((
            bevy::ui_widgets::Button,
            TabIndex(0),
            FocusHighlight,
            creator_stats::StatsControl,
            action,
            ControlSkin::new(frame, frame + 1, frame + 2, frame + 2, frame + 3),
            Picture { sheet: buttons.clone(), frame, at: Vec2::new(x, y), bounds: Some(frame) },
            Pickable::default(),
            ChildOf(root),
        ));
    }
    input(commands, root, font.clone(), 214., false);
    input(commands, root, font, 242., true);
    choice_group(commands, root, assets, true);
    choice_group(commands, root, assets, false);
    commands
        .spawn((
            Button,
            TabIndex(0),
            FocusHighlight,
            ControlSkin::new(20, 21, 22, 22, 24),
            Picture {
                sheet: buttons.clone(),
                frame: 20,
                at: Vec2::new(560., 384.),
                bounds: Some(20),
            },
            Pickable::default(),
            ChildOf(root),
        ))
        .observe(open_summary);
    creator_stats::button(commands, root, bottom.clone(), Vec2::new(464., 450.), 4, Action::Cancel);
    spawn_picture(commands, root, bottom.clone(), 23, Vec2::new(508., 450.));
    creator_stats::button(
        commands,
        root,
        bottom.clone(),
        Vec2::new(552., 450.),
        12,
        Action::Page(Page::Skills),
    );
    creator_stats::button(commands, root, bottom, Vec2::new(596., 450.), 16, Action::Finish);
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
fn press_input(
    mut event: On<PointerPress>,
    hits: Query<&ChildOf, With<InputHit>>,
    mut inputs: Query<(&mut EditableText, &BitmapInput)>,
    sheets: Res<Assets<SpriteSheet>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform, &ComputedUiRenderTargetInfo)>,
    scale: Res<UiScale>,
    keys: Res<ButtonInput<Key>>,
    mut fonts: ResMut<FontCx>,
    mut layouts: ResMut<LayoutCx>,
    mut focus: ResMut<InputFocus>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    event.propagate(false);
    let (Ok(parent), Ok((node, transform, target))) =
        (hits.get(event.entity), nodes.get(event.entity))
    else {
        return;
    };
    let Ok((mut input, art)) = inputs.get_mut(parent.parent()) else { return };
    if input.is_composing() {
        return;
    }
    let (Some(font), Some(inverse)) = (sheets.get(&art.font), transform.try_inverse()) else {
        return;
    };
    let x = inverse.transform_point2(event.pointer.position * target.scale_factor() / scale.0).x
        - node.content_box().min.x
        - 2.;
    let index = nearest_byte(font, input.editor.raw_text(), x);
    let mut driver = input.editor.driver(&mut fonts, &mut layouts.0);
    if event.count == 2 {
        driver.move_to_byte(index);
        if let Some(caret) = driver.editor.cursor_geometry(1.0) {
            driver.select_word_at_point(caret.x0 as f32, ((caret.y0 + caret.y1) / 2.) as f32);
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
fn drag_input(
    mut event: On<PointerDrag>,
    hits: Query<&ChildOf, With<InputHit>>,
    mut inputs: Query<(&mut EditableText, &BitmapInput)>,
    sheets: Res<Assets<SpriteSheet>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform, &ComputedUiRenderTargetInfo)>,
    scale: Res<UiScale>,
    mut fonts: ResMut<FontCx>,
    mut layouts: ResMut<LayoutCx>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    event.propagate(false);
    let (Ok(parent), Ok((node, transform, target))) =
        (hits.get(event.entity), nodes.get(event.entity))
    else {
        return;
    };
    let Ok((mut input, art)) = inputs.get_mut(parent.parent()) else { return };
    if input.is_composing() {
        return;
    }
    let (Some(font), Some(inverse)) = (sheets.get(&art.font), transform.try_inverse()) else {
        return;
    };
    let x = inverse.transform_point2(event.pointer.position * target.scale_factor() / scale.0).x
        - node.content_box().min.x
        - 2.;
    let index = nearest_byte(font, input.editor.raw_text(), x);
    input.editor.driver(&mut fonts, &mut layouts.0).extend_selection_to_byte(index);
}

pub(super) fn refresh(
    session: Option<Single<Ref<Creation>>>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    inputs: Query<(Entity, &EditableText, &BitmapInput, &Children)>,
    mut hits: Query<(&Children, &mut BackgroundColor), With<InputHit>>,
    mut text: Query<&mut BitmapText, With<InputText>>,
    mut marks: Query<(&mut Node, Has<InputCaret>), Or<(With<InputCaret>, With<InputSelection>)>>,
    focus: Res<InputFocus>,
    time: Res<Time<Real>>,
    choices: Query<(Entity, &Choice, &Hovered, Has<Pressed>, Has<Checked>, &Children)>,
    mut pictures: Query<&mut Picture, Without<Portrait>>,
    mut portraits: Query<&mut Picture, With<Portrait>>,
    mut commands: Commands,
) {
    let Some(session) = session else { return };
    let Some(draft) = session.mode.draft() else { return };
    for (entity, input, art, children) in &inputs {
        let Some(font) = sheets.get(&art.font) else { continue };
        let value = input.editor.raw_text();
        let selection = input.editor.raw_selection();
        let range = selection.text_range();
        let prefix = |i| font.text_width(value.get(..i).unwrap_or(value)) as f32 + 2.;
        for child in children.iter() {
            let Ok((kids, mut background)) = hits.get_mut(child) else { continue };
            background.set_if_neq(BackgroundColor(if focus.get() == Some(entity) {
                Color::srgb_u8(25, 41, 96)
            } else {
                Color::NONE
            }));
            for kid in kids.iter() {
                if let Ok(mut label) = text.get_mut(kid)
                    && label.value != value
                {
                    label.value = value.to_owned();
                }
                if let Ok((mut node, caret)) = marks.get_mut(kid) {
                    let (left, width) = if caret {
                        (prefix(selection.focus().index()), 1.)
                    } else {
                        (prefix(range.start), prefix(range.end) - prefix(range.start))
                    };
                    let mut desired = absolute(Vec2::new(left, 2.), Vec2::new(width, 12.));
                    desired.display = if focus.get() == Some(entity)
                        && (!caret || time.elapsed_secs() % 1.0 < 0.5)
                    {
                        Display::Flex
                    } else {
                        Display::None
                    };
                    node.set_if_neq(desired);
                }
            }
        }
    }
    for (entity, choice, hover, pressed, checked, children) in &choices {
        let selected = match choice {
            Choice::Personality(p) => *p == draft.personality,
            Choice::Voice(v) => *v == draft.voice,
        };
        if selected != checked {
            if selected {
                commands.entity(entity).insert(Checked);
            } else {
                commands.entity(entity).remove::<Checked>();
            }
        }
        let frame = if selected {
            if hover.get() { 8 } else { 7 }
        } else if hover.get() || pressed {
            6
        } else {
            5
        };
        for child in children.iter() {
            if let Ok(mut picture) = pictures.get_mut(child)
                && picture.frame != frame
            {
                picture.frame = frame;
            }
        }
    }
    let handle: Handle<SpriteSheet> =
        assets.load(format!("retail://data/portraits/large/l{}.sti", draft.portrait));
    // The portrait itself keeps the selected sheet alive.
    for mut picture in &mut portraits {
        if picture.sheet != handle {
            picture.sheet = handle.clone();
        }
    }
}

/// The description comes from the same personality, gender and voice quote file as retail.
pub(super) fn refresh_quote(
    session: Option<Single<Ref<Creation>>>,
    source: Option<Single<&mut QuoteSource>>,
    assets: Res<AssetServer>,
    tables: Res<Assets<TextTable>>,
    fonts: Res<Assets<SpriteSheet>>,
    mut lines: Query<(&QuoteLine, &mut BitmapText)>,
) {
    let (Some(session), Some(mut source)) = (session, source) else { return };
    let Some(draft) = session.mode.draft() else { return };
    let key = (draft.gender(), draft.personality, draft.voice);
    if source.key != Some(key) {
        let stem = crate::presentation::voice::pc_voice_stem(key.0, key.1, key.2);
        source.table = assets.load(format!("retail://data/quotes/pcs/{stem}.msg"));
        source.key = Some(key);
    }
    let quote = tables.get(&source.table).and_then(|table| table.get(0)).unwrap_or("").trim_end();
    for (line, mut text) in &mut lines {
        let value = if quote.is_empty() {
            String::new()
        } else if let Some(font) = fonts.get(&text.font) {
            let wrapped = crate::presentation::notices::wrap(&format!("\"{quote}\""), 224, |s| {
                font.text_width(s)
            });
            wrapped.get(line.0).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        if text.value != value {
            text.value = value;
        }
    }
}

#[derive(Component)]
pub(super) struct Summary {
    previous_focus: Option<Entity>,
    voice_path: String,
    timer: Timer,
    started: bool,
}

fn open_summary(
    _: On<Activate>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    session: Single<&Creation>,
    page: Single<Entity, With<QuoteSource>>,
    existing: Query<(), With<Summary>>,
    mut focus: ResMut<InputFocus>,
) {
    if !existing.is_empty() {
        return;
    }
    let Some(draft) = session.mode.draft() else { return };
    let stem =
        crate::presentation::voice::pc_voice_stem(draft.gender(), draft.personality, draft.voice);
    let previous_focus = focus.get();
    let dialog = commands
        .spawn((
            Summary {
                previous_focus,
                voice_path: crate::presentation::voice::pc_voice_path(&stem, 0),
                timer: Timer::from_seconds(0.75, TimerMode::Once),
                started: false,
            },
            absolute(Vec2::ZERO, Vec2::new(640., 480.)),
            BackgroundColor(Color::srgba(0., 0., 0., 0.5)),
            GlobalZIndex(100),
            ModalDialog,
            TabGroup::modal(),
            Pickable::default(),
            ChildOf(*page),
        ))
        .observe(close_summary_on_pointer)
        .observe(close_summary_on_key)
        .id();
    focus.set(dialog, FocusCause::Navigated);
    let panel = commands
        .spawn((
            absolute(Vec2::new(135., 192.), Vec2::new(369., 96.)),
            Pickable::IGNORE,
            ChildOf(dialog),
        ))
        .id();
    spawn_picture(
        &mut commands,
        panel,
        assets.load("retail://data/dialogs/popup_quote.sti"),
        0,
        Vec2::ZERO,
    );
    spawn_picture(
        &mut commands,
        panel,
        assets.load(format!("retail://data/portraits/medium/m{}.sti", draft.portrait)),
        0,
        Vec2::new(11., 11.),
    );
    let font: Handle<SpriteSheet> = assets.load("retail://data/fonts/wiz_text_font.sti");
    for line in 0..4 {
        commands.spawn((
            QuoteLine(line),
            BitmapText {
                font: font.clone(),
                at: Vec2::new(121., 25. + line as f32 * 11.),
                width: 224.,
                justify: bevy::text::Justify::Center,
                ..default()
            },
            ChildOf(panel),
        ));
    }
}

fn close_summary_on_pointer(
    event: On<PointerPress>,
    dialogs: Query<&Summary>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) {
    if let Ok(dialog) = dialogs.get(event.entity) {
        if let Some(previous) = dialog.previous_focus {
            focus.set(previous, FocusCause::Navigated);
        } else {
            focus.clear();
        }
        commands.entity(event.entity).despawn();
    }
}
fn close_summary_on_key(
    event: On<FocusedInput<KeyboardInput>>,
    dialogs: Query<&Summary>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) {
    if event.input.state.is_pressed()
        && matches!(
            event.input.key_code,
            KeyCode::Escape | KeyCode::Enter | KeyCode::Space | KeyCode::Backspace
        )
        && let Ok(dialog) = dialogs.get(event.focused_entity)
    {
        if let Some(previous) = dialog.previous_focus {
            focus.set(previous, FocusCause::Navigated);
        } else {
            focus.clear();
        }
        commands.entity(event.focused_entity).despawn();
    }
}

pub(super) fn play_summary_voice(
    mut dialogs: Query<(Entity, &mut Summary)>,
    time: Res<Time>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    for (entity, mut dialog) in &mut dialogs {
        if !dialog.started && dialog.timer.tick(time.delta()).just_finished() {
            dialog.started = true;
            commands.spawn((
                AudioPlayer::new(assets.load(dialog.voice_path.clone())),
                PlaybackSettings::DESPAWN,
                ChildOf(entity),
            ));
        }
    }
}
