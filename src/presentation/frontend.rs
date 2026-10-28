//! Retained frontend screens. Drafts belong to the creation root, never the campaign.
use super::screen_art::{BitmapText, Picture, absolute, spawn_picture};
use crate::content::characters::{Attribute, Gender, Profession, Race, Skill};
use crate::content::ui::SpriteSheet;
use crate::game::characters::creation::{
    CharacterDraft, NAME_LIMIT, NICKNAME_LIMIT, PARTY_LIMIT, PLAYABLE_RACES, admit, default_gender,
    identity,
};
use crate::game::session::{Campaign, NewGameDestination, Session, begin_adventure};
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

pub struct FrontendPlugin;
impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TabNavigationPlugin)
            .add_sub_state::<MenuPage>()
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
                    report_entry_failure,
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
                    .run_if(not(in_state(crate::game::session::Session::Playing))),
            );
    }
}

/// The first level could not be prepared: the party stays in creation to
/// try again.
fn report_entry_failure(
    mut failures: MessageReader<crate::game::world::level::LevelEntryFailed>,
    mut sessions: Query<&mut Creation>,
) {
    for failure in failures.read() {
        for mut session in &mut sessions {
            session.message = format!("{} could not be loaded.", failure.path);
        }
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
enum MenuPage {
    #[default]
    Main,
    Credits,
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
    identity_initialized: bool,
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
    state: Res<State<Session>>,
    mut focus: ResMut<InputFocus>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    campaign: Campaign,
    assets: Res<AssetServer>,
    destination: Res<NewGameDestination>,
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
                if !session.choices.identity_initialized {
                    if let Some(draft) = session.mode.draft_mut() {
                        identity::initialize(draft);
                    }
                    session.choices.identity_initialized = true;
                }
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
            if let Err(message) =
                begin_adventure(&drafts, &state, &campaign, &mut commands, &assets, &destination)
            {
                session.message = message.into();
            }
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
                if matches!(
                    action,
                    Action::Race(_)
                        | Action::Gender(_)
                        | Action::Profession(_)
                        | Action::ChooseRace(_)
                        | Action::ChooseGender(_)
                        | Action::ChooseProfession(_)
                ) {
                    session.choices.identity_initialized = false;
                }
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
            *name = EditableText {
                max_characters: Some(if nickname { NICKNAME_LIMIT } else { NAME_LIMIT }),
                allow_newlines: false,
                ..EditableText::new(if nickname { &draft.nickname } else { &draft.name })
            };
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
    session: Option<Single<Ref<Creation>>>,
    mut editors: Query<(&mut Node, &mut Visibility), With<EditorGroup>>,
) {
    let Some(session) = session else { return };
    for (mut node, mut visibility) in &mut editors {
        let shown = session.mode.page() != Page::Party;
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        visibility.set_if_neq(crate::presentation::screen_art::shown(shown));
    }
}

fn refresh(
    session: Option<Single<Ref<Creation>>>,
    mut pages: Query<(&Page, &mut Node, &mut Visibility)>,
) {
    let Some(session) = session else { return };
    for (page, mut node, mut visibility) in &mut pages {
        let shown = *page == session.mode.page();
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        visibility.set_if_neq(crate::presentation::screen_art::shown(shown));
    }
}
// Retail portrait catalog; any portrait may be selected for any character.

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
                mode: CreationMode::Editing {
                    draft: {
                        let mut draft = CharacterDraft::default();
                        crate::game::characters::creation::complete_allocations(&mut draft);
                        Box::new(draft)
                    },
                    step: EditStep::Identity,
                },
                choices: CreatorChoices {
                    profession: true,
                    race: true,
                    gender: true,
                    open: None,
                    identity_initialized: true,
                },
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
        assert_eq!(app.world().get::<Creation>(root).unwrap().mode.draft().unwrap().name, "Nora");
        for edit in
            [TextEdit::TextStart(false), TextEdit::Right(true), TextEdit::Insert("Z".into())]
        {
            app.world_mut().get_mut::<EditableText>(field).unwrap().queue_edit(edit);
        }
        app.update();
        assert_eq!(app.world().get::<Creation>(root).unwrap().mode.draft().unwrap().name, "Zora");
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .queue_edit(TextEdit::ImeSetCompose { value: "é".into(), cursor: None });
        app.update();
        assert_eq!(app.world().get::<Creation>(root).unwrap().mode.draft().unwrap().name, "Zora");
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .queue_edit(TextEdit::ImeCommit { value: "é".into() });
        app.update();
        assert_eq!(app.world().get::<Creation>(root).unwrap().mode.draft().unwrap().name, "Zéora");
        assert!(app.world().get::<Creation>(root).unwrap().party.is_empty());

        // Finish must consume the final queued edit, not the previous name.
        app.add_systems(PostUpdate, commit_creation.after(apply_text_edits));
        app.world_mut().spawn((NicknameInput, EditableText::new("Kai")));
        app.world_mut().get_mut::<Creation>(root).unwrap().mode.set_page(Page::Review);
        for edit in [TextEdit::SelectAll, TextEdit::Insert("Kai the Brave".into())] {
            app.world_mut().get_mut::<EditableText>(field).unwrap().queue_edit(edit);
        }
        app.world_mut().entity_mut(root).insert(FinishPending);
        app.update();
        let session = app.world().get::<Creation>(root).unwrap();
        assert_eq!(session.candidates[0].1.name, "Kai the Brave");
        assert_eq!(session.candidates[0].1.display_name(), "Kai");
        assert!(session.mode.draft().is_none());
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
        .init_asset::<crate::content::spells::SpellCatalog>()
        .init_asset::<crate::content::spells::SpellText>()
        .init_asset::<Image>()
        .init_state::<Session>()
        .add_message::<crate::game::world::level::LevelEntryFailed>()
        .init_asset::<crate::content::ui::SpriteSheet>()
        .init_asset::<Mesh>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .add_message::<bevy::window::Ime>()
        .add_message::<bevy::picking::events::PointerRelease>()
        .init_resource::<bevy::picking::events::PointerState>()
        .init_resource::<UiScale>()
        .insert_resource(NewGameDestination { path: String::new(), start: default() })
        .add_message::<AppExit>()
        .add_plugins(FrontendPlugin)
        .add_observer(crate::presentation::retail_controls::primary_buttons_only)
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
        session.mode = CreationMode::Party;
        session.mode.set_page(Page::Party);
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
