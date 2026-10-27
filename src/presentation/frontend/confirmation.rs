//! Confirm consequential choices in the character creator.
use super::{Creation, Page, Session};
use crate::content::ui::{SpriteSheet, TextTable};
use crate::presentation::retail_controls::ControlSkin;
use crate::presentation::screen_art::{BitmapText, Picture};
use bevy::input_focus::{
    FocusCause, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button, ModalDialog};
use bevy_enhanced_input::prelude::*;

#[derive(Clone)]
enum Purpose {
    Delete { candidate: u32, name: String },
    KeepSpells,
}

#[derive(Component, Clone)]
pub(super) struct ConfirmDialog {
    purpose: Purpose,
    previous_focus: Option<Entity>,
}
#[derive(Component)]
pub(super) struct QuestionText {
    purpose: Purpose,
    line: usize,
    strings: Handle<TextTable>,
}
#[derive(Component)]
pub(super) struct ConfirmationButton(usize);
#[derive(Component)]
pub(super) struct Panel;
#[derive(Component)]
pub(super) struct Measured;
#[derive(Component)]
pub(super) struct BorderPiece {
    sheet: Handle<SpriteSheet>,
    frame: usize,
}
#[derive(Event)]
pub(super) struct Answer {
    dialog: Entity,
    accepted: bool,
}
#[derive(InputAction)]
#[action_output(bool)]
struct Accept;
#[derive(InputAction)]
#[action_output(bool)]
struct Cancel;

pub(super) fn open(
    commands: &mut Commands,
    assets: &AssetServer,
    candidate: u32,
    name: &str,
    focus: &mut InputFocus,
) {
    open_dialog(commands, assets, Purpose::Delete { candidate, name: name.to_owned() }, focus);
}

pub(super) fn confirm_remaining_spells(
    commands: &mut Commands,
    assets: &AssetServer,
    focus: &mut InputFocus,
) {
    open_dialog(commands, assets, Purpose::KeepSpells, focus);
}

fn open_dialog(
    commands: &mut Commands,
    assets: &AssetServer,
    purpose: Purpose,
    focus: &mut InputFocus,
) {
    let previous_focus = focus.get();
    let dialog = commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        GlobalZIndex(100)
        Pickable::default()
        TabGroup::modal()
        ModalDialog
        ContextPriority::<ConfirmDialog>::new(10)
        ContextActivity::<ConfirmDialog>::INACTIVE
        Actions<ConfirmDialog> [
            Action::<Accept>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::Enter) ]
            -- Action::<Cancel>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::Escape) ]
        ]
        on(|event: On<Start<Accept>>, mut commands: Commands| { commands.trigger(Answer { dialog: event.context, accepted: true }); })
        on(|event: On<Start<Cancel>>, mut commands: Commands| { commands.trigger(Answer { dialog: event.context, accepted: false }); })
    }).insert((ConfirmDialog { purpose: purpose.clone(), previous_focus }, DespawnOnExit(Session::Creation))).id();
    let panel = commands
        .spawn((
            Panel,
            Node { width: px(210), height: px(100), overflow: Overflow::clip(), ..default() },
            Pickable::IGNORE,
            ChildOf(dialog),
        ))
        .id();
    commands.spawn((
        BorderPiece { sheet: assets.load("retail://data/dialogs/dialogbackground.sti"), frame: 0 },
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ImageNode {
            image_mode: bevy::ui::widget::NodeImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: 1.,
            },
            ..default()
        },
        Visibility::Hidden,
        Pickable::IGNORE,
        ChildOf(panel),
    ));
    for (path, inset, thickness) in [
        ("retail://data/dialogs/dialogborder.sti", 0., 9.),
        ("retail://data/dialogs/dialogedge.sti", 9., 3.),
    ] {
        let sheet = assets.load(path);
        for frame in 0..8 {
            let mut node = Node { position_type: PositionType::Absolute, ..default() };
            match frame {
                0 | 2 | 5 | 7 => {
                    node.width = px(thickness);
                    node.height = px(thickness);
                    if frame == 0 || frame == 5 {
                        node.left = px(inset);
                    } else {
                        node.right = px(inset);
                    }
                    if frame == 0 || frame == 2 {
                        node.top = px(inset);
                    } else {
                        node.bottom = px(inset);
                    }
                }
                1 | 6 => {
                    node.left = px(inset + thickness);
                    node.right = px(inset + thickness);
                    node.height = px(thickness);
                    if frame == 1 {
                        node.top = px(inset);
                    } else {
                        node.bottom = px(inset);
                    }
                }
                3 | 4 => {
                    node.top = px(inset + thickness);
                    node.bottom = px(inset + thickness);
                    node.width = px(thickness);
                    if frame == 3 {
                        node.left = px(inset);
                    } else {
                        node.right = px(inset);
                    }
                }
                _ => unreachable!(),
            }
            commands.spawn((
                BorderPiece { sheet: sheet.clone(), frame },
                node,
                ImageNode {
                    image_mode: bevy::ui::widget::NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: true,
                        stretch_value: 1.,
                    },
                    ..default()
                },
                Visibility::Hidden,
                Pickable::IGNORE,
                ChildOf(panel),
            ));
        }
    }
    for line in 0..3 {
        commands.spawn((
            QuestionText {
                purpose: purpose.clone(),
                line,
                strings: assets.load("retail://data/strings/stringdata.dat"),
            },
            BitmapText {
                font: assets.load("retail://data/fonts/wiz_text_font.sti"),
                at: Vec2::new(4., 20. + line as f32 * 11.),
                width: 202.,
                justify: bevy::text::Justify::Center,
                ..default()
            },
            ChildOf(panel),
        ));
    }
    let sheet: Handle<SpriteSheet> = assets.load("retail://data/dialogs/dialogconfirmation.sti");
    for (x, frame, accepted) in [(68., 0, true), (117., 4, false)] {
        let button = commands
            .spawn((
                Button,
                TabIndex(0),
                ConfirmationButton(frame),
                ControlSkin::new(frame, frame + 1, frame + 2, frame + 2, frame + 3),
                Picture { sheet: sheet.clone(), frame, at: Vec2::new(x, 61.), bounds: Some(frame) },
                Pickable::default(),
                ChildOf(panel),
            ))
            .observe(move |_: On<Activate>, mut commands: Commands| {
                commands.trigger(Answer { dialog, accepted });
            })
            .id();
        if accepted {
            focus.set(button, FocusCause::Navigated);
        }
    }
}

