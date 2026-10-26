//! Retail character information dialogs, owned by the review screen.
use super::{InventoryArt, InventoryScreen, absolute};
use crate::content::characters::{Attribute, Skill};
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::characters::{Character, governing};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::{BitmapText, Picture};
use bevy::input_focus::{
    FocusCause, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::picking::events::{PointerClick, PointerScroll};
use bevy::picking::hover::Hovered;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{
    Activate, Button, ModalDialog, SelectedTab, SetSliderValue, Slider, SliderOrientation,
    SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue, SliderValueChange,
    slider_self_update,
};
use bevy_enhanced_input::prelude::*;

#[derive(Clone, Copy)]
pub(super) enum Subject {
    Skill(Skill),
    Attribute(Attribute),
    Secondary(usize),
}

#[derive(Component)]
pub(super) struct InfoTarget {
    pub subject: Subject,
    pub root: Entity,
}

#[derive(Component, Clone, Default)]
pub(super) struct InfoDialog {
    line_height: f32,
}
#[derive(Component)]
pub(super) struct Lines;
#[derive(Component)]
pub(super) struct Thumb;
#[derive(Component)]
pub(super) struct InfoScroll;
#[derive(Component)]
pub(super) struct CloseButton;
#[derive(Component)]
pub(super) struct ScrollArrow(bool);
#[derive(InputAction)]
#[action_output(bool)]
struct Close;
#[derive(InputAction)]
#[action_output(bool)]
struct BlockInventoryKey;

// Retail places the Martial Arts description before the four preceding skills.
const DETAILS: [usize; 41] = [
    0x677, 0x678, 0x679, 0x67a, 0x67b, 0x67c, 0x67d, 0x67e, 0x67f, 0x680, 0x682, 0x683, 0x684,
    0x685, 0x681, 0x686, 0x687, 0x688, 0x689, 0x68a, 0x68b, 0x68c, 0x68d, 0x68e, 0x68f, 0x690,
    0x691, 0x692, 0x693, 0x694, 0x695, 0x696, 0x697, 0x698, 0x699, 0x69a, 0x69b, 0x69c, 0x69d,
    0x69e, 0x69f,
];

pub(super) fn open_info(
    click: On<PointerClick>,
    targets: Query<&InfoTarget>,
    screen: Single<&InventoryScreen>,
    profiles: Query<&Character>,
    roster: Res<PartyRoster>,
    existing: Query<(), With<InfoDialog>>,
    art: Res<InventoryArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    mut commands: Commands,
    mut focus: Option<ResMut<InputFocus>>,
) {
    if click.button != PointerButton::Secondary || !existing.is_empty() {
        return;
    }
    let (Ok(target), Ok(profile), Some(font), Some(strings)) = (
        targets.get(click.entity),
        profiles.get(screen.member),
        sheets.get(&art.font),
        tables.get(&art.strings),
    ) else {
        return;
    };
    let string = |id| strings.get(id).unwrap_or("");
    let (title_id, detail_id) = match target.subject {
        Subject::Skill(skill) => (0x2e2 + skill as usize, DETAILS[skill as usize]),
        Subject::Attribute(attribute) => (0x274 + attribute as usize, 0x6a0 + attribute as usize),
        Subject::Secondary(index) => (0x6ac + index, 0x6a7 + index),
    };
    let mut entries: Vec<(String, Option<Handle<SpriteSheet>>)> = Vec::new();
    let prefix = format!("{}: ", string(0x155));
    let description = format!("{}{}", prefix, string(detail_id));
    for (i, line) in
        crate::presentation::notices::wrap(&description, 265, |line| font.text_width(line))
            .into_iter()
            .enumerate()
    {
        if i == 0 && line.starts_with(&prefix) {
            entries.push((prefix.clone(), Some(art.bronze.clone())));
            entries.push((line[prefix.len()..].to_owned(), None));
        } else {
            entries.push((line, None));
        }
    }
    entries.push((String::new(), None));
    if let Subject::Skill(skill) = target.subject {
        let (first_attribute, second_attribute) = governing(skill);
        let level = profile.skill(skill);
        let best = level > 0
            && roster.0.iter().all(|member| {
                profiles.get(*member).map_or(true, |other| other.skill(skill) <= level)
            });
        entries.push((format!("{}: ", string(0x156)), Some(art.bronze.clone())));
        entries.push((string(0x274 + first_attribute as usize).to_owned(), None));
        if first_attribute != second_attribute {
            entries.push((string(0x274 + second_attribute as usize).to_owned(), None));
        }
        for (show, id, palette) in [
            (best, 0x157, &art.yellow),
            (!profile.skill_available(skill), 0x158, &art.gray),
            (profile.bonus_skill() == skill, 0x159, &art.blue),
        ] {
            if show {
                entries.push((String::new(), None));
                let value = string(id).replace("%d", "25").replace("%u", "25").replace("%%", "%");
                for line in
                    crate::presentation::notices::wrap(&value, 265, |line| font.text_width(line))
                {
                    entries.push((line, Some(palette.clone())));
                }
            }
        }
    }
    let line_height = font
        .frames
        .first()
        .map_or(12.0, |frame| (frame.height as i32 + frame.offset[1] as i32).max(1) as f32);
    // The first two labels share a line (the differently coloured prefix).
    let count = entries.len().saturating_sub(1);
    let maximum = count.saturating_sub((185.0 / line_height).floor() as usize);
    let root = commands.spawn_scene(bsn! {
        InfoDialog { line_height: {line_height} }
        ContextPriority::<InfoDialog>::new(2)
        Actions<InfoDialog> [
            Action::<Close>::new() ActionSettings { consume_input: true } Bindings [ Binding(KeyCode::Escape) ]
            -- Action::<BlockInventoryKey>::new() ActionSettings { consume_input: true } Bindings [ Binding(KeyCode::KeyI) ]
        ]
        on(close_keyboard)
    }).insert((absolute(Vec2::ZERO, Vec2::new(640.0, 480.0)), GlobalZIndex(20), TabGroup::modal(), ModalDialog, Pickable::default(), ChildOf(target.root))).observe(scroll).observe(dismiss_right).id();
    image(&mut commands, root, art.info_dialog.clone(), 0, Vec2::new(156.0, 105.0));
    text(
        &mut commands,
        root,
        art.font.clone(),
        None,
        string(title_id).to_owned(),
        Vec2::new(170.0, 122.0),
        274.0,
    );
    let viewport = commands
        .spawn((
            Node {
                overflow: Overflow::clip(),
                ..absolute(Vec2::new(173.0, 143.0), Vec2::new(270.0, 185.0))
            },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    let lines = commands
        .spawn((
            Lines,
            absolute(Vec2::ZERO, Vec2::new(270.0, count as f32 * line_height)),
            Pickable::IGNORE,
            ChildOf(viewport),
        ))
        .id();
    for (i, (value, palette)) in entries.into_iter().enumerate() {
        let (x, y) = if i == 1 {
            (font.text_width(&prefix) as f32, 0.0)
        } else {
            (0.0, i.saturating_sub(usize::from(i > 0)) as f32 * line_height)
        };
        text(&mut commands, lines, art.font.clone(), palette, value, Vec2::new(x, y), 0.0);
    }
    image(&mut commands, root, art.info_dialog.clone(), 1, Vec2::new(455.0, 143.0));
    for (down, frame, y) in [(false, 0, 143.0), (true, 8, 314.0)] {
        let at = Vec2::new(456.0, y);
        commands
            .spawn((
                Button,
                TabIndex(0),
                Hovered::default(),
                ScrollArrow(down),
                Picture { sheet: art.scroll.clone(), frame, at, bounds: None },
                absolute(at, Vec2::splat(16.0)),
                ChildOf(root),
            ))
            .observe(
                move |_: On<Activate>,
                      slider: Single<Entity, With<InfoScroll>>,
                      mut commands: Commands| {
                    commands.trigger(SetSliderValue {
                        entity: *slider,
                        change: SliderValueChange::RelativeStep(if down { 1.0 } else { -1.0 }),
                    });
                },
            );
    }
    let track = commands
        .spawn((
            InfoScroll,
            Slider { orientation: SliderOrientation::Vertical, ..default() },
            SliderRange::new(0.0, maximum as f32),
            SliderStep(1.0),
            SliderPrecision(0),
            absolute(Vec2::new(456.0, 159.0), Vec2::new(16.0, 155.0)),
            Pickable::default(),
            ChildOf(root),
        ))
        .observe(slider_self_update)
        .observe(super::cancel_slider_drag)
        .id();
    commands.spawn((
        SliderThumb,
        Thumb,
        Hovered::default(),
        Picture { sheet: art.scroll.clone(), frame: 4, at: Vec2::ZERO, bounds: None },
        absolute(Vec2::ZERO, Vec2::new(16.0, 10.0)),
        Pickable::default(),
        ChildOf(track),
    ));
    let at = Vec2::new(438.0, 335.0);
    let close_button = commands
        .spawn((
            Button,
            TabIndex(0),
            Hovered::default(),
            CloseButton,
            Picture { sheet: art.info_buttons.clone(), frame: 0, at, bounds: None },
            absolute(at, Vec2::splat(30.0)),
            ChildOf(root),
        ))
        .observe(
            move |_: On<Activate>,
                  mut commands: Commands,
                  selected: Option<Single<&SelectedTab, With<super::review::PageTabs>>>,
                  mut focus: Option<ResMut<InputFocus>>| {
                restore_focus(selected, focus.as_deref_mut());
                commands.entity(root).despawn();
            },
        )
        .id();
    if let Some(focus) = focus.as_mut() {
        focus.set(close_button, FocusCause::Navigated);
    }
}

fn image(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<SpriteSheet>,
    frame: usize,
    at: Vec2,
) {
    commands.spawn((
        Picture { sheet, frame, at, bounds: None },
        Node { position_type: PositionType::Absolute, ..default() },
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
fn text(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    palette: Option<Handle<SpriteSheet>>,
    value: String,
    at: Vec2,
    width: f32,
) {
    commands.spawn((
        BitmapText {
            font,
            palette,
            value,
            at,
            width,
            justify: if width > 0.0 {
                bevy::text::Justify::Center
            } else {
                bevy::text::Justify::Left
            },
        },
        Node { position_type: PositionType::Absolute, ..default() },
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
fn scroll(
    event: On<PointerScroll>,
    slider: Single<Entity, With<InfoScroll>>,
    mut commands: Commands,
) {
    if event.y != 0.0 {
        commands.trigger(SetSliderValue {
            entity: *slider,
            change: SliderValueChange::RelativeStep(if event.y < 0.0 { 1.0 } else { -1.0 }),
        });
    }
}
pub(super) fn update_scroll(
    dialog: Option<Single<&InfoDialog>>,
    slider: Option<Single<(Ref<SliderValue>, &SliderRange), With<InfoScroll>>>,
    mut lines: Query<&mut Node, (With<Lines>, Without<Thumb>)>,
    mut thumbs: Query<(&mut Picture, &mut Visibility), With<Thumb>>,
) {
    let (Some(dialog), Some((value, range))) = (dialog, slider.as_deref()) else { return };
    for mut node in &mut lines {
        let top = px(-value.0 * dialog.line_height);
        if node.top != top {
            node.top = top;
        }
    }
    for (mut picture, mut visible) in &mut thumbs {
        visible.set_if_neq(if range.end() > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let y = if range.end() > 0.0 { 145.0 * value.0 / range.end() } else { 0.0 };
        if picture.at.y != y {
            picture.at.y = y;
        }
    }
}

fn restore_focus(
    selected: Option<Single<&SelectedTab, With<super::review::PageTabs>>>,
    focus: Option<&mut InputFocus>,
) {
    if let (Some(selected), Some(focus)) = (selected, focus)
        && let Some(entity) = selected.0
    {
        focus.set(entity, FocusCause::Navigated);
    }
}
fn close_keyboard(
    event: On<Start<Close>>,
    mut commands: Commands,
    selected: Option<Single<&SelectedTab, With<super::review::PageTabs>>>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    restore_focus(selected, focus.as_deref_mut());
    commands.entity(event.context).despawn();
}
fn dismiss_right(
    event: On<PointerClick>,
    dialog: Single<Entity, With<InfoDialog>>,
    mut commands: Commands,
    selected: Option<Single<&SelectedTab, With<super::review::PageTabs>>>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    if event.button == PointerButton::Secondary {
        restore_focus(selected, focus.as_deref_mut());
        commands.entity(*dialog).despawn();
    }
}

pub(super) fn style_close(
    mut buttons: Query<(&Hovered, Has<Pressed>, &mut Picture), With<CloseButton>>,
) {
    for (hover, pressed, mut picture) in &mut buttons {
        let frame = if pressed {
            4
        } else if hover.get() {
            1
        } else {
            0
        };
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
}

pub(super) fn style_arrows(
    slider: Option<Single<&SliderRange, With<InfoScroll>>>,
    mut arrows: Query<(&ScrollArrow, &Hovered, Has<Pressed>, &mut Picture)>,
) {
    let Some(slider) = slider else { return };
    for (arrow, hover, pressed, mut picture) in &mut arrows {
        let base = if arrow.0 { 8 } else { 0 };
        let frame = base
            + if slider.end() == 0.0 {
                3
            } else if pressed {
                2
            } else if hover.get() {
                1
            } else {
                0
            };
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
}

/// Pointer-focus handling can clear the clicked non-focusable skill row;
/// initialize after the popup and its controls have been spawned.
pub(super) fn initialize_focus(
    buttons: Query<Entity, Added<CloseButton>>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    if let (Ok(button), Some(focus)) = (buttons.single(), focus.as_mut()) {
        focus.set(button, FocusCause::Navigated);
    }
}
