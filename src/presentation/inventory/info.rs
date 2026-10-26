//! Retail skill information, owned by the character-review screen.
use super::{InventoryArt, InventoryScreen, absolute};
use crate::content::characters::Skill;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::characters::{Character, governing};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen_art::{BitmapText, Picture};
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::picking::events::{PointerClick, PointerDrag, PointerDragStart, PointerScroll};
use bevy::picking::hover::Hovered;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy_enhanced_input::prelude::*;

#[derive(Component)]
pub(super) struct SkillTarget {
    pub skill: Skill,
    pub root: Entity,
}

#[derive(Component, Clone, Default)]
pub(super) struct SkillDialog {
    first: usize,
    maximum: usize,
    line_height: f32,
    drag_start: usize,
}
#[derive(Component)]
pub(super) struct Lines;
#[derive(Component)]
pub(super) struct Thumb;
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

pub(super) fn open_skill(
    click: On<PointerClick>,
    targets: Query<&SkillTarget>,
    screen: Single<&InventoryScreen>,
    profiles: Query<&Character>,
    roster: Res<PartyRoster>,
    existing: Query<(), With<SkillDialog>>,
    art: Res<InventoryArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    mut commands: Commands,
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
    let skill = target.skill;
    let string = |id| strings.get(id).unwrap_or("");
    let (first_attribute, second_attribute) = governing(skill);
    let level = profile.skill(skill);
    let best = level > 0
        && roster
            .0
            .iter()
            .all(|member| profiles.get(*member).map_or(true, |other| other.skill(skill) <= level));
    let mut entries: Vec<(String, Option<Handle<SpriteSheet>>)> = Vec::new();
    let prefix = format!("{}: ", string(0x155));
    let description = format!("{}{}", prefix, string(DETAILS[skill as usize]));
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
    let line_height = font
        .frames
        .first()
        .map_or(12.0, |frame| (frame.height as i32 + frame.offset[1] as i32).max(1) as f32);
    // The first two labels share a line (the differently coloured prefix).
    let count = entries.len().saturating_sub(1);
    let maximum = count.saturating_sub((185.0 / line_height).floor() as usize);
    let root = commands.spawn_scene(bsn! {
        SkillDialog { first: 0, maximum: {maximum}, line_height: {line_height}, drag_start: 0 }
        ContextPriority::<SkillDialog>::new(2)
        Actions<SkillDialog> [
            Action::<Close>::new() ActionSettings { consume_input: true } Bindings [ Binding(KeyCode::Escape) ]
            -- Action::<BlockInventoryKey>::new() ActionSettings { consume_input: true } Bindings [ Binding(KeyCode::KeyI) ]
        ]
        on(|event: On<Start<Close>>, mut commands: Commands| { commands.entity(event.context).despawn(); })
    }).insert((absolute(Vec2::ZERO, Vec2::new(640.0, 480.0)), GlobalZIndex(20), TabGroup::modal(), Pickable::default(), ChildOf(target.root))).observe(scroll).observe(dismiss_right).id();
    image(&mut commands, root, art.info_dialog.clone(), 0, Vec2::new(156.0, 105.0));
    text(
        &mut commands,
        root,
        art.font.clone(),
        None,
        string(0x2e2 + skill as usize).to_owned(),
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
            .observe(move |_: On<Activate>, mut dialog: Single<&mut SkillDialog>| {
                step(&mut dialog, down)
            });
    }
    let at = Vec2::new(456.0, 159.0);
    commands
        .spawn((
            Thumb,
            Button,
            Picture { sheet: art.scroll.clone(), frame: 4, at, bounds: None },
            absolute(at, Vec2::new(16.0, 10.0)),
            ChildOf(root),
        ))
        .observe(start_drag)
        .observe(drag);
    let at = Vec2::new(438.0, 335.0);
    commands
        .spawn((
            Button,
            TabIndex(0),
            Hovered::default(),
            CloseButton,
            Picture { sheet: art.info_buttons.clone(), frame: 0, at, bounds: None },
            absolute(at, Vec2::splat(30.0)),
            ChildOf(root),
        ))
        .observe(move |_: On<Activate>, mut commands: Commands| {
            commands.entity(root).despawn();
        });
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
fn step(dialog: &mut SkillDialog, down: bool) {
    dialog.first =
        if down { (dialog.first + 1).min(dialog.maximum) } else { dialog.first.saturating_sub(1) };
}
fn scroll(event: On<PointerScroll>, mut dialog: Single<&mut SkillDialog>) {
    if event.y != 0.0 {
        step(&mut dialog, event.y < 0.0);
    }
}
fn start_drag(event: On<PointerDragStart>, mut dialog: Single<&mut SkillDialog>) {
    if event.button == PointerButton::Primary {
        dialog.drag_start = dialog.first;
    }
}
fn drag(event: On<PointerDrag>, scale: Res<UiScale>, mut dialog: Single<&mut SkillDialog>) {
    if event.button == PointerButton::Primary && dialog.maximum > 0 {
        let delta = event.distance.y / scale.0 / 145.0 * dialog.maximum as f32;
        dialog.first =
            (dialog.drag_start as f32 + delta).round().clamp(0.0, dialog.maximum as f32) as usize;
    }
}
pub(super) fn update_scroll(
    dialog: Option<Single<Ref<SkillDialog>>>,
    mut lines: Query<&mut Node, (With<Lines>, Without<Thumb>)>,
    mut thumbs: Query<(&mut Picture, &mut Visibility), With<Thumb>>,
) {
    let Some(dialog) = dialog else { return };
    if !dialog.is_changed() {
        return;
    }
    for mut node in &mut lines {
        node.top = px(-(dialog.first as f32) * dialog.line_height);
    }
    for (mut picture, mut visible) in &mut thumbs {
        *visible = if dialog.maximum > 0 { Visibility::Inherited } else { Visibility::Hidden };
        picture.at.y = 159.0
            + if dialog.maximum > 0 {
                145.0 * dialog.first as f32 / dialog.maximum as f32
            } else {
                0.0
            };
    }
}

fn dismiss_right(
    event: On<PointerClick>,
    dialog: Single<Entity, With<SkillDialog>>,
    mut commands: Commands,
) {
    if event.button == PointerButton::Secondary {
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
    dialog: Option<Single<&SkillDialog>>,
    mut arrows: Query<(&ScrollArrow, &Hovered, Has<Pressed>, &mut Picture)>,
) {
    let Some(dialog) = dialog else { return };
    for (arrow, hover, pressed, mut picture) in &mut arrows {
        let base = if arrow.0 { 8 } else { 0 };
        let frame = base
            + if dialog.maximum == 0 {
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
