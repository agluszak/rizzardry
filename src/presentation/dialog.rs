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
#[derive(Component, Clone)]
#[relationship(relationship_target = QuestionView)]
pub struct QuestionDialog(pub Entity);

#[derive(Component)]
#[relationship_target(relationship = QuestionDialog, linked_spawn)]
pub struct QuestionView(Entity);

#[derive(InputAction)]
#[action_output(bool)]
struct Accept;

#[derive(InputAction)]
#[action_output(bool)]
struct Decline;

/// Show the question when one is asked, once its art has loaded.
pub fn show_travel_question(
    mut commands: Commands,
    questions: Query<(Entity, &TravelQuestion), Without<QuestionView>>,
    art: Res<DialogArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Ok((question_id, question)) = questions.single() else { return };
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
    let dialog = QuestionDialog(question_id);
    commands.spawn_scene(bsn! {
        @centred()
        dialog
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ending_a_question_removes_its_dialog_and_controls() {
        let mut world = World::new();
        let level = world.spawn_empty().id();
        let question =
            world.spawn((TravelQuestion { entrance: "MO204".into() }, ChildOf(level))).id();
        let dialog = world.spawn(QuestionDialog(question)).id();
        let button = world.spawn(ChildOf(dialog)).id();
        assert_eq!(world.get::<QuestionView>(question).unwrap().0, dialog);
        world.despawn(level);
        assert!(world.get_entity(question).is_err());
        assert!(world.get_entity(dialog).is_err());
        assert!(world.get_entity(button).is_err());
    }

    #[test]
    fn rebuilding_a_dialog_keeps_the_question_and_allows_a_new_view() {
        let mut world = World::new();
        let question = world.spawn(TravelQuestion { entrance: "MO204".into() }).id();
        let dialog = world.spawn(QuestionDialog(question)).id();
        world.despawn(dialog);
        world.flush();
        assert!(world.get::<TravelQuestion>(question).is_some());
        assert!(world.get::<QuestionView>(question).is_none());
        let replacement = world.spawn(QuestionDialog(question)).id();
        assert_eq!(world.get::<QuestionView>(question).unwrap().0, replacement);
    }
}
