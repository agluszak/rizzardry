//! A settings draft; Apply persists before changing the live owners.
use super::screen_art::{BitmapText, absolute};
use super::settings::{Preferences, SavedSettings, SettingsFile};
use crate::game::{combat::CombatSettings, difficulty::Difficulty};
use crate::presentation::screen::Screen;
use bevy::input_focus::{
    FocusCause, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};
use bevy_enhanced_input::prelude::*;
#[derive(Component, Clone)]
struct OptionsView {
    draft: SavedSettings,
    previous_focus: Option<Entity>,
    error: String,
}
#[derive(Component, Clone, Copy)]
enum Choice {
    Difficulty,
    Combat,
    VolumeDown,
    VolumeUp,
    Invert,
    Fullscreen,
    Apply,
    Cancel,
}
#[derive(Component)]
struct Status;
#[derive(InputAction)]
#[action_output(bool)]
struct Close;
#[derive(Message)]
pub struct OpenOptions;
pub fn plugin(app: &mut App) {
    app.init_resource::<Preferences>()
        .init_resource::<SettingsFile>()
        .add_input_context::<OptionsView>()
        .add_message::<OpenOptions>()
        .add_systems(Startup, super::settings::load)
        .add_systems(Update, (request, refresh, style_buttons))
        .add_systems(
            Update,
            super::settings::apply
                .before(super::music::play_music)
                .before(super::ambience::play_ambient_sounds),
        )
        .add_systems(OnEnter(Screen::Options), enter)
        .add_observer(activate);
}
fn request(
    mut requests: MessageReader<OpenOptions>,
    mut state: ParamSet<(super::screen::WorldInput, ResMut<NextState<Screen>>)>,
) {
    if requests.read().count() > 0 && state.p0().free() {
        state.p1().set(Screen::Options);
    }
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    assets: &AssetServer,
    at: Vec2,
    width: f32,
    text: &str,
) -> Entity {
    let button = commands
        .spawn((
            Button,
            bevy::picking::hover::Hovered::default(),
            TabIndex(0),
            super::retail_controls::FocusRing,
            absolute(at, Vec2::new(width, 30.)),
            BackgroundColor(Color::srgb(0.15, 0.13, 0.10)),
            Pickable::default(),
            ChildOf(parent),
        ))
        .id();
    commands.spawn((
        BitmapText {
            font: assets.load("retail://data/fonts/opt_detail_font.sti"),
            value: text.into(),
            at: Vec2::new(4., 8.),
            width: width - 8.,
            justify: bevy::text::Justify::Center,
            ..default()
        },
        ChildOf(button),
    ));
    button
}
fn enter(
    mut commands: Commands,
    assets: Res<AssetServer>,
    preferences: Res<Preferences>,
    difficulty: Res<Difficulty>,
    combat: Res<CombatSettings>,
    mut focus: ResMut<InputFocus>,
) {
    let view = OptionsView {
        draft: SavedSettings {
            preferences: preferences.clone(),
            difficulty: *difficulty,
            continuous: combat.continuous,
        },
        previous_focus: focus.get(),
        error: String::new(),
    };
    let root=commands.spawn_scene(bsn! {
        ContextPriority::<OptionsView>::new(10)
        Node {width:percent(100),height:percent(100),justify_content:JustifyContent::Center,align_items:AlignItems::Center}
        GlobalZIndex(60) TabGroup::modal() DespawnOnExit::<Screen>(Screen::Options)
        Actions<OptionsView> [Action::<Close>::new() ActionSettings {consume_input:true,require_reset:true} Bindings [Binding(KeyCode::Escape)]]
        on(|_:On<Start<Close>>,view:Single<&OptionsView>,mut next:ResMut<NextState<Screen>>,mut menu:Option<ResMut<NextState<super::frontend::MenuPage>>>,mut focus:ResMut<InputFocus>|{close(&view,&mut next,menu.as_deref_mut(),&mut focus);})
    }).insert(view).id();
    let panel = commands
        .spawn((
            Node { width: px(640), height: px(480), ..default() },
            BackgroundColor(Color::srgb(0.035, 0.045, 0.06)),
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        Status,
        BitmapText {
            font: assets.load("retail://data/fonts/opt_detail_font.sti"),
            at: Vec2::new(32., 20.),
            width: 576.,
            ..default()
        },
        ChildOf(panel),
    ));
    for (i, (label, choice)) in [
        ("Difficulty", Choice::Difficulty),
        ("Combat mode", Choice::Combat),
        ("Decrease volume", Choice::VolumeDown),
        ("Increase volume", Choice::VolumeUp),
        ("Invert mouse Y", Choice::Invert),
        ("Fullscreen", Choice::Fullscreen),
        ("Apply", Choice::Apply),
        ("Cancel", Choice::Cancel),
    ]
    .into_iter()
    .enumerate()
    {
        let e = button(
            &mut commands,
            panel,
            &assets,
            Vec2::new(200., 115. + i as f32 * 40.),
            240.,
            label,
        );
        commands.entity(e).insert(choice);
        if i == 0 {
            focus.set(e, FocusCause::Navigated);
        }
    }
}
fn close(
    view: &OptionsView,
    next: &mut NextState<Screen>,
    menu: Option<&mut NextState<super::frontend::MenuPage>>,
    focus: &mut InputFocus,
) {
    next.set(Screen::World);
    if let Some(menu) = menu {
        menu.set(super::frontend::MenuPage::Main);
    }
    if let Some(e) = view.previous_focus {
        focus.set(e, FocusCause::Navigated);
    } else {
        focus.clear();
    }
}
fn activate(
    event: On<Activate>,
    choices: Query<&Choice>,
    mut view: Option<Single<&mut OptionsView>>,
    file: Res<SettingsFile>,
    mut preferences: ResMut<Preferences>,
    mut difficulty: ResMut<Difficulty>,
    mut combat: ResMut<CombatSettings>,
    mut next: ResMut<NextState<Screen>>,
    mut menu: Option<ResMut<NextState<super::frontend::MenuPage>>>,
    mut focus: ResMut<InputFocus>,
) {
    let (Ok(choice), Some(view)) = (choices.get(event.entity), view.as_mut()) else { return };
    match choice {
        Choice::Difficulty => {
            view.draft.difficulty = match view.draft.difficulty {
                Difficulty::Novice => Difficulty::Normal,
                Difficulty::Normal => Difficulty::Expert,
                Difficulty::Expert => Difficulty::Novice,
            }
        }
        Choice::Combat => view.draft.continuous = !view.draft.continuous,
        Choice::VolumeDown => {
            view.draft.preferences.volume = view.draft.preferences.volume.saturating_sub(10)
        }
        Choice::VolumeUp => {
            view.draft.preferences.volume = (view.draft.preferences.volume + 10).min(100)
        }
        Choice::Invert => {
            view.draft.preferences.invert_mouse_y = !view.draft.preferences.invert_mouse_y
        }
        Choice::Fullscreen => {
            view.draft.preferences.fullscreen = !view.draft.preferences.fullscreen
        }
        Choice::Cancel => close(view, &mut next, menu.as_deref_mut(), &mut focus),
        Choice::Apply => match view.draft.save(&file.0) {
            Ok(()) => {
                *preferences = view.draft.preferences.clone();
                *difficulty = view.draft.difficulty;
                combat.continuous = view.draft.continuous;
                close(view, &mut next, menu.as_deref_mut(), &mut focus);
            }
            Err(error) => {
                warn!("Cannot save settings: {error}");
                view.error = "Settings could not be saved. Please try again.".into();
            }
        },
    }
}
/// A button's colour follows whether it is pressed or hovered; an unchanged
/// colour is left untouched.
fn style_buttons(
    mut buttons: Query<
        (&bevy::picking::hover::Hovered, Has<bevy::ui::Pressed>, &mut BackgroundColor),
        With<Choice>,
    >,
) {
    for (hovered, pressed, mut background) in &mut buttons {
        background.set_if_neq(BackgroundColor(if pressed {
            Color::srgb(0.30, 0.24, 0.15)
        } else if hovered.get() {
            Color::srgb(0.23, 0.20, 0.14)
        } else {
            Color::srgb(0.15, 0.13, 0.10)
        }));
    }
}

/// The settings shown follow the draft, redrawn only when it changes.
fn refresh(
    view: Option<Single<Ref<OptionsView>>>,
    mut status: Query<&mut BitmapText, With<Status>>,
) {
    let Some(view) = view else { return };
    for mut text in &mut status {
        if !view.is_changed() && !text.is_added() {
            continue;
        }
        let value = format!(
            "Options\nDifficulty: {:?}    Combat: {}    Volume: {}%\nInvert mouse Y: {}    Fullscreen: {}\n{}",
            view.draft.difficulty,
            if view.draft.continuous { "Continuous" } else { "Phased" },
            view.draft.preferences.volume,
            if view.draft.preferences.invert_mouse_y { "On" } else { "Off" },
            if view.draft.preferences.fullscreen { "On" } else { "Off" },
            view.error
        );
        if text.value != value {
            text.value = value;
        }
    }
}
