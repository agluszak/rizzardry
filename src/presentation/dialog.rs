//! Retail's "Proceed to <level>?" question before a level change, in native
//! Bevy UI with the retail message box buttons and text font.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::levels::LevelInfo;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::world::level::{TravelAnswer, TravelQuestion};
use crate::presentation::screen_art::{centred, hover_art};

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

/// The travel question on screen, with its keys: Enter accepts and Escape
/// declines, like its buttons. They come before every other screen's keys,
/// which do not see them while it is up.
#[derive(Component, Clone, Default)]
pub struct QuestionDialog;

#[derive(Component, Clone, FromTemplate)]
pub struct QuestionSession(pub Entity);

#[derive(InputAction)]
#[action_output(bool)]
struct Accept;

#[derive(InputAction)]
#[action_output(bool)]
struct Decline;

/// Show the question when one is asked, once its art has loaded.
pub fn show_travel_question(
    mut commands: Commands,
    questions: Query<(Entity, &TravelQuestion)>,
    shown: Query<&QuestionSession>,
    art: Res<DialogArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Ok((question_id, question)) = questions.single() else { return };
    if shown.iter().any(|shown| shown.0 == question_id) {
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
    let (width, height) = (label.width(), label.height());
    let label = images.add(label);
    let buttons: Vec<_> = [(ok, true), (cancel, false)]
        .into_iter()
        .filter_map(|(sheet, accept)| {
            let frame = sheet.frames.first()?;
            let (width, height) = (frame.width, frame.height);
            Some(bsn! {
                Node { width: px(width), height: px(height) }
                ImageNode
                @hover_art(sheet, 0, if sheet.images.len() > 1 { 1 } else { 0 })
                on(move |_: On<PointerClick>, mut answers: MessageWriter<TravelAnswer>| {
                    answers.write(TravelAnswer { question: question_id, accept });
                })
            })
        })
        .collect();
    commands.spawn_scene(bsn! {
        @centred()
        QuestionDialog
        QuestionSession({question_id})
        ContextPriority::<QuestionDialog>::new(1)
        Actions<QuestionDialog> [
            Action::<Accept>::new()
            ActionSettings { consume_input: true }
            Bindings [ Binding(KeyCode::Enter) ]
            --
            Action::<Decline>::new()
            ActionSettings { consume_input: true }
            Bindings [ Binding(KeyCode::Escape) ]
        ]
        on(move |_: On<Start<Accept>>, mut answers: MessageWriter<TravelAnswer>| {
            answers.write(TravelAnswer { question: question_id, accept: true });
        })
        on(move |_: On<Start<Decline>>, mut answers: MessageWriter<TravelAnswer>| {
            answers.write(TravelAnswer { question: question_id, accept: false });
        })
        Children [
            Node {
                width: px(PANEL.x),
                height: px(PANEL.y),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceEvenly,
                align_items: AlignItems::Center,
                border: UiRect::all(px(1)),
            }
            BackgroundColor(Color::srgba(0.05, 0.04, 0.03, 0.92))
            BorderColor::all(Color::srgb(0.55, 0.45, 0.3))
            Children [
                Node { width: px(width), height: px(height) }
                ImageNode { image: {label} }
                Pickable::IGNORE
                --
                Node { column_gap: px(24) }
                Children [ {buttons} ]
            ]
        ]
    });
}

/// A dialog is a view of its gameplay session, including cancellation by travel.
pub fn close_ended_questions(
    mut commands: Commands,
    questions: Query<(), With<TravelQuestion>>,
    dialogs: Query<(Entity, &QuestionSession)>,
) {
    for (entity, dialog) in &dialogs {
        if !questions.contains(dialog.0) {
            commands.entity(entity).despawn();
        }
    }
}
