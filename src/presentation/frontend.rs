//! Retained frontend screens. Drafts belong to the creation root, never the campaign.
use super::screen_art::{BitmapText, Picture, absolute, spawn_picture};
use crate::content::characters::{Attribute, Gender, Profession, Race, Skill};
use crate::content::ui::SpriteSheet;
use crate::game::characters::creation::{
    CharacterDraft, PARTY_LIMIT, PLAYABLE_RACES, admit, default_gender, identity,
};
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
            .add_observer(activate)
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
                    party::refresh,
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
#[derive(Component)]
struct Creation {
    candidates: Vec<(u32, CharacterDraft)>,
    party: Vec<u32>,
    selected: Option<u32>,
    next_id: u32,
    mode: CreationMode,
    choices: CreatorChoices,
    message: String,
}
enum CreationMode {
    Party,
    Editing { draft: Box<CharacterDraft>, step: EditStep },
}
#[derive(Clone, Copy)]
enum EditStep {
    Identity,
    Magic,
    Skills,
    Review,
}
impl CreationMode {
    fn draft(&self) -> Option<&CharacterDraft> {
        match self {
            Self::Party => None,
            Self::Editing { draft, .. } => Some(draft),
        }
    }
    fn draft_mut(&mut self) -> Option<&mut CharacterDraft> {
        match self {
            Self::Party => None,
            Self::Editing { draft, .. } => Some(draft),
        }
    }
    fn take_draft(&mut self) -> Option<CharacterDraft> {
        match std::mem::replace(self, Self::Party) {
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
#[derive(Component, Clone, Copy, PartialEq)]
enum Action {
    NewGame,
    Options,
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
        next_id: 0,
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
    buttons: Query<&Action, Without<InteractionDisabled>>,
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
        Action::Options => {
            menu.set(MenuPage::Options);
            screen.set(Screen::Options);
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
    let Ok((session_entity, mut session)) = sessions.single_mut() else { return };
    session.message.clear();
    if !matches!(action, Action::ToggleChoice(_)) {
        session.choices.open = None;
    }
    match *action {
        Action::Add if session.candidates.len() < 6 => {
            session.mode =
                CreationMode::Editing { draft: Box::default(), step: EditStep::Identity };
            session.choices = CreatorChoices::default();
            session.mode.set_page(Page::Identity);
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
                } else if session.party.len() < PARTY_LIMIT {
                    session.party.push(id);
                }
            }
        }
        Action::DeleteCandidate => {
            if let Some((id, draft)) = session
                .selected
                .and_then(|id| session.candidates.iter().find(|(candidate, _)| *candidate == id))
            {
                confirmation::open(&mut commands, &assets, *id, draft.display_name(), &mut focus);
            }
        }
        Action::Cancel => {
            session.mode = CreationMode::Party;
            session.mode.set_page(Page::Party);
        }
        Action::ToggleChoice(choice) => {
            session.choices.open =
                if session.choices.open == Some(choice) { None } else { Some(choice) };
        }
        Action::Page(Page::Skills) if session.mode.page() == Page::Identity => {
            if session.choices.complete()
                && session.mode.draft().is_some_and(|draft| {
                    !EnumSet::<Attribute>::all()
                        .iter()
                        .any(|attribute| draft.can_adjust_attribute(attribute, true))
                })
            {
                let page = if session.mode.draft().unwrap().spell_points_total() > 0 {
                    Page::Magic
                } else {
                    Page::Skills
                };
                session.mode.set_page(page);
            }
        }
        Action::Page(Page::Skills) if session.mode.page() == Page::Magic => {
            if session
                .mode
                .draft()
                .is_some_and(|draft| draft.spell_remaining() < draft.spell_points_total())
            {
                if session.mode.draft().is_some_and(|draft| draft.spell_remaining() > 0) {
                    confirmation::confirm_remaining_spells(&mut commands, &assets, &mut focus);
                } else {
                    session.mode.set_page(Page::Skills);
                }
            }
        }
        Action::Page(Page::Identity) if session.mode.page() == Page::Skills => {
            let page = if session.mode.draft().is_some_and(|draft| draft.spell_points_total() > 0) {
                Page::Magic
            } else {
                Page::Identity
            };
            session.mode.set_page(page);
        }
        Action::Page(Page::Review) if session.mode.page() == Page::Skills => {
            if session.mode.draft().is_some_and(|draft| {
                !EnumSet::<Skill>::all().iter().any(|skill| draft.can_adjust_skill(skill, true))
            }) {
                session.mode.set_page(Page::Review);
                let nickname =
                    session.mode.draft().is_some_and(|draft| !draft.name.trim().is_empty());
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
            let Some(drafts) = drafts else {
                session.message = "A party member is no longer available.".into();
                return;
            };
            commands.queue(move |world: &mut World| {
                if let Err(message) = begin_adventure(world, &drafts)
                    && let Some(mut session) = world.get_mut::<Creation>(session_entity)
                {
                    session.message = message.into();
                }
            });
        }
        action => {
            let choices = session.choices;
            let Some(draft) = session.mode.draft_mut() else { return };
            let result = match action {
                Action::ResetSpells => {
                    draft.reset_spells();
                    Ok(())
                }
                Action::Race(direction) => {
                    let i = if choices.race {
                        PLAYABLE_RACES.iter().position(|&r| r == draft.race()).unwrap() as i32
                    } else if direction > 0 {
                        -1
                    } else {
                        0
                    };
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
                Action::Gender(direction) => {
                    let gender = if !choices.gender {
                        if draft.profession() == Profession::Valkyrie || direction < 0 {
                            Gender::Female
                        } else {
                            Gender::Male
                        }
                    } else if draft.gender() == Gender::Male {
                        Gender::Female
                    } else {
                        Gender::Male
                    };
                    draft.choose(draft.race(), gender, draft.profession())
                }
                Action::Profession(direction) => {
                    let professions: Vec<_> = EnumSet::<Profession>::all().iter().collect();
                    let i = if choices.profession {
                        draft.profession() as i32
                    } else if direction > 0 {
                        -1
                    } else {
                        0
                    };
                    // Skip professions unavailable to this race and gender; selection is still validated.
                    let choice = (1..=15)
                        .map(|offset| professions[(i + direction * offset).rem_euclid(15) as usize])
                        .find(|&p| {
                            let gender = default_gender(p, draft.gender(), choices.gender);
                            CharacterDraft::budget(draft.race(), gender, p).is_ok()
                        })
                        .unwrap();
                    let gender = default_gender(choice, draft.gender(), choices.gender);
                    draft.choose(draft.race(), gender, choice)
                }
                Action::ChooseProfession(profession) => {
                    let gender = default_gender(profession, draft.gender(), choices.gender);
                    draft.choose(draft.race(), gender, profession)
                }
                Action::ChooseRace(race) => draft.choose(race, draft.gender(), draft.profession()),
                Action::ChooseGender(gender) => {
                    draft.choose(draft.race(), gender, draft.profession())
                }
                Action::Portrait(direction) => {
                    draft.portrait = identity::next_portrait(&draft.portrait, direction).into();
                    Ok(())
                }
                Action::PortraitGroup(direction) => {
                    draft.portrait =
                        identity::next_portrait_group(&draft.portrait, direction).into();
                    Ok(())
                }
                Action::Attribute(a, increase) => {
                    if draft.adjust_attribute(a, increase) {
                        Ok(())
                    } else {
                        Err("Attribute minimum, allocation limit or point budget reached.")
                    }
                }
                Action::ResetAttributes => {
                    draft.reset_attributes();
                    Ok(())
                }
                Action::ResetSkills => {
                    draft.reset_skills();
                    Ok(())
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
    if matches!(*action, Action::Add)
        && let Some(draft) = session.mode.draft()
    {
        for (_, mut name, nickname) in &mut names {
            name.clear();
            name.queue_edit(bevy::text::TextEdit::Insert(
                (if nickname { &draft.nickname } else { &draft.name }).as_str().into(),
            ));
        }
    }
}

fn commit_creation(
    mut sessions: Query<(Entity, &mut Creation), With<FinishPending>>,
    inputs: Query<(&EditableText, Has<NicknameInput>), Or<(With<NameInput>, With<NicknameInput>)>>,
    mut commands: Commands,
) {
    for (entity, mut session) in &mut sessions {
        if inputs.iter().any(|(input, _)| {
            !input.pending_edits.is_empty() || input.pending_paste.is_some() || input.is_composing()
        }) {
            continue;
        }
        if session.mode.page() != Page::Review || !session.choices.complete() {
            commands.entity(entity).remove::<FinishPending>();
            continue;
        }
        let Some(draft) = session.mode.draft_mut() else {
            commands.entity(entity).remove::<FinishPending>();
            continue;
        };
        for (input, nickname) in &inputs {
            if nickname {
                draft.nickname = input.value().to_string();
            } else {
                draft.name = input.value().to_string();
            }
        }
        let draft = session.mode.draft().unwrap();
        match admit(draft, session.candidates.iter().map(|(_, candidate)| candidate)) {
            Err(message) => session.message = message.into(),
            Ok(()) => {
                let draft = session.mode.take_draft().unwrap();
                let id = session.next_id;
                session.next_id += 1;
                session.candidates.push((id, draft));
                session.selected = Some(id);
                session.mode.set_page(Page::Party);
            }
        }
        commands.entity(entity).remove::<FinishPending>();
    }
}

fn update_name(
    event: On<TextEditChange>,
    names: Query<(&EditableText, Has<NicknameInput>), Or<(With<NameInput>, With<NicknameInput>)>>,
    mut sessions: Query<&mut Creation>,
) {
    let (Ok((name, nickname)), Ok(mut session)) =
        (names.get(event.event_target()), sessions.single_mut())
    else {
        return;
    };
    if let Some(draft) = session.mode.draft_mut() {
        let value = name.value().to_string();
        let field = if nickname { &mut draft.nickname } else { &mut draft.name };
        if *field != value {
            *field = value;
        }
    }
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
