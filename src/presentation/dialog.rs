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

#[derive(Component)]
pub struct DialogBuilt;

/// Input belongs to the question, not to the availability of its artwork.
pub fn open_travel_question(
    mut commands: Commands,
    questions: Query<Entity, (With<TravelQuestion>, Without<QuestionView>)>,
) {
    for question_id in &questions {
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
        on(move |event: On<Start<Accept>>, built: Query<(), With<DialogBuilt>>, mut answers: MessageWriter<TravelAnswer>| {
            if built.contains(event.context) {
                answers.write(TravelAnswer { question: question_id, accept: true });
            }
        })
        on(move |_: On<Start<Decline>>, mut answers: MessageWriter<TravelAnswer>| {
            answers.write(TravelAnswer { question: question_id, accept: false });
        })
    });
    }
}

/// Attach the visuals when ready; a failed asset must not strand the question.
pub fn show_travel_question(
    mut commands: Commands,
    questions: Query<&TravelQuestion>,
    dialogs: Query<(Entity, &QuestionDialog), Without<DialogBuilt>>,
    assets: Res<AssetServer>,
    mut answers: MessageWriter<TravelAnswer>,
    art: Res<DialogArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Ok((dialog, owner)) = dialogs.single() else { return };
    let question_id = owner.0;
    let Ok(question) = questions.get(question_id) else { return };
    if [&art.ok, &art.cancel, &art.font].into_iter().any(|handle| {
        matches!(assets.get_load_state(handle), Some(bevy::asset::LoadState::Failed(_)))
    }) || matches!(assets.get_load_state(&art.strings), Some(bevy::asset::LoadState::Failed(_)))
    {
        warn!("travel confirmation artwork failed; declining travel");
        answers.write(TravelAnswer { question: question_id, accept: false });
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
    commands.entity(dialog).insert(DialogBuilt).queue_spawn_related_scenes::<Children>(bsn! {

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
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::test_input::tap;

    fn input_app() -> App {
        let mut app = crate::presentation::test_input::input_app();
        app.add_input_context::<QuestionDialog>()
            .add_message::<TravelAnswer>()
            .init_asset::<SpriteSheet>()
            .init_asset::<TextTable>()
            .init_asset::<Image>()
            .insert_resource(DialogArt {
                ok: default(),
                cancel: default(),
                font: default(),
                strings: default(),
            })
            .add_systems(Update, (open_travel_question, show_travel_question).chain());
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn escape_answers_a_question_even_without_artwork() {
        let mut app = input_app();
        let level = app.world_mut().spawn_empty().id();
        let question = app
            .world_mut()
            .spawn((TravelQuestion { entrance: "MO204".into() }, ChildOf(level)))
            .id();
        app.update();
        app.update();
        let dialog = app.world().get::<QuestionView>(question).unwrap().0;
        assert!(app.world().get::<DialogBuilt>(dialog).is_none());
        tap(&mut app, Entity::PLACEHOLDER, KeyCode::Escape);
        let answers: Vec<_> =
            app.world_mut().resource_mut::<Messages<TravelAnswer>>().drain().collect();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].question, question);
        assert!(!answers[0].accept);
        // Leaving the level ends the question, and its dialog goes with it.
        app.world_mut().despawn(level);
        app.update();
        assert!(app.world().get_entity(dialog).is_err());
    }

    #[test]
    fn terminal_asset_failure_declines_the_current_question() {
        let mut app = input_app();
        // No loader exists for this deliberately invalid asset; wait for the real
        // async asset pipeline rather than simulating a failed-state flag.
        let font = app.world().resource::<AssetServer>().load::<SpriteSheet>("missing.invalid");
        app.world_mut().resource_mut::<DialogArt>().font = font;
        let question = app.world_mut().spawn(TravelQuestion { entrance: "MO204".into() }).id();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.update();
            let answers: Vec<_> =
                app.world_mut().resource_mut::<Messages<TravelAnswer>>().drain().collect();
            if let Some(answer) = answers.first() {
                assert_eq!(answer.question, question);
                assert!(!answer.accept);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "asset failure did not resolve travel");
            std::thread::yield_now();
        }
    }
}
