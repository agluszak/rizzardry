//! Retail character information dialogs, owned by the review screen.
use super::{InventoryArt, InventoryScreen, absolute};
use crate::content::characters::{Attribute, Skill};
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::characters::{Character, governing};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::spawn_picture as image;
use crate::presentation::screen_art::{BitmapText, Picture};
use bevy::input_focus::{
    FocusCause, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::picking::events::PointerClick;
use bevy::picking::hover::Hovered;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button, ModalDialog, SelectedTab, SliderValue};
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
pub(super) struct InfoScroll;
#[derive(Component)]
pub(super) struct CloseButton;
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
    roster: Single<&PartyRoster>,
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
            && profiles.iter_many(roster.iter()).matched().all(|other| other.skill(skill) <= level);
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
    }).insert((absolute(Vec2::ZERO, Vec2::new(640.0, 480.0)), GlobalZIndex(20), TabGroup::modal(), ModalDialog, Pickable::default(), ChildOf(target.root))).observe(crate::presentation::retail_controls::scroll_wheel).observe(dismiss_right).id();
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
    let track = crate::presentation::retail_controls::step_scrollbar(
        &mut commands,
        root,
        art.scroll.clone(),
        Vec2::new(456.0, 143.0),
        155.0,
        maximum as f32,
    );
    commands.entity(track).insert(InfoScroll);
    commands.entity(root).insert(crate::presentation::retail_controls::ScrollWheel(track));
    let at = Vec2::new(438.0, 335.0);
    let close_button = commands
        .spawn((
            Button,
            TabIndex(0),
            Hovered::default(),
            CloseButton,
            crate::presentation::retail_controls::FocusRing,
            crate::presentation::retail_controls::ControlSkin::new(0, 1, 4, 4, 0),
            Pickable::default(),
            Picture { sheet: art.info_buttons.clone(), frame: 0, at, bounds: Some(0) },
            absolute(at, Vec2::splat(30.0)),
            ChildOf(root),
        ))
        .observe(crate::presentation::retail_controls::primary_press)
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

fn text(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    palette: Option<Handle<SpriteSheet>>,
    value: String,
    at: Vec2,
    width: f32,
) {
    crate::presentation::screen_art::spawn_label(
        commands,
        parent,
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
    );
}
pub(super) fn update_scroll(
    dialog: Option<Single<&InfoDialog>>,
    slider: Option<Single<&SliderValue, With<InfoScroll>>>,
    mut lines: Query<&mut Node, With<Lines>>,
) {
    let (Some(dialog), Some(value)) = (dialog, slider) else { return };
    for mut node in &mut lines {
        let top = px(-value.0 * dialog.line_height);
        if node.top != top {
            node.top = top;
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
