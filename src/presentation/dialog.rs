//! Retail's "Proceed to <level>?" question before a level change, in native
//! Bevy UI with the retail message box buttons and text font.

use bevy::input_focus::{InputFocus, tab_navigation::TabGroup};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::levels::LevelInfo;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::world::level::{TravelAnswer, TravelQuestion};
use crate::presentation::screen_art::{BitmapText, centred};

/// "Proceed to" in the game string table.
const PROCEED_TO: usize = 0x799;
const PANEL: Vec2 = Vec2::new(320.0, 110.0);

#[derive(Component, Clone, Default)]
pub struct DialogButton(bool);

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
        dialog TabGroup::modal() bevy::ui_widgets::ModalDialog
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
        on(move |event: On<Start<Accept>>, built: Query<(), With<DialogBuilt>>, buttons: Query<(), With<DialogButton>>, focus: Option<Res<InputFocus>>, mut answers: MessageWriter<TravelAnswer>| {
            if built.contains(event.context) && !focus.as_ref().and_then(|f| f.get()).is_some_and(|e| buttons.contains(e)) {
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
    let (Some(ok), Some(cancel), Some(_font), Some(strings)) = (
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
    let label_font = art.font.clone();
    let buttons: Vec<_> = [(ok, art.ok.clone(), true), (cancel, art.cancel.clone(), false)]
        .into_iter().filter_map(|(sheet, handle, accept)| {
            sheet.frames.first()?;
            let hover = usize::from(sheet.images.len() > 1);
            Some(bsn! {
                @crate::presentation::retail_controls::image_button(handle, Vec2::ZERO, crate::presentation::retail_controls::ControlSkin::new(0, hover, hover, hover, 0))
                Node { position_type: PositionType::Relative } template(move |_| Ok(DialogButton(accept)))
                on(move |_: On<bevy::ui_widgets::Activate>, mut answers: MessageWriter<TravelAnswer>| {
                    answers.write(TravelAnswer { question: question_id, accept });
                })
            })
        }).collect();
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
                template(move |_| Ok(BitmapText { font: label_font.clone(), value: text.clone(), ..default() }))
                Node { position_type: PositionType::Relative }
                --
                Node { column_gap: px(24) }
                Children [ {buttons} ]
            ]
    });
}

pub fn focus_ready_question(
    buttons: Query<(Entity, &DialogButton), Added<DialogButton>>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    if let Some(focus) = focus.as_mut() {
        for (entity, button) in &buttons {
            if button.0 {
                focus.set(entity, bevy::input_focus::FocusCause::Navigated);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            bevy::input_focus::InputFocusPlugin,
            bevy::input_focus::InputDispatchPlugin,
            bevy::input_focus::tab_navigation::TabNavigationPlugin,
            bevy::ui_widgets::ButtonPlugin,
            bevy::ui_widgets::ModalDialogPlugin,
        ))
        .add_input_context::<QuestionDialog>()
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
        .add_systems(
            Update,
            (open_travel_question, show_travel_question, focus_ready_question).chain(),
        );
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn escape_answers_a_question_even_without_artwork() {
        use bevy::input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
        };
        let mut app = input_app();
        let question = app.world_mut().spawn(TravelQuestion { entrance: "MO204".into() }).id();
        app.update();
        app.update();
        let dialog = app.world().get::<QuestionView>(question).unwrap().0;
        assert!(app.world().get::<DialogBuilt>(dialog).is_none());
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        let answers: Vec<_> =
            app.world_mut().resource_mut::<Messages<TravelAnswer>>().drain().collect();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].question, question);
        assert!(!answers[0].accept);
    }

    #[test]
    fn delayed_art_focuses_ok_and_focused_cancel_owns_enter() {
        use bevy::input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
        };
        let mut app = input_app();
        let window = app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow)).id();
        let question = app.world_mut().spawn(TravelQuestion { entrance: "MO204".into() }).id();
        app.update();
        app.update();
        assert!(app.world_mut().query::<&DialogButton>().iter(app.world()).next().is_none());
        let sheet = app.world_mut().resource_mut::<Assets<SpriteSheet>>().add(SpriteSheet {
            frames: vec![crate::content::ui::Frame {
                offset: [0, 0],
                width: 16,
                height: 16,
                rgba: vec![255; 16 * 16 * 4],
            }],
            images: vec![default()],
            palette: None,
        });
        let strings = app.world_mut().resource_mut::<Assets<TextTable>>().add(TextTable(vec![]));
        app.insert_resource(DialogArt {
            ok: sheet.clone(),
            cancel: sheet.clone(),
            font: sheet,
            strings,
        });
        // Scene spawning is deferred; wait for that actual readiness boundary.
        for _ in 0..4 {
            app.update();
            if app
                .world()
                .resource::<InputFocus>()
                .get()
                .is_some_and(|e| app.world().get::<DialogButton>(e).is_some())
            {
                break;
            }
        }
        let focused =
            app.world().resource::<InputFocus>().get().expect("built question focuses OK");
        assert!(app.world().get::<DialogButton>(focused).unwrap().0);
        let cancel = app
            .world_mut()
            .query::<(Entity, &DialogButton)>()
            .iter(app.world())
            .find_map(|(e, b)| (!b.0).then_some(e))
            .unwrap();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(cancel, bevy::input_focus::FocusCause::Navigated);
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Enter,
            logical_key: Key::Enter,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.update();
        let answers: Vec<_> =
            app.world_mut().resource_mut::<Messages<TravelAnswer>>().drain().collect();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].question, question);
        assert!(!answers[0].accept);
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
