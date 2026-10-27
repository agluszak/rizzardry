//! Retained frontend screens. Drafts belong to the creation root, never the campaign.
use super::screen_art::{BitmapText, Picture, absolute, spawn_picture};
use crate::content::characters::{Attribute, Gender, Profession, Skill};
use crate::content::ui::SpriteSheet;
use crate::game::characters::creation::{CharacterDraft, PLAYABLE_RACES};
use crate::game::party::{formation::Formation, roster::PartyRoster};
use crate::game::session::{NewGameDestination, Session, begin_adventure};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{
    InputFocus,
    tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextEditChange};
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::{Activate, Button as UiButton, TextInput};
use bevy_enhanced_input::prelude::Action as InputBindingAction;
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

mod confirmation;
mod credits;
mod party;

pub struct FrontendPlugin;
impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TabNavigationPlugin)
            .add_sub_state::<MenuPage>()
            .add_input_context::<MenuInput>()
            .add_input_context::<credits::CreditsInput>()
            .add_input_context::<confirmation::DeleteDialog>()
            .add_systems(
                PreUpdate,
                arm_confirmation_input
                    .after(bevy::input::InputSystems)
                    .before(EnhancedInputSystems::Update),
            )
            .add_observer(activate)
            .add_observer(confirmation::resolve)
            .add_systems(OnEnter(MenuPage::Main), main_menu)
            .add_systems(OnEnter(MenuPage::Credits), credits::enter)
            .add_systems(OnEnter(Session::Creation), creation)
            .add_systems(
                Update,
                (
                    refresh,
                    show_editor,
                    party::refresh,
                    button_colours,
                    paint_menu,
                    party::paint_labels,
                    confirmation::show_text,
                    credits::populate,
                    credits::scroll,
                )
                    .chain(),
            );
    }
}

// Native buttons also receive raw keyboard events, so the confirmation waits
// for the opening key to be released before accepting either input path.
fn arm_confirmation_input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    confirmation: Query<(Entity, &ContextActivity<confirmation::DeleteDialog>)>,
) {
    if [KeyCode::Enter, KeyCode::Space]
        .into_iter()
        .any(|key| keys.pressed(key) || keys.just_pressed(key))
    {
        return;
    }
    for (entity, active) in &confirmation {
        if !**active {
            commands.entity(entity).insert(ContextActivity::<confirmation::DeleteDialog>::ACTIVE);
        }
    }
}

#[derive(SubStates, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[source(Session = Session::MainMenu)]
enum MenuPage {
    #[default]
    Main,
    Credits,
}

const INK: Color = Color::srgb(0.035, 0.045, 0.06);
const PANEL: Color = Color::srgb(0.075, 0.09, 0.115);
const GOLD: Color = Color::srgb(0.83, 0.68, 0.40);
const PAPER: Color = Color::srgb(0.91, 0.88, 0.80);

#[derive(Clone, Copy, PartialEq, Eq, Component)]
enum Page {
    Party,
    Identity,
    Attributes,
    Skills,
    Review,
}
#[derive(Component)]
struct Creation {
    candidates: Vec<(u32, CharacterDraft)>,
    party: Vec<u32>,
    selected: Option<u32>,
    next_id: u32,
    editing: Option<CharacterDraft>,
    page: Page,
    message: String,
}
#[derive(Component, Clone, Copy)]
enum Action {
    NewGame,
    Credits,
    Quit,
    Menu,
    Add,
    SelectCandidate(Option<u32>),
    ToggleCandidate,
    DeleteCandidate,
    Page(Page),
    Cancel,
    Finish,
    Begin,
    Race(i32),
    Gender,
    Profession(i32),
    Portrait(i32),
    Attribute(Attribute, bool),
    Skill(Skill, bool),
}
#[derive(Component)]
enum Field {
    Race,
    Gender,
    Profession,
    Points,
    Attribute(Attribute),
    Skill(Skill),
    Review,
    Message,
}
#[derive(Component)]
struct PreviewPortrait;
#[derive(Component)]
struct NameInput;
#[derive(Component)]
struct EditorGroup;