pub(super) fn resolve(
    answer: On<Answer>,
    dialogs: Query<(&ConfirmDialog, &ContextActivity<ConfirmDialog>)>,
    mut sessions: Query<&mut Creation>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) {
    let Ok((dialog, active)) = dialogs.get(answer.dialog) else { return };
    if !**active {
        return;
    }
    if answer.accepted
        && let Ok(mut session) = sessions.single_mut()
    {
        match &dialog.purpose {
            Purpose::Delete { candidate, .. } => {
                if let Some(index) = session.candidates.iter().position(|(id, _)| id == candidate) {
                    session.candidates.remove(index);
                    session.party.retain(|id| id != candidate);
                    session.selected = index
                        .checked_sub(1)
                        .and_then(|i| session.candidates.get(i))
                        .or_else(|| session.candidates.first())
                        .map(|(id, _)| *id);
                }
            }
            Purpose::KeepSpells => {
                if session.mode.page() == Page::Magic
                    && session
                        .mode
                        .draft()
                        .is_some_and(|draft| draft.spell_remaining() < draft.spell_points_total())
                {
                    session.mode.set_page(Page::Skills);
                }
            }
        }
    }
    commands.entity(answer.dialog).despawn();
    if let Some(previous) = dialog.previous_focus {
        focus.set(previous, FocusCause::Navigated);
    } else {
        focus.clear();
    }
}

pub(super) fn show_text(
    mut commands: Commands,
    tables: Res<Assets<TextTable>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut texts: Query<(Entity, &QuestionText, &mut BitmapText), Without<Measured>>,
    mut panels: Query<&mut Node, (With<Panel>, Without<ConfirmationButton>)>,
    mut buttons: Query<(&ConfirmationButton, &mut Picture), Without<Panel>>,
    mut borders: Query<(&BorderPiece, &mut ImageNode, &mut Visibility), Without<Picture>>,
) {
    let mut size = None;
    for (entity, question, mut text) in &mut texts {
        let Some(table) = tables.get(&question.strings) else { continue };
        let message = match &question.purpose {
            Purpose::Delete { name, .. } => format!(
                "{} {} {}?",
                table.get(0x6d4).unwrap_or("Erase"),
                name,
                table.get(0x6d5).unwrap_or("Permanently")
            ),
            Purpose::KeepSpells => table
                .get(0xc4)
                .unwrap_or("You still have some spell picks. Save them and proceed?")
                .to_owned(),
        };
        if let Some(font) = sheets.get(&text.font) {
            let lines =
                crate::presentation::notices::wrap(&message, 220, |line| font.text_width(line));
            let value = lines.get(question.line).cloned().unwrap_or_default();
            if text.value != value {
                text.value = value;
            }
            let width = lines.iter().map(|line| font.text_width(line)).max().unwrap_or(0) as f32;
            let width = (width * 1.25).max(170.).floor();
            let height = font.text_image("A").height() as f32;
            let text_height = height * lines.len() as f32;
            let panel_height = (text_height + 63.).max(text_height * 7.);
            text.width = width;
            text.at = Vec2::new(
                0.,
                ((panel_height - 33. - text_height) / 2.).floor() + question.line as f32 * height,
            );
            size = Some((width, panel_height));
            commands.entity(entity).insert(Measured);
        }
    }
    if let Some((width, height)) = size {
        for mut panel in &mut panels {
            if panel.width != px(width) {
                panel.width = px(width);
            }
            if panel.height != px(height) {
                panel.height = px(height);
            }
        }
    }
    for (button, mut picture) in &mut buttons {
        if let Some((width, height)) = size {
            let left = if button.0 == 0 {
                ((width - 75.) / 2.).floor()
            } else {
                ((width + 25.) / 2.).floor()
            };
            let top = height - 24. - 15.;
            if picture.at != Vec2::new(left, top) {
                picture.at = Vec2::new(left, top);
            }
        }
    }
    for (border, mut image, mut visibility) in &mut borders {
        if let Some(handle) =
            sheets.get(&border.sheet).and_then(|sheet| sheet.images.get(border.frame))
        {
            if image.image != *handle {
                image.image = handle.clone();
            }
            visibility.set_if_neq(Visibility::Inherited);
        }
    }
}
