//! Retained frontend screens. Each character being made is a draft entity
//! (`game::characters::draft`); the creation root holds which they are, and
//! they outlive it into the campaign they begin.
use super::screen_art::{BitmapText, Picture, absolute, spawn_picture};
use crate::content::characters::{Attribute, Gender, Profession, Race, Skill};
use crate::content::ui::SpriteSheet;
use crate::game::actors::{Abilities, UsableSkills};
use crate::game::characters::creation::{PARTY_LIMIT, PLAYABLE_RACES, default_gender, identity};
use crate::game::characters::draft::{
    Allot, Allotment, Allotted, Conceive, Forced, Introduce, MadeIn, Newborn, Pick, Picked, Points,
    budget, may_raise_attribute, may_raise_skill, newborn_bundle, picks_left, unready,
};
use crate::game::characters::progression::SpellPicks;
use crate::game::characters::{Birth, Calling, Identity, Training};
use crate::game::data::Catalogs;
use crate::game::session::begin_adventure;
use crate::presentation::screen::Screen;
use crate::presentation::screen_art::visible_if;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{
    InputFocus,
    tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEditChange};
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::{Activate, Button as UiButton};
use bevy_enhanced_input::prelude::Action as InputBindingAction;
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

mod confirmation;
mod creator_identity;
mod creator_skills;
mod creator_spells;
mod creator_stats;
mod credits;
mod party;
mod spell_info;

/// Which frontend screen shows: the main menu, party creation, or the world
/// once a party plays in it.
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Session {
    #[default]
    MainMenu,
    Creation,
    Playing,
}

pub struct FrontendPlugin;
impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TabNavigationPlugin)
            .init_state::<Session>()
            .add_sub_state::<MenuPage>()
            .add_observer(show_the_world)
            .add_systems(PostStartup, skip_the_menu)
            .add_input_context::<MenuInput>()
            .add_input_context::<credits::CreditsInput>()
            .add_input_context::<confirmation::ConfirmDialog>()
            .add_systems(
                PreUpdate,
                arm_confirmation_input
                    .after(bevy::input::InputSystems)
                    .before(EnhancedInputSystems::Update),
            )
            .add_observer(confirmation::resolve)
            .add_systems(PostUpdate, commit_creation.after(bevy::text::EditableTextSystems))
            .add_systems(OnEnter(MenuPage::Main), main_menu)
            .add_systems(OnEnter(MenuPage::Credits), credits::enter)
            .add_systems(OnEnter(Session::Creation), creation)
            .add_systems(
                Update,
                (
                    refresh,
                    creator_stats::refresh,
                    creator_skills::refresh,
                    creator_spells::refresh,
                    spell_info::populate,
                    spell_info::scroll,
                    spell_info::animate,
                    creator_identity::refresh,
                    creator_identity::refresh_quote,
                    creator_identity::play_summary_voice,
                    show_editor,
                    party::sync_views,
                    (
                        party::refresh_controls,
                        party::refresh_selection_marks,
                        party::refresh_selected,
                    )
                        .run_if(party::outdated),
                    paint_menu,
                    party::paint_labels,
                    confirmation::show_text,
                    credits::populate,
                    credits::scroll,
                )
                    .chain()
                    .run_if(not(in_state(Session::Playing))),
            );
    }
}

/// The world shows once a party plays in it: a new party's first level has
/// committed. A failed entry leaves creation, and its drafts, to retry.
fn show_the_world(
    _: On<Add<crate::game::party::roster::Party>>,
    mut next: ResMut<NextState<Session>>,
) {
    next.set(Session::Playing);
}

/// A campaign already entering its level at startup (a quick start) skips
/// the menu.
fn skip_the_menu(
    entering: Query<(), With<crate::game::world::level::Destination>>,
    mut next: ResMut<NextState<Session>>,
) {
    if !entering.is_empty() {
        next.set(Session::Playing);
    }
}

