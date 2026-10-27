//! Confirmation for removing a character from the pre-game party list.
use super::{Creation, Session};
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

#[derive(Component, Clone)]
pub(super) struct DeleteDialog {
    candidate: u32,
    previous_focus: Option<Entity>,
}
#[derive(Component)]
pub(super) struct QuestionText {
    name: String,
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
    let previous_focus = focus.get();
    let dialog = commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        GlobalZIndex(100)
        Pickable::default()
        TabGroup::modal()
        ModalDialog
        ContextPriority::<DeleteDialog>::new(10)
        ContextActivity::<DeleteDialog>::INACTIVE
        Actions<DeleteDialog> [
            Action::<Accept>::new() ActionSettings { consume_input: true } Bindings [ Binding(KeyCode::Enter) ]
            -- Action::<Cancel>::new() ActionSettings { consume_input: true } Bindings [ Binding(KeyCode::Escape) ]
        ]
        on(|event: On<Start<Accept>>, mut commands: Commands| { commands.trigger(Answer { dialog: event.context, accepted: true }); })
        on(|event: On<Start<Cancel>>, mut commands: Commands| { commands.trigger(Answer { dialog: event.context, accepted: false }); })
    }).insert((DeleteDialog { candidate, previous_focus }, DespawnOnExit(Session::Creation))).id();
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
    commands.spawn((
        QuestionText {
            name: name.to_owned(),
            strings: assets.load("retail://data/strings/stringdata.dat"),
        },
        BitmapText {
            font: assets.load("retail://data/fonts/wiz_text_font.sti"),
            value: format!("Erase {name} Permanently?"),
            at: Vec2::new(4., 20.),
            width: 202.,
            justify: bevy::text::Justify::Center,
            ..default()
        },
        ChildOf(panel),
    ));
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
    dialogs: Query<(&DeleteDialog, &ContextActivity<DeleteDialog>)>,
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
        && let Some(index) = session.candidates.iter().position(|(id, _)| *id == dialog.candidate)
    {
        session.candidates.remove(index);
        session.party.retain(|id| *id != dialog.candidate);
        session.selected = index
            .checked_sub(1)
            .and_then(|i| session.candidates.get(i))
            .or_else(|| session.candidates.first())
            .map(|(id, _)| *id);
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
        let value = format!(
            "{} {} {}?",
            table.get(0x6d4).unwrap_or("Erase"),
            question.name,
            table.get(0x6d5).unwrap_or("Permanently")
        );
        if text.value != value {
            text.value = value;
        }
        if let Some(font) = sheets.get(&text.font) {
            let image = font.text_image(&text.value);
            let height = image.height() as f32;
            let width = (image.width() as f32 * 1.25).max(170.).floor();
            let panel_height = (height + 63.).max(height * 7.);
            if text.width != width {
                text.width = width;
            }
            let at = Vec2::new(0., ((panel_height - 33. - height) / 2.).floor());
            if text.at != at {
                text.at = at;
            }
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
