//! Retail's "Proceed to <level>?" question before a level change, in native
//! Bevy UI with the retail message box buttons and text font.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::levels::LevelInfo;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::party::body::Exploration;
use crate::game::world::level::{Travel, TravelQuestion};

/// "Proceed to" in the game string table.
const PROCEED_TO: usize = 0x799;
const PANEL: Vec2 = Vec2::new(320.0, 110.0);

#[derive(Resource)]
pub struct DialogArt {
    ok: Handle<SpriteSheet>,
    cancel: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    strings: Handle<TextTable>,
}

pub fn load_dialog_art(mut commands: Commands, assets: Res<AssetServer>) {
    let art = |name: &str| assets.load(format!("retail://data/message box/{name}"));
    commands.insert_resource(DialogArt {
        ok: art("ok.sti"),
        cancel: art("cancel.sti"),
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        strings: assets.load("retail://data/strings/stringdata.dat"),
    });
}

#[derive(Component)]
pub struct QuestionDialog;

/// A dialog button, drawn from its sheet: frame 0, frame 1 while hovered.
#[derive(Component)]
pub struct DialogButton {
    sheet: Handle<SpriteSheet>,
    accept: bool,
}

/// Show the question when one is asked, once its art has loaded.
pub fn show_travel_question(
    mut commands: Commands,
    question: Option<Res<TravelQuestion>>,
    shown: Query<(), With<QuestionDialog>>,
    art: Res<DialogArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    mut images: ResMut<Assets<Image>>,
    exploration: Query<Entity, With<Exploration>>,
) {
    let Some(question) = question else { return };
    if !shown.is_empty() {
        return;
    }
    let (Some(ok), Some(cancel), Some(font), Some(strings)) = (
        sheets.get(&art.ok),
        sheets.get(&art.cancel),
        sheets.get(&art.font),
        tables.get(&art.strings),
    ) else {
        return;
    };
    let level = LevelInfo::for_entrance(&question.entrance)
        .and_then(LevelInfo::name_string)
        .and_then(|name| strings.get(name))
        .unwrap_or(&question.entrance);
    let text = format!("{} {}?", strings.get(PROCEED_TO).unwrap_or("Proceed to"), level.trim());
    let label = font.text_image(&text);
    let label_size = label.size_f32();
    for controls in &exploration {
        commands.entity(controls).insert(ContextActivity::<Exploration>::INACTIVE);
    }
    let screen = commands
        .spawn((
            QuestionDialog,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    let panel = commands
        .spawn((
            Node {
                width: Val::Px(PANEL.x),
                height: Val::Px(PANEL.y),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceEvenly,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.04, 0.03, 0.92)),
            BorderColor::from(Color::srgb(0.55, 0.45, 0.3)),
            ChildOf(screen),
        ))
        .id();
    commands.spawn((
        ImageNode::new(images.add(label)),
        Node { width: Val::Px(label_size.x), height: Val::Px(label_size.y), ..default() },
        Pickable::IGNORE,
        ChildOf(panel),
    ));
    let row =
        commands.spawn((Node { column_gap: Val::Px(24.0), ..default() }, ChildOf(panel))).id();
    for (sheet, handle, accept) in [(ok, &art.ok, true), (cancel, &art.cancel, false)] {
        let (Some(image), Some(frame)) = (sheet.images.first(), sheet.frames.first()) else {
            continue;
        };
        commands
            .spawn((
                DialogButton { sheet: handle.clone(), accept },
                Hovered::default(),
                ImageNode::new(image.clone()),
                Node {
                    width: Val::Px(frame.width as f32),
                    height: Val::Px(frame.height as f32),
                    ..default()
                },
                ChildOf(row),
            ))
            .observe(press_dialog_button);
    }
}

fn press_dialog_button(
    click: On<PointerClick>,
    buttons: Query<&DialogButton>,
    mut answers: MessageWriter<TravelAnswer>,
) {
    if let Ok(button) = buttons.get(click.entity) {
        answers.write(TravelAnswer(button.accept));
    }
}

/// The player's answer to the travel question.
#[derive(Message, Debug, Clone, Copy)]
pub struct TravelAnswer(pub bool);

/// Enter accepts and Escape declines, like the buttons.
pub fn answer_with_keys(
    keys: Res<ButtonInput<KeyCode>>,
    question: Option<Res<TravelQuestion>>,
    mut answers: MessageWriter<TravelAnswer>,
) {
    if question.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        answers.write(TravelAnswer(true));
    } else if keys.just_pressed(KeyCode::Escape) {
        answers.write(TravelAnswer(false));
    }
}

/// Travel on yes; on no the party stays where it is (retail places it back
/// at the trigger it stands in, which does not fire again until it leaves).
pub fn settle_travel_question(
    mut commands: Commands,
    mut answers: MessageReader<TravelAnswer>,
    question: Option<Res<TravelQuestion>>,
    dialogs: Query<Entity, With<QuestionDialog>>,
    exploration: Query<Entity, With<Exploration>>,
    mut travel: MessageWriter<Travel>,
) {
    let (Some(answer), Some(question)) = (answers.read().last(), question) else { return };
    if answer.0 {
        travel.write(Travel { entrance: question.entrance.clone() });
    }
    commands.remove_resource::<TravelQuestion>();
    for dialog in &dialogs {
        commands.entity(dialog).despawn();
    }
    for controls in &exploration {
        commands.entity(controls).insert(ContextActivity::<Exploration>::ACTIVE);
    }
}

pub fn paint_dialog_buttons(
    mut buttons: Query<(&Hovered, &DialogButton, &mut ImageNode)>,
    sheets: Res<Assets<SpriteSheet>>,
) {
    for (hovered, button, mut image) in &mut buttons {
        let Some(sheet) = sheets.get(&button.sheet) else { continue };
        let frame = if hovered.get() && sheet.images.len() > 1 { 1 } else { 0 };
        if image.image != sheet.images[frame] {
            image.image = sheet.images[frame].clone();
        }
    }
}