// Native buttons also receive raw keyboard events, so the confirmation waits
// for the opening key to be released before accepting either input path.
fn arm_confirmation_input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    confirmation: Query<(Entity, &ContextActivity<confirmation::ConfirmDialog>)>,
) {
    if [KeyCode::Enter, KeyCode::Space]
        .into_iter()
        .any(|key| keys.pressed(key) || keys.just_pressed(key))
    {
        return;
    }
    for (entity, active) in &confirmation {
        if !**active {
            commands.entity(entity).insert(ContextActivity::<confirmation::ConfirmDialog>::ACTIVE);
        }
    }
}

#[derive(SubStates, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[source(Session = Session::MainMenu)]
pub(super) enum MenuPage {
    #[default]
    Main,
    Credits,
    Options,
}

const INK: Color = Color::srgb(0.035, 0.045, 0.06);

#[derive(Clone, Copy, PartialEq, Eq, Component)]
enum Page {
    Party,
    Identity,
    Magic,
    Skills,
    Review,
}
/// Party creation: the characters made (draft entities), those chosen for
/// the party in order, the one selected, and the one being made.
#[derive(Component)]
struct Creation {
    candidates: Vec<Entity>,
    party: Vec<Entity>,
    selected: Option<Entity>,
    mode: CreationMode,
    choices: CreatorChoices,
    message: String,
}
enum CreationMode {
    Party,
    Editing { draft: Entity, step: EditStep },
}
#[derive(Clone, Copy)]
enum EditStep {
    Identity,
    Magic,
    Skills,
    Review,
}
impl CreationMode {
    fn draft(&self) -> Option<Entity> {
        match self {
            Self::Party => None,
            Self::Editing { draft, .. } => Some(*draft),
        }
    }
    fn page(&self) -> Page {
        match self {
            Self::Party => Page::Party,
            Self::Editing { step, .. } => match step {
                EditStep::Identity => Page::Identity,
                EditStep::Magic => Page::Magic,
                EditStep::Skills => Page::Skills,
                EditStep::Review => Page::Review,
            },
        }
    }
    fn set_page(&mut self, page: Page) {
        let next = match page {
            Page::Party => {
                *self = Self::Party;
                return;
            }
            Page::Identity => EditStep::Identity,
            Page::Magic => EditStep::Magic,
            Page::Skills => EditStep::Skills,
            Page::Review => EditStep::Review,
        };
        if let Self::Editing { step, .. } = self {
            *step = next;
        }
    }
}
#[derive(Clone, Copy, Default)]
struct CreatorChoices {
    profession: bool,
    race: bool,
    gender: bool,
    open: Option<CreatorChoice>,
}
impl CreatorChoices {
    fn complete(self) -> bool {
        self.profession && self.race && self.gender
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CreatorChoice {
    Profession,
    Race,
    Gender,
}
/// A menu button; pressed, it does what it is ([`activate`], its own
/// observer).
#[derive(Component, Clone, Copy, PartialEq)]
#[component(on_add = pressed)]
enum Action {
    NewGame,
    Options,
    Credits,
    Quit,
    Menu,
    Add,
    SelectCandidate(Option<Entity>),
    ToggleCandidate,
    DeleteCandidate,
    Page(Page),
    Cancel,
    Finish,
    Begin,
    Race(i32),
    Gender(i32),
    Profession(i32),
    ToggleChoice(CreatorChoice),
    ChooseProfession(Profession),
    ChooseRace(Race),
    ChooseGender(Gender),
    ResetAttributes,
    ResetSkills,
    ResetSpells,
    Portrait(i32),
    PortraitGroup(i32),
    Attribute(Attribute, bool),
    Skill(Skill, bool),
}

fn pressed(mut world: bevy::ecs::world::DeferredWorld, context: bevy::ecs::lifecycle::HookContext) {
    world.commands().entity(context.entity).observe(activate);
}
#[derive(Component)]
struct NameInput;
/// A Finish pressed while the name editor still had unapplied input.
#[derive(Component)]
struct NicknameInput;
#[derive(Component)]
struct EditorGroup;
#[derive(Component)]
struct FinishPending;

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
struct MenuOptions;
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
        (234., 335., 175., Some(Action::Options)),
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
            button.insert((InteractionDisabled, TabIndex(-1)));
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
            -- InputBindingAction::<MenuOptions>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::KeyO) ]
            -- InputBindingAction::<MenuExit>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::Escape) Binding(KeyCode::KeyX) Binding(KeyCode::KeyE) ]
        ]
        on(|_: On<Start<MenuOptions>>, mut menu: ResMut<NextState<MenuPage>>, mut screen: ResMut<NextState<Screen>>| {menu.set(MenuPage::Options);screen.set(Screen::Options);})
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
        mode: CreationMode::Party,
        choices: CreatorChoices::default(),
        message: String::new(),
    });
    party::build(&mut commands, root, &assets);
    let editor = commands
        .spawn((EditorGroup, absolute(Vec2::ZERO, Vec2::new(640., 480.)), ChildOf(root)))
        .id();
    for page in [Page::Identity, Page::Magic, Page::Skills, Page::Review] {
        let node = absolute(Vec2::ZERO, Vec2::new(640., 480.));
        let panel = commands.spawn((node, page)).id();
        commands.entity(editor).add_child(panel);
        match page {
            Page::Party => unreachable!(),
            Page::Identity => creator_stats::build(&mut commands, panel, &assets),
            Page::Magic => creator_spells::build(&mut commands, panel, &assets),
            Page::Skills => creator_skills::build(&mut commands, panel, &assets),
            Page::Review => creator_identity::build(&mut commands, panel, &assets),
        }
    }
}
fn activate(
    click: On<Activate>,
    buttons: Query<(&Action, Has<InteractionDisabled>)>,
    mut sessions: Query<(Entity, &mut Creation)>,
    mut next: ResMut<NextState<Session>>,
    mut menu: ResMut<NextState<MenuPage>>,
    mut screen: ResMut<NextState<Screen>>,
    mut focus: ResMut<InputFocus>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut names: Query<
        (Entity, &mut EditableText, Has<NicknameInput>),
        Or<(With<NameInput>, With<NicknameInput>)>,
    >,
    dialogs: Query<(), With<confirmation::ConfirmDialog>>,
    drafts: Query<(
        (&Points, &Forced, &Allotted),
        (&Abilities, &Training, &UsableSkills),
        (&Birth, &Calling, &SpellPicks, &Picked),
        &Identity,
    )>,
    content: If<Res<Catalogs>>,
) -> Result {
    if !dialogs.is_empty() {
        return Ok(());
    }
    let (action, disabled) = buttons.get(click.entity)?;
    if disabled {
        return Ok(());
    }
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
            return Ok(());
        }
        Action::Quit => {
            exit.write(AppExit::Success);
            return Ok(());
        }
        Action::Options => {
            menu.set(MenuPage::Options);
            screen.set(Screen::Options);
        }
        Action::Credits => {
            focus.clear();
            menu.set(MenuPage::Credits);
            return Ok(());
        }
        Action::Menu => {
            // The characters made go with the session (`MadeIn`).
            next.set(Session::MainMenu);
            return Ok(());
        }
        _ => {}
    }
    let Ok((session_entity, mut session)) = sessions.single_mut() else { return Ok(()) };
    session.message.clear();
    if !matches!(action, Action::ToggleChoice(_)) {
        session.choices.open = None;
    }
    let editing = session.mode.draft();
    let sheet = editing.and_then(|draft| drafts.get(draft).ok());
    match *action {
        Action::Add if session.candidates.len() < PARTY_LIMIT => {
            let draft = commands.spawn((newborn_bundle(""), Newborn, MadeIn(session_entity))).id();
            commands.trigger(Conceive {
                draft,
                race: Race::Human,
                gender: Gender::Male,
                profession: Profession::Fighter,
            });
            session.mode = CreationMode::Editing { draft, step: EditStep::Identity };
            session.choices = CreatorChoices::default();
            for (_, mut name, _) in &mut names {
                name.clear();
            }
        }
        Action::SelectCandidate(Some(candidate)) => {
            if session.candidates.contains(&candidate) {
                session.selected = Some(candidate);
            }
        }
        Action::ToggleCandidate => {
            if let Some(candidate) = session.selected {
                if let Some(index) = session.party.iter().position(|&member| member == candidate) {
                    session.party.remove(index);
                } else if session.party.len() < PARTY_LIMIT {
                    session.party.push(candidate);
                }
            }
        }
        Action::DeleteCandidate => {
            if let Some(candidate) = session.selected
                && let Ok((.., identity)) = drafts.get(candidate)
            {
                confirmation::open(&mut commands, &assets, candidate, &identity.name, &mut focus);
            }
        }
        Action::Cancel => {
            if let Some(draft) = editing {
                commands.entity(draft).despawn();
            }
            session.mode = CreationMode::Party;
        }
        Action::ToggleChoice(choice) => {
            session.choices.open =
                if session.choices.open == Some(choice) { None } else { Some(choice) };
        }
        Action::Page(Page::Skills) if session.mode.page() == Page::Identity => {
            if let Some((spent, (abilities, ..), (_, calling, granted, picked), _)) = sheet
                && session.choices.complete()
                && attributes_done(spent, abilities)
            {
                let (_, total) = spell_picks(calling, granted, picked, &content);
                session.mode.set_page(if total > 0 { Page::Magic } else { Page::Skills });
            }
        }
        Action::Page(Page::Skills) if session.mode.page() == Page::Magic => {
            if let Some((_, _, (_, calling, granted, picked), _)) = sheet {
                let (left, total) = spell_picks(calling, granted, picked, &content);
                if left > 0 && left < total {
                    confirmation::confirm_remaining_spells(&mut commands, &assets, &mut focus);
                } else if left < total {
                    session.mode.set_page(Page::Skills);
                }
            }
        }
        Action::Page(Page::Identity) if session.mode.page() == Page::Skills => {
            let magic = sheet.is_some_and(|(_, _, (_, calling, granted, picked), _)| {
                spell_picks(calling, granted, picked, &content).1 > 0
            });
            session.mode.set_page(if magic { Page::Magic } else { Page::Identity });
        }
        Action::Page(Page::Review) if session.mode.page() == Page::Skills => {
            if let Some(((points, _, allotted), (_, training, usable), _, identity)) = sheet
                && skills_done((points, allotted), (training, usable))
            {
                session.mode.set_page(Page::Review);
                let nickname = !identity.full_name.trim().is_empty();
                if let Some(entity) = names.iter_mut().find_map(|(entity, _, is_nickname)| {
                    (nickname == is_nickname).then_some(entity)
                }) {
                    focus.set(entity, bevy::input_focus::FocusCause::Navigated);
                }
            }
        }
        Action::Page(page) => session.mode.set_page(page),
        Action::Finish => {
            if session.mode.page() == Page::Review && session.choices.complete() {
                commands.entity(session_entity).insert(FinishPending);
            }
        }
        Action::Begin => {
            let party = session.party.clone();
            commands.queue(move |world: &mut World| {
                if let Err(message) = begin_adventure(world, &party) {
                    let mut sessions = world.query::<&mut Creation>();
                    if let Ok(mut session) = sessions.single_mut(world) {
                        session.message = message.into();
                    }
                }
            });
        }
        action => {
            let (
                Some(draft),
                Some((spent, (abilities, training, usable), (birth, calling, _, picked), identity)),
            ) = (editing, sheet)
            else {
                return Ok(());
            };
            let (points, forced, allotted) = spent;
            let choices = session.choices;
            let (race, gender, profession) = (birth.race, birth.gender, calling.profession);
            let conceive = |race, gender, profession| {
                budget(race, gender, profession).map(|_| Conceive {
                    draft,
                    race,
                    gender,
                    profession,
                })
            };
            let result = match action {
                Action::ResetSpells => {
                    for &spell in &picked.0 {
                        commands.trigger(Pick { draft, spell });
                    }
                    Ok(())
                }
                Action::Race(direction) => {
                    let i = if choices.race {
                        PLAYABLE_RACES.iter().position(|&r| r == race).unwrap() as i32
                    } else if direction > 0 {
                        -1
                    } else {
                        0
                    };
                    let choice = (1..=11)
                        .map(|offset| {
                            PLAYABLE_RACES[(i + direction * offset).rem_euclid(11) as usize]
                        })
                        .find(|&r| budget(r, gender, profession).is_ok())
                        .unwrap();
                    conceive(choice, gender, profession).map(|c| commands.trigger(c))
                }
                Action::Gender(direction) => {
                    let gender = if !choices.gender {
                        if profession.row().only_gender == Some(Gender::Female) || direction < 0 {
                            Gender::Female
                        } else {
                            Gender::Male
                        }
                    } else if gender == Gender::Male {
                        Gender::Female
                    } else {
                        Gender::Male
                    };
                    conceive(race, gender, profession).map(|c| commands.trigger(c))
                }
                Action::Profession(direction) => {
                    let professions: Vec<_> = EnumSet::<Profession>::all().iter().collect();
                    let i = if choices.profession {
                        profession as i32
                    } else if direction > 0 {
                        -1
                    } else {
                        0
                    };
                    // Skip professions unavailable to this race and gender; selection is still validated.
                    let choice = (1..=15)
                        .map(|offset| professions[(i + direction * offset).rem_euclid(15) as usize])
                        .find(|&p| {
                            budget(race, default_gender(p, gender, choices.gender), p).is_ok()
                        })
                        .unwrap();
                    let gender = default_gender(choice, gender, choices.gender);
                    conceive(race, gender, choice).map(|c| commands.trigger(c))
                }
                Action::ChooseProfession(choice) => {
                    let gender = default_gender(choice, gender, choices.gender);
                    conceive(race, gender, choice).map(|c| commands.trigger(c))
                }
                Action::ChooseRace(choice) => {
                    conceive(choice, gender, profession).map(|c| commands.trigger(c))
                }
                Action::ChooseGender(choice) => {
                    conceive(race, choice, profession).map(|c| commands.trigger(c))
                }
                Action::Portrait(direction) => {
                    let portrait = identity::next_portrait(&identity.portrait, direction);
                    let identity = Identity { portrait: portrait.into(), ..identity.clone() };
                    commands.trigger(Introduce { draft, identity });
                    Ok(())
                }
                Action::PortraitGroup(direction) => {
                    let portrait = identity::next_portrait_group(&identity.portrait, direction);
                    let identity = Identity { portrait: portrait.into(), ..identity.clone() };
                    commands.trigger(Introduce { draft, identity });
                    Ok(())
                }
                Action::Attribute(a, up) => {
                    let may = if up {
                        may_raise_attribute((points, forced, allotted), abilities, a)
                    } else {
                        allotted.attributes[a] > 0
                    };
                    if may {
                        commands.trigger(Allot { draft, what: Allotment::Attribute(a), up });
                        Ok(())
                    } else {
                        Err("Attribute minimum, allocation limit or point budget reached.")
                    }
                }
                Action::ResetAttributes => {
                    for (a, &put) in &allotted.attributes {
                        for _ in 0..put {
                            commands.trigger(Allot {
                                draft,
                                what: Allotment::Attribute(a),
                                up: false,
                            });
                        }
                    }
                    Ok(())
                }
                Action::ResetSkills => {
                    for (s, &put) in &allotted.skills {
                        for _ in 0..put {
                            commands.trigger(Allot { draft, what: Allotment::Skill(s), up: false });
                        }
                    }
                    Ok(())
                }
                Action::Skill(s, up) => {
                    let may = if up {
                        may_raise_skill((points, allotted), (training, usable), s)
                    } else {
                        allotted.skills[s] > 0
                    };
                    if may {
                        commands.trigger(Allot { draft, what: Allotment::Skill(s), up });
                        Ok(())
                    } else {
                        Err("Skills allow five allocated points each, within a pool of fifteen.")
                    }
                }
                _ => Ok(()),
            };
            if let Err(message) = result {
                session.message = message.into();
            } else {
                match action {
                    Action::Race(_) => session.choices.race = true,
                    Action::Gender(_) => session.choices.gender = true,
                    Action::Profession(_) => session.choices.profession = true,
                    Action::ChooseRace(_) => {
                        session.choices.race = true;
                        session.choices.open = None;
                    }
                    Action::ChooseGender(_) => {
                        session.choices.gender = true;
                        session.choices.open = None;
                    }
                    Action::ChooseProfession(_) => {
                        session.choices.profession = true;
                        session.choices.open = None;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// The picks a new character may spend, as (left, granted).
fn spell_picks(
    calling: &Calling,
    granted: &SpellPicks,
    picked: &Picked,
    content: &Catalogs,
) -> (usize, usize) {
    let left = picks_left(granted, calling, &picked.0, content).map_or(0, |p| p.available(calling));
    (left, granted.available(calling))
}

/// No attribute may take another point.
fn attributes_done(spent: (&Points, &Forced, &Allotted), abilities: &Abilities) -> bool {
    !EnumSet::<Attribute>::all().iter().any(|a| may_raise_attribute(spent, abilities, a))
}

/// No skill may take another point.
fn skills_done(spent: (&Points, &Allotted), sheet: (&Training, &UsableSkills)) -> bool {
    !EnumSet::<Skill>::all().iter().any(|s| may_raise_skill(spent, sheet, s))
}

#[allow(clippy::type_complexity)]
fn commit_creation(
    mut sessions: Query<(Entity, &mut Creation), With<FinishPending>>,
    inputs: Query<(&EditableText, Has<NicknameInput>), Or<(With<NameInput>, With<NicknameInput>)>>,
    drafts: Query<(
        &Identity,
        (&Points, &Forced, &Allotted),
        (&Abilities, &Training, &UsableSkills),
    )>,
    mut commands: Commands,
) {
    for (entity, mut session) in &mut sessions {
        if inputs.iter().any(|(input, _)| {
            !input.pending_edits.is_empty() || input.pending_paste.is_some() || input.is_composing()
        }) {
            continue;
        }
        commands.entity(entity).remove::<FinishPending>();
        if session.mode.page() != Page::Review || !session.choices.complete() {
            continue;
        }
        let Some(draft) = session.mode.draft() else { continue };
        let (identity, spent, sheet) = drafts.get(draft).expect("the draft edited is a character");
        let mut identity = identity.clone();
        for (input, nickname) in &inputs {
            let value = input.value().to_string();
            if nickname {
                identity.name = value;
            } else {
                identity.full_name = value;
            }
        }
        // The nickname is told apart from every other character made.
        let others = session.candidates.iter().filter_map(|&other| drafts.get(other).ok());
        let taken = crate::game::characters::draft::nickname_taken(
            &identity.name,
            others.map(|(theirs, ..)| theirs),
        );
        match unready(&identity, spent, sheet) {
            Some(why) => session.message = why.into(),
            None if taken => {
                session.message = "A character with that nickname already exists.".into();
            }
            None => {
                commands.trigger(Introduce { draft, identity });
                session.candidates.push(draft);
                session.selected = Some(draft);
                session.mode = CreationMode::Party;
            }
        }
    }
}

fn update_name(
    event: On<TextEditChange>,
    names: Query<(&EditableText, Has<NicknameInput>), Or<(With<NameInput>, With<NicknameInput>)>>,
    session: Single<&Creation>,
    drafts: Query<&Identity>,
    mut commands: Commands,
) -> Result {
    let (name, nickname) = names.get(event.event_target())?;
    let Some(draft) = session.mode.draft() else { return Ok(()) };
    let mut identity = drafts.get(draft)?.clone();
    let value = name.value().to_string();
    let field = if nickname { &mut identity.name } else { &mut identity.full_name };
    if *field != value {
        *field = value;
        commands.trigger(Introduce { draft, identity });
    }
    Ok(())
}

fn finish_name(
    event: On<bevy::input_focus::FocusedInput<KeyboardInput>>,
    names: Query<&EditableText, Or<(With<NameInput>, With<NicknameInput>)>>,
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
    session: Single<Ref<Creation>>,
    mut editors: Query<(&mut Node, &mut Visibility), With<EditorGroup>>,
) {
    for (mut node, mut visibility) in &mut editors {
        let shown = session.mode.page() != Page::Party;
        crate::presentation::screen_art::display_if(&mut node, shown);
        visibility.set_if_neq(visible_if(shown));
    }
}

fn refresh(session: Single<Ref<Creation>>, mut pages: Query<(&Page, &mut Node, &mut Visibility)>) {
    for (page, mut node, mut visibility) in &mut pages {
        let shown = *page == session.mode.page();
        crate::presentation::screen_art::display_if(&mut node, shown);
        visibility.set_if_neq(visible_if(shown));
    }
}
// Retail portrait catalog; any portrait may be selected for any character.