fn text(
    commands: &mut Commands,
    parent: Entity,
    at: (f32, f32),
    width: f32,
    size: f32,
    value: impl Into<String>,
) -> Entity {
    let entity = commands
        .spawn((
            absolute(Vec2::new(at.0, at.1), Vec2::new(width, 44.0)),
            Text::new(value),
            TextFont { font_size: size.into(), ..default() },
            TextColor(PAPER),
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(parent).add_child(entity);
    entity
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    at: (f32, f32),
    width: f32,
    label: &str,
    action: Action,
) -> Entity {
    let entity = commands
        .spawn((
            UiButton,
            TabIndex(0),
            Hovered::default(),
            action,
            Node {
                border: UiRect::all(px(1)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..absolute(
                    Vec2::new(at.0, at.1),
                    Vec2::new(
                        width,
                        if width <= 20. {
                            20.
                        } else if width <= 32. {
                            23.
                        } else {
                            27.
                        },
                    ),
                )
            },
            BackgroundColor(PANEL),
            BorderColor::all(GOLD),
        ))
        .observe(crate::presentation::retail_controls::primary_press)
        .id();
    let label = commands
        .spawn((
            Text::new(label),
            TextFont { font_size: 11.0.into(), ..default() },
            TextColor(PAPER),
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(entity).add_child(label);
    commands.entity(parent).add_child(entity);
    label
}
fn root(commands: &mut Commands, state: Session) -> Entity {
    let outer = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(INK),
            GlobalZIndex(50),
            DespawnOnExit(state),
            TabGroup::default(),
        ))
        .id();
    let inner = commands
        .spawn((Node { width: px(640), height: px(480), ..default() }, BackgroundColor(INK)))
        .id();
    commands.entity(outer).add_child(inner);
    if state == Session::MainMenu {
        commands.entity(outer).insert(DespawnOnExit(MenuPage::Main));
    }
    inner
}
#[derive(Component)]
struct MenuItem {
    index: usize,
    normal: Handle<SpriteSheet>,
    highlighted: Handle<SpriteSheet>,
    pressed: Handle<SpriteSheet>,
    unavailable: Handle<SpriteSheet>,
}

#[derive(Component, Clone, Default)]
struct MenuInput;
#[derive(InputAction)]
#[action_output(bool)]
struct MenuUp;
#[derive(InputAction)]
#[action_output(bool)]
struct MenuDown;
#[derive(InputAction)]
#[action_output(bool)]
struct MenuStart;
#[derive(InputAction)]
#[action_output(bool)]
struct MenuExit;

fn main_menu(mut commands: Commands, assets: Res<AssetServer>, mut focus: ResMut<InputFocus>) {
    let root = root(&mut commands, Session::MainMenu);
    let load = |path: &str| assets.load(format!("retail://data/options/{path}.sti"));
    let normal = load("introtext_normal");
    let highlighted = load("introtext_highlight");
    let pressed = load("introtext_depressed");
    let unavailable = load("introtext_unavailable");
    spawn_picture(&mut commands, root, load("intro_bg_text"), 0, Vec2::ZERO);
    // Retail's hit rectangles are narrower than the shared artwork origin.
    for (frame, (x, y, width, action)) in [
        (174., 138., 294., None),
        (140., 187., 362., Some(Action::NewGame)),
        (204., 235., 233., None),
        (239., 284., 165., Some(Action::Credits)),
        (234., 335., 175., None),
        (279., 423., 86., Some(Action::Quit)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut button = commands.spawn((
            UiButton,
            TabIndex(0),
            Hovered::default(),
            absolute(Vec2::new(x, y), Vec2::new(width, 45.)),
            Pickable::default(),
            MenuItem {
                index: frame,
                normal: normal.clone(),
                highlighted: highlighted.clone(),
                pressed: pressed.clone(),
                unavailable: unavailable.clone(),
            },
            ChildOf(root),
        ));
        if let Some(action) = action {
            button.insert(action);
            if frame == 1 {
                focus.set(button.id(), bevy::input_focus::FocusCause::Navigated);
            }
        } else {
            button.insert(InteractionDisabled);
        }
        let button = button.id();
        commands.spawn((
            Picture {
                sheet: if action.is_some() { normal.clone() } else { unavailable.clone() },
                frame,
                at: Vec2::new(152. - x, 0.),
                bounds: None,
            },
            ChildOf(button),
        ));
    }
    commands.spawn((
        BitmapText {
            font: assets.load("retail://data/fonts/smfnt.sti"),
            value: "v1.2.4".into(),
            at: Vec2::new(590., 5.),
            width: 45.,
            justify: bevy::text::Justify::Right,
            ..default()
        },
        ChildOf(root),
    ));
    commands.spawn_scene(bsn! {
        MenuInput
        ChildOf({root})
        ContextPriority::<MenuInput>::new(1)
        Actions<MenuInput> [
            InputBindingAction::<MenuUp>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::ArrowUp) ]
            -- InputBindingAction::<MenuDown>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::ArrowDown) ]
            -- InputBindingAction::<MenuStart>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::KeyS) ]
            -- InputBindingAction::<MenuExit>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::Escape) Binding(KeyCode::KeyX) Binding(KeyCode::KeyE) ]
        ]
        on(menu_up)
        on(menu_down)
        on(|_: On<Start<MenuStart>>, mut next: ResMut<NextState<Session>>| { next.set(Session::Creation); })
        on(|_: On<Start<MenuExit>>, mut exit: MessageWriter<AppExit>| { exit.write(AppExit::Success); })
    });
}

fn move_menu_focus(
    forwards: bool,
    focus: &mut InputFocus,
    buttons: &Query<(Entity, &MenuItem), Without<InteractionDisabled>>,
) {
    let mut entries: Vec<_> = buttons.iter().map(|(entity, item)| (item.index, entity)).collect();
    entries.sort_unstable_by_key(|entry| entry.0);
    if entries.is_empty() {
        return;
    }
    let current = entries.iter().position(|(_, entity)| Some(*entity) == focus.get()).unwrap_or(0);
    let next = if forwards {
        (current + 1) % entries.len()
    } else {
        (current + entries.len() - 1) % entries.len()
    };
    focus.set(entries[next].1, bevy::input_focus::FocusCause::Navigated);
}

fn menu_up(
    _: On<Start<MenuUp>>,
    mut focus: ResMut<InputFocus>,
    buttons: Query<(Entity, &MenuItem), Without<InteractionDisabled>>,
) {
    move_menu_focus(false, &mut focus, &buttons);
}

fn menu_down(
    _: On<Start<MenuDown>>,
    mut focus: ResMut<InputFocus>,
    buttons: Query<(Entity, &MenuItem), Without<InteractionDisabled>>,
) {
    move_menu_focus(true, &mut focus, &buttons);
}

fn paint_menu(
    focus: Res<InputFocus>,
    buttons: Query<(
        Entity,
        &MenuItem,
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        &Children,
    )>,
    mut pictures: Query<&mut Picture>,
) {
    let hovered_button = buttons.iter().find_map(|(entity, _, hovered, _, disabled, _)| {
        (hovered.get() && !disabled).then_some(entity)
    });
    for (entity, item, hovered, pressed, disabled, children) in &buttons {
        let sheet = if disabled {
            &item.unavailable
        } else if pressed {
            &item.pressed
        } else if hovered.get() || (hovered_button.is_none() && focus.get() == Some(entity)) {
            &item.highlighted
        } else {
            &item.normal
        };
        for child in children.iter() {
            if let Ok(mut picture) = pictures.get_mut(child)
                && picture.sheet != *sheet
            {
                picture.sheet = sheet.clone();
            }
        }
    }
}
fn creation(mut commands: Commands, assets: Res<AssetServer>) {
    let root = root(&mut commands, Session::Creation);
    commands.entity(root).insert(Creation {
        candidates: Vec::new(),
        party: Vec::new(),
        selected: None,
        next_id: 0,
        editing: None,
        page: Page::Party,
        message: String::new(),
    });
    party::build(&mut commands, root, &assets);
    let editor = commands
        .spawn((EditorGroup, absolute(Vec2::ZERO, Vec2::new(640., 480.)), ChildOf(root)))
        .id();
    text(
        &mut commands,
        editor,
        (205., 445.),
        410.,
        10.,
        "TAB / SHIFT+TAB to navigate / ENTER to activate",
    );
    let message = text(&mut commands, editor, (205., 402.), 410., 10., "");
    commands.entity(message).insert(Field::Message);
    for page in [Page::Identity, Page::Attributes, Page::Skills, Page::Review] {
        let panel =
            commands.spawn((absolute(Vec2::new(205., 80.), Vec2::new(413., 310.)), page)).id();
        commands.entity(editor).add_child(panel);
        match page {
            Page::Party => unreachable!(),
            Page::Identity => {
                text(&mut commands, panel, (0., 0.), 320., 20., "01 / Identity");
                commands
                    .spawn((
                        NameInput,
                        TextInput,
                        TextLayout::no_wrap(),
                        bevy::text::TextCursorStyle::default(),
                        TabIndex(0),
                        EditableText {
                            max_characters: Some(24),
                            allow_newlines: false,
                            ..EditableText::new("")
                        },
                        EditableTextFilter::new(|c| !c.is_control()),
                        TextFont { font_size: 11.0.into(), ..default() },
                        TextColor(PAPER),
                        absolute(Vec2::new(0.0, 43.0), Vec2::new(260.0, 27.0)),
                        BackgroundColor(PANEL),
                        BorderColor::all(GOLD),
                        ChildOf(panel),
                    ))
                    .observe(update_name)
                    .observe(finish_name);
                for (y, title, previous, next, field) in [
                    (87., "Race", Action::Race(-1), Action::Race(1), Field::Race),
                    (128., "Gender", Action::Gender, Action::Gender, Field::Gender),
                    (
                        169.,
                        "Profession",
                        Action::Profession(-1),
                        Action::Profession(1),
                        Field::Profession,
                    ),
                ] {
                    text(&mut commands, panel, (0., y), 75., 11., title);
                    button(&mut commands, panel, (77., y - 4.), 25., "<", previous);
                    let value = text(&mut commands, panel, (112., y), 130., 12., "");
                    commands.entity(value).insert(field);
                    button(&mut commands, panel, (260., y - 4.), 25., ">", next);
                }
                let portrait = commands
                    .spawn((
                        PreviewPortrait,
                        Picture {
                            sheet: assets.load("retail://data/portraits/medium/mhummf.sti"),
                            frame: 0,
                            bounds: None,
                            at: Vec2::new(313., 42.),
                        },
                    ))
                    .id();
                commands.entity(panel).add_child(portrait);
                button(&mut commands, panel, (313., 134.), 32., "<", Action::Portrait(-1));
                button(&mut commands, panel, (355., 134.), 32., ">", Action::Portrait(1));
                text(
                    &mut commands,
                    panel,
                    (0., 210.),
                    395.,
                    11.,
                    "Changing race, gender or profession refunds allocated points.\nPortraits are cosmetic and may be chosen freely.",
                );
                button(&mut commands, panel, (0., 275.), 100., "CANCEL", Action::Cancel);
                button(
                    &mut commands,
                    panel,
                    (225., 275.),
                    175.,
                    "ATTRIBUTES  >",
                    Action::Page(Page::Attributes),
                );
            }
            Page::Attributes => {
                text(&mut commands, panel, (0., 0.), 300., 20., "02 / Attributes");
                let value = text(&mut commands, panel, (0., 31.), 395., 11., "");
                commands.entity(value).insert(Field::Points);
                for (i, attribute) in EnumSet::<Attribute>::all().iter().enumerate() {
                    let y = 63. + i as f32 * 27.;
                    text(&mut commands, panel, (0., y), 155., 12., format!("{attribute:?}"));
                    button(
                        &mut commands,
                        panel,
                        (170., y - 4.),
                        28.,
                        "-",
                        Action::Attribute(attribute, false),
                    );
                    let value = text(&mut commands, panel, (215., y), 60., 12., "");
                    commands.entity(value).insert(Field::Attribute(attribute));
                    button(
                        &mut commands,
                        panel,
                        (275., y - 4.),
                        28.,
                        "+",
                        Action::Attribute(attribute, true),
                    );
                }
                button(
                    &mut commands,
                    panel,
                    (0., 275.),
                    100.,
                    "<  IDENTITY",
                    Action::Page(Page::Identity),
                );
                button(
                    &mut commands,
                    panel,
                    (225., 275.),
                    175.,
                    "SKILLS  >",
                    Action::Page(Page::Skills),
                );
            }
            Page::Skills => {
                text(&mut commands, panel, (0., 0.), 300., 20., "03 / Training");
                let value = text(&mut commands, panel, (0., 31.), 395., 11., "");
                commands.entity(value).insert(Field::Points);
                // Only eligible rows are laid out by refresh; the controls persist.
                for skill in EnumSet::<Skill>::all().iter().take(28) {
                    let row = commands.spawn((Node::default(), SkillRow(skill))).id();
                    commands.entity(panel).add_child(row);
                    text(&mut commands, row, (0., 5.), 112., 9., words(&format!("{skill:?}")));
                    button(&mut commands, row, (112., 0.), 20., "-", Action::Skill(skill, false));
                    let value = text(&mut commands, row, (136., 5.), 25., 10., "");
                    commands.entity(value).insert(Field::Skill(skill));
                    button(&mut commands, row, (164., 0.), 20., "+", Action::Skill(skill, true));
                }
                button(
                    &mut commands,
                    panel,
                    (0., 275.),
                    100.,
                    "<  ATTRIBUTES",
                    Action::Page(Page::Attributes),
                );
                button(
                    &mut commands,
                    panel,
                    (225., 275.),
                    175.,
                    "REVIEW  >",
                    Action::Page(Page::Review),
                );
            }
            Page::Review => {
                text(&mut commands, panel, (0., 0.), 300., 20., "04 / Review adventurer");
                let value = text(&mut commands, panel, (0., 45.), 395., 13., "");
                commands.entity(value).insert(Field::Review);
                text(
                    &mut commands,
                    panel,
                    (0., 205.),
                    395.,
                    11.,
                    "Spell selection and personality voices are not implemented.\nThis adventurer will begin without learned spells.\nUnspent creation points are discarded when the adventure begins.",
                );
                button(
                    &mut commands,
                    panel,
                    (0., 275.),
                    100.,
                    "<  SKILLS",
                    Action::Page(Page::Skills),
                );
                button(
                    &mut commands,
                    panel,
                    (225., 275.),
                    175.,
                    "ACCEPT ADVENTURER",
                    Action::Finish,
                );
            }
        }
    }
}
#[derive(Component)]
struct SkillRow(Skill);
fn words(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

fn activate(
    click: On<Activate>,
    buttons: Query<&Action, Without<InteractionDisabled>>,
    mut sessions: Query<&mut Creation>,
    mut next: ResMut<NextState<Session>>,
    mut menu: ResMut<NextState<MenuPage>>,
    state: Res<State<Session>>,
    mut focus: ResMut<InputFocus>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    mut roster: ResMut<PartyRoster>,
    mut formation: ResMut<Formation>,
    assets: Res<AssetServer>,
    destination: Res<NewGameDestination>,
    mut names: Query<&mut EditableText, With<NameInput>>,
    dialogs: Query<(), With<confirmation::DeleteDialog>>,
) {
    if !dialogs.is_empty() {
        return;
    }
    let Ok(action) = buttons.get(click.entity) else { return };
    if matches!(
        action,
        Action::NewGame
            | Action::Menu
            | Action::Add
            | Action::Cancel
            | Action::Finish
            | Action::Begin
            | Action::Page(_)
    ) {
        focus.clear();
    }
    match action {
        Action::NewGame => {
            next.set(Session::Creation);
            return;
        }
        Action::Quit => {
            exit.write(AppExit::Success);
            return;
        }
        Action::Credits => {
            focus.clear();
            menu.set(MenuPage::Credits);
            return;
        }
        Action::Menu => {
            next.set(Session::MainMenu);
            return;
        }
        _ => {}
    }
    let Ok(mut session) = sessions.single_mut() else { return };
    session.message.clear();
    match *action {
        Action::Add if session.candidates.len() < 6 => {
            session.editing = Some(CharacterDraft::default());
            session.page = Page::Identity;
        }
        Action::SelectCandidate(Some(id)) => {
            if session.candidates.iter().any(|(candidate, _)| *candidate == id) {
                session.selected = Some(id);
            }
        }
        Action::ToggleCandidate => {
            if let Some(id) = session.selected {
                if let Some(index) = session.party.iter().position(|member| *member == id) {
                    session.party.remove(index);
                } else if session.party.len() < 6 {
                    session.party.push(id);
                }
            }
        }
        Action::DeleteCandidate => {
            if let Some((id, draft)) = session
                .selected
                .and_then(|id| session.candidates.iter().find(|(candidate, _)| *candidate == id))
            {
                confirmation::open(&mut commands, &assets, *id, &draft.name, &mut focus);
            }
        }
        Action::Cancel => {
            session.editing = None;
            session.page = Page::Party;
        }
        Action::Page(page) => session.page = page,
        Action::Finish => {
            if let Some(draft) = &session.editing {
                match draft.finish() {
                    Err(message) => session.message = message.into(),
                    Ok(_) => {
                        let draft = session.editing.take().unwrap();
                        let id = session.next_id;
                        session.next_id += 1;
                        session.candidates.push((id, draft));
                        session.selected = Some(id);
                        session.page = Page::Party;
                    }
                }
            }
        }
        Action::Begin => {
            let drafts: Option<Vec<_>> = session
                .party
                .iter()
                .map(|id| {
                    session
                        .candidates
                        .iter()
                        .find(|(candidate, _)| candidate == id)
                        .map(|(_, draft)| draft.clone())
                })
                .collect();
            let Some(mut drafts) = drafts else {
                session.message = "A party member is no longer available.".into();
                return;
            };
            if let Err(message) = begin_adventure(
                &mut drafts,
                &state,
                &mut commands,
                &mut roster,
                &mut formation,
                &assets,
                &destination,
                &mut next,
            ) {
                session.message = message.into();
            }
        }
        action => {
            let Some(draft) = session.editing.as_mut() else { return };
            let result = match action {
                Action::Race(direction) => {
                    let i = PLAYABLE_RACES.iter().position(|&r| r == draft.race()).unwrap() as i32;
                    let choice = (1..=11)
                        .map(|offset| {
                            PLAYABLE_RACES[(i + direction * offset).rem_euclid(11) as usize]
                        })
                        .find(|&r| {
                            CharacterDraft::budget(r, draft.gender(), draft.profession()).is_ok()
                        })
                        .unwrap();
                    draft.choose(choice, draft.gender(), draft.profession())
                }
                Action::Gender => draft.choose(
                    draft.race(),
                    if draft.gender() == Gender::Male { Gender::Female } else { Gender::Male },
                    draft.profession(),
                ),
                Action::Profession(direction) => {
                    let professions: Vec<_> = EnumSet::<Profession>::all().iter().collect();
                    let i = draft.profession() as i32;
                    // Skip professions unavailable to this race and gender; selection is still validated.
                    let choice = (1..=15)
                        .map(|offset| professions[(i + direction * offset).rem_euclid(15) as usize])
                        .find(|&p| CharacterDraft::budget(draft.race(), draft.gender(), p).is_ok())
                        .unwrap();
                    draft.choose(draft.race(), draft.gender(), choice)
                }
                Action::Portrait(direction) => {
                    let i = PORTRAITS.iter().position(|&p| p == draft.portrait).unwrap_or(0) as i32;
                    draft.portrait = PORTRAITS
                        [(i + direction).rem_euclid(PORTRAITS.len() as i32) as usize]
                        .into();
                    Ok(())
                }
                Action::Attribute(a, increase) => {
                    if draft.adjust_attribute(a, increase) {
                        Ok(())
                    } else {
                        Err("Attribute minimum, allocation limit or point budget reached.")
                    }
                }
                Action::Skill(s, increase) => {
                    if draft.adjust_skill(s, increase) {
                        Ok(())
                    } else {
                        Err("Skills allow five allocated points each, within a pool of fifteen.")
                    }
                }
                _ => Ok(()),
            };
            if let Err(message) = result {
                session.message = message.into();
            }
        }
    }
    if matches!(*action, Action::Add)
        && let Some(draft) = &session.editing
    {
        for mut name in &mut names {
            *name = EditableText {
                max_characters: Some(24),
                allow_newlines: false,
                ..EditableText::new(&draft.name)
            };
        }
    }
}

fn update_name(
    event: On<TextEditChange>,
    names: Query<&EditableText, With<NameInput>>,
    mut sessions: Query<&mut Creation>,
) {
    let (Ok(name), Ok(mut session)) = (names.get(event.event_target()), sessions.single_mut())
    else {
        return;
    };
    if let Some(draft) = session.editing.as_mut() {
        let value = name.value().to_string();
        if draft.name != value {
            draft.name = value;
        }
    }
}

fn finish_name(
    event: On<bevy::input_focus::FocusedInput<KeyboardInput>>,
    names: Query<&EditableText, With<NameInput>>,
    mut focus: ResMut<InputFocus>,
) {
    if event.input.state.is_pressed()
        && event.input.logical_key == Key::Enter
        && names.get(event.original_event_target()).is_ok_and(|name| !name.is_composing())
    {
        focus.clear();
    }
}

fn show_editor(
    session: Option<Single<Ref<Creation>>>,
    mut editors: Query<(&mut Node, &mut Visibility), With<EditorGroup>>,
) {
    let Some(session) = session else { return };
    for (mut node, mut visibility) in &mut editors {
        let shown = session.page != Page::Party;
        node.display = if shown { Display::Flex } else { Display::None };
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

fn refresh(
    sessions: Query<Ref<Creation>>,
    focus: Res<InputFocus>,
    mut commands: Commands,
    buttons: Query<(Entity, &Action, Has<InteractionDisabled>), Without<party::Control>>,
    mut pages: Query<(&Page, &mut Node, &mut Visibility), Without<SkillRow>>,
    mut fields: Query<(&Field, &mut Text)>,
    mut rows: Query<(&SkillRow, &mut Node, &mut Visibility), Without<Page>>,
    mut portraits: Query<&mut Picture, With<PreviewPortrait>>,
    assets: Res<AssetServer>,
) {
    let Ok(session) = sessions.single() else { return };
    if !session.is_changed() && !focus.is_changed() {
        return;
    }
    for (page, mut node, mut visibility) in &mut pages {
        let display = if *page == session.page { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        // Tab navigation follows visibility rather than layout display.
        visibility.set_if_neq(if *page == session.page {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let draft = session.editing.as_ref();
    let profile = draft.map(CharacterDraft::preview);
    for (entity, action, disabled) in &buttons {
        let enabled = match *action {
            Action::Add => session.editing.is_none() && session.candidates.len() < 6,
            Action::Begin => session.editing.is_none() && !session.party.is_empty(),
            _ => true,
        };
        crate::presentation::retail_controls::set_enabled(&mut commands, entity, enabled, disabled);
    }

    for (field, mut text) in &mut fields {
        let value = match field {
            Field::Message => session.message.clone(),
            _ => {
                let (Some(draft), Some(profile)) = (draft, &profile) else { continue };
                match field {
                    Field::Race => format!("{:?}", draft.race()),
                    Field::Gender => format!("{:?}", draft.gender()),
                    Field::Profession => format!("{:?}", draft.profession()),
                    Field::Points => {
                        if session.page == Page::Attributes {
                            format!(
                                "{} points remaining / profession minimums already paid",
                                draft.attribute_remaining()
                            )
                        } else {
                            format!(
                                "{} points remaining / up to five per skill / optional",
                                draft.skill_remaining()
                            )
                        }
                    }
                    Field::Attribute(a) => profile.attributes[*a].to_string(),
                    Field::Skill(s) => profile.skill_points[*s].to_string(),
                    Field::Review => format!(
                        "{}\n{:?} / {:?} / {:?}\n\nHealth {}     Stamina {}\n\nUnspent attributes: {}    Unspent skills: {}\nStarting equipment is chosen from your profession and skills.",
                        profile.name,
                        profile.race,
                        profile.gender,
                        profile.profession,
                        profile.max_hp(),
                        profile.max_stamina(),
                        draft.attribute_remaining(),
                        draft.skill_remaining()
                    ),
                    _ => unreachable!(),
                }
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    if let Some(draft) = draft {
        // Query order does not decide the skill order.
        let available: Vec<_> =
            EnumSet::<Skill>::all().iter().filter(|&s| draft.skill_available(s)).collect();
        for (row, mut node, mut visibility) in &mut rows {
            if let Some(i) = available.iter().position(|s| *s == row.0) {
                visibility.set_if_neq(Visibility::Inherited);
                let desired = absolute(
                    Vec2::new((i / 10) as f32 * 205., 57. + (i % 10) as f32 * 21.),
                    Vec2::new(190., 25.),
                );
                if node.left != desired.left
                    || node.top != desired.top
                    || node.display == Display::None
                {
                    *node = desired;
                }
            } else {
                visibility.set_if_neq(Visibility::Hidden);
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
        for mut picture in &mut portraits {
            let sheet =
                assets.load(format!("retail://data/portraits/medium/m{}.sti", draft.portrait));
            if picture.sheet != sheet {
                picture.sheet = sheet;
            }
        }
    }
}
fn button_colours(
    focus: Res<InputFocus>,
    mut buttons: Query<
        (Entity, &Hovered, Has<Pressed>, Has<InteractionDisabled>, &mut BackgroundColor),
        (With<Action>, Without<MenuItem>, Without<party::Control>),
    >,
) {
    for (entity, hovered, pressed, disabled, mut background) in &mut buttons {
        let colour = if disabled {
            INK
        } else if pressed {
            Color::srgb(0.29, 0.23, 0.13)
        } else if hovered.get() || focus.get() == Some(entity) {
            Color::srgb(0.16, 0.18, 0.21)
        } else {
            PANEL
        };
        if background.0 != colour {
            background.0 = colour;
        }
    }
}

// Retail portrait catalog; any portrait may be selected for any character.
const PORTRAITS: &[&str] = &[
    "hummf", "hummn", "hummm", "humm1", "humm2", "humm3", "humff", "humfn", "humfm", "humf1",
    "humf2", "humf3", "elfmf", "elfmn", "elfmm", "elfff", "elffn", "elffm", "dwarfmf", "dwarfmn",
    "dwarfmm", "dwarfff", "dwarffn", "dwarffm", "gnomemn", "gnomemm", "gnomeff", "gnomefn",
    "hobmf", "hobmn", "hobff", "hobfn", "fairymf", "fairymm", "fairyff", "fairyfn", "lizmf",
    "lizmn", "lizff", "lizfn", "dracmf", "dracmn", "dracff", "dracfn", "felpmf", "felpmn",
    "felpff", "felpfn", "rawmn", "rawmm", "rawff", "rawfn", "mookmf", "mookmn", "mookff", "mookfn",
    "ninmf", "ninff", "mook", "trynm1", "trynm2", "trynm3", "trang", "umpani", "sexus", "urq",
    "madras", "sparkle", "myles", "vi", "drazic", "tantris", "rodan", "glumph", "saxx", "humm4",
    "humf4", "elfm1", "elff1",
];

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use crate::presentation::test_input::{key, tap};
    use bevy::input::ButtonState;
    use bevy::input_focus::InputDispatchPlugin;
    use bevy::ui_widgets::ButtonPlugin;
    use bevy::window::PrimaryWindow;

    #[test]
    fn native_name_edits_update_only_the_working_draft() {
        use bevy::text::{FontCx, LayoutCx, TextEdit, apply_text_edits};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<FontCx>()
            .init_resource::<LayoutCx>()
            .init_resource::<bevy::clipboard::Clipboard>()
            .add_systems(Update, apply_text_edits);
        app.world_mut().resource_mut::<FontCx>().collection.register_fonts(
            bevy::text::Font::from_bytes(bevy::text::DEFAULT_FONT_DATA.to_vec()).data,
            None,
        );
        let root = app
            .world_mut()
            .spawn(Creation {
                candidates: vec![],
                party: vec![],
                selected: None,
                next_id: 0,
                editing: Some(CharacterDraft::default()),
                page: Page::Identity,
                message: String::new(),
            })
            .id();
        let field = app
            .world_mut()
            .spawn((NameInput, EditableText { max_characters: Some(24), ..EditableText::new("A") }))
            .observe(update_name)
            .id();
        // The real UI style system supplies the font family before editing.
        let family = bevy::text::FontSource::Family("Fira Mono".into())
            .resolve_font_family(&Assets::<bevy::text::Font>::default())
            .unwrap()
            .into_owned();
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor
            .edit_styles()
            .insert(family.into());
        for edit in [TextEdit::SelectAll, TextEdit::Insert("Nora".into())] {
            app.world_mut().get_mut::<EditableText>(field).unwrap().queue_edit(edit);
        }
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().name,
            "Nora"
        );
        for edit in
            [TextEdit::TextStart(false), TextEdit::Right(true), TextEdit::Insert("Z".into())]
        {
            app.world_mut().get_mut::<EditableText>(field).unwrap().queue_edit(edit);
        }
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().name,
            "Zora"
        );
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .queue_edit(TextEdit::ImeSetCompose { value: "é".into(), cursor: None });
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().name,
            "Zora"
        );
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .queue_edit(TextEdit::ImeCommit { value: "é".into() });
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().name,
            "Zéora"
        );
        assert!(app.world().get::<Creation>(root).unwrap().party.is_empty());
    }

    #[test]
    fn frontend_can_reenter_tab_navigation_after_page_and_name_changes() {
        let mut app = crate::presentation::test_input::input_app();
        app.add_plugins((
            InputDispatchPlugin,
            ButtonPlugin,
            bevy::ui_widgets::ModalDialogPlugin,
            bevy::ui_widgets::TextInputPlugin,
            bevy::camera::visibility::VisibilityPlugin,
        ))
        .init_asset::<crate::content::ui::Credits>()
        .init_asset::<crate::content::ui::TextTable>()
        .init_asset::<Image>()
        .init_state::<Session>()
        .init_asset::<crate::content::ui::SpriteSheet>()
        .init_asset::<Mesh>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .add_message::<bevy::window::Ime>()
        .add_message::<bevy::picking::events::PointerRelease>()
        .init_resource::<bevy::picking::events::PointerState>()
        .init_resource::<UiScale>()
        .init_resource::<PartyRoster>()
        .init_resource::<Formation>()
        .insert_resource(NewGameDestination { path: String::new(), start: default() })
        .add_message::<AppExit>()
        .add_plugins(FrontendPlugin)
        .add_systems(PostUpdate, crate::presentation::retail_controls::sync_button_navigation);
        let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        app.finish();
        app.cleanup();
        app.update();
        let credits = app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
            .find_map(|(entity, action)| matches!(action, Action::Credits).then_some(entity))
            .unwrap();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(credits, bevy::input_focus::FocusCause::Navigated);
        key(&mut app, window, KeyCode::Enter, true);
        app.update();
        assert_eq!(*app.world().resource::<State<MenuPage>>().get(), MenuPage::Credits);
        app.update();
        assert_eq!(*app.world().resource::<State<MenuPage>>().get(), MenuPage::Credits);
        key(&mut app, window, KeyCode::Enter, false);
        key(&mut app, window, KeyCode::Escape, true);
        app.update();
        assert_eq!(*app.world().resource::<State<MenuPage>>().get(), MenuPage::Main);
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        key(&mut app, window, KeyCode::Escape, false);
        key(&mut app, window, KeyCode::KeyS, true);
        app.update();
        assert_eq!(*app.world().resource::<State<Session>>().get(), Session::Creation);
        key(&mut app, window, KeyCode::KeyS, false);
        tap(&mut app, window, KeyCode::Tab);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert!(matches!(app.world().get::<Action>(focused), Some(Action::Add)));
        tap(&mut app, window, KeyCode::Enter);
        app.update();
        // The transition cleared focus. Tab still finds a visible enabled control.
        tap(&mut app, window, KeyCode::Tab);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert!(
            app.world().get::<Action>(focused).is_some()
                || app.world().get::<NameInput>(focused).is_some()
        );
        assert!(app.world().get::<InteractionDisabled>(focused).is_none());
        assert!(app.world().get::<InheritedVisibility>(focused).unwrap().get());
        let name = app
            .world_mut()
            .query_filtered::<Entity, With<NameInput>>()
            .iter(app.world())
            .next()
            .unwrap();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(name, bevy::input_focus::FocusCause::Navigated);
        tap(&mut app, window, KeyCode::Escape);
        tap(&mut app, window, KeyCode::Tab);
        if app.world().resource::<InputFocus>().get() == Some(name) {
            tap(&mut app, window, KeyCode::Tab);
        }
        assert!(
            app.world()
                .resource::<InputFocus>()
                .get()
                .and_then(|e| app.world().get::<Action>(e))
                .is_some()
        );

        let owner =
            app.world_mut().query_filtered::<Entity, With<Creation>>().single(app.world()).unwrap();
        let mut session = app.world_mut().get_mut::<Creation>(owner).unwrap();
        session.editing = None;
        session.page = Page::Party;
        session.candidates.push((7, CharacterDraft::default()));
        session.party.push(7);
        session.selected = Some(7);
        app.update();
        let delete = app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
            .find_map(|(entity, action)| {
                matches!(action, Action::DeleteCandidate).then_some(entity)
            })
            .unwrap();
        key(&mut app, window, KeyCode::ShiftLeft, true);
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Enter,
            logical_key: Key::Enter,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.world_mut().trigger(Activate { entity: delete });
        app.update();
        app.update();
        assert_eq!(app.world().get::<Creation>(owner).unwrap().party, [7]);
        key(&mut app, window, KeyCode::Enter, false);
        tap(&mut app, window, KeyCode::Escape);
        assert_eq!(app.world().get::<Creation>(owner).unwrap().party, [7]);
        app.world_mut().trigger(Activate { entity: delete });
        app.update();
        tap(&mut app, window, KeyCode::Enter);
        let session = app.world().get::<Creation>(owner).unwrap();
        assert!(session.candidates.is_empty());
        assert!(session.party.is_empty());

        let back = app
            .world_mut()
            .query::<(Entity, &Action, Has<party::Control>)>()
            .iter(app.world())
            .find_map(|(entity, action, party)| {
                (party && matches!(action, Action::Menu)).then_some(entity)
            })
            .unwrap();
        app.world_mut().trigger(Activate { entity: back });
        app.update();
        app.update();
        assert_eq!(*app.world().resource::<State<Session>>().get(), Session::MainMenu);
        key(&mut app, window, KeyCode::KeyS, true);
        app.update();
        assert_eq!(*app.world().resource::<State<Session>>().get(), Session::Creation);
    }
}
