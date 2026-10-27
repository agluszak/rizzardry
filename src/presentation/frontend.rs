//! Retained frontend screens. Drafts belong to the creation root, never the campaign.
use super::screen_art::{Picture, absolute};
use crate::content::characters::{Attribute, Gender, Profession, Skill};
use crate::game::characters::creation::{CharacterDraft, PLAYABLE_RACES};
use crate::game::party::{formation::Formation, roster::PartyRoster};
use crate::game::session::{NewGameDestination, Session, begin_adventure};
#[cfg(test)]
use bevy::input::ButtonState;
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
use enumset::EnumSet;

pub struct FrontendPlugin;
impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TabNavigationPlugin)
            .add_observer(activate)
            .add_systems(OnEnter(Session::MainMenu), main_menu)
            .add_systems(OnEnter(Session::Creation), creation)
            .add_systems(Update, (refresh, button_colours).chain());
    }
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
    party: Vec<CharacterDraft>,
    editing: Option<(Option<usize>, CharacterDraft)>,
    page: Page,
    message: String,
}
#[derive(Component, Clone, Copy)]
enum Action {
    NewGame,
    Quit,
    Menu,
    Add,
    Edit(usize),
    Remove(usize),
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
    Slot(usize),
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
    inner
}
fn main_menu(mut commands: Commands) {
    let root = root(&mut commands, Session::MainMenu);
    text(&mut commands, root, (54., 58.), 530., 12., "A JOURNEY TO DOMINUS");
    text(&mut commands, root, (50., 90.), 540., 56., "RIZZARDRY");
    text(
        &mut commands,
        root,
        (54., 161.),
        480.,
        15.,
        "Six strangers. Three artifacts. One ascension.",
    );
    button(&mut commands, root, (54., 245.), 236., "NEW ADVENTURE", Action::NewGame);
    button(&mut commands, root, (54., 285.), 236., "QUIT", Action::Quit);
    text(
        &mut commands,
        root,
        (54., 348.),
        510.,
        12.,
        "Create your party, then begin on the shores of the monastery.\nSaved-game loading is not available yet.",
    );
    text(
        &mut commands,
        root,
        (54., 438.),
        530.,
        10.,
        "WIZARDRY 8  /  RUST + BEVY RECREATION  /  DEVELOPMENT BUILD",
    );
}
fn creation(mut commands: Commands, assets: Res<AssetServer>) {
    let root = root(&mut commands, Session::Creation);
    commands.entity(root).insert(Creation {
        party: Vec::new(),
        editing: None,
        page: Page::Party,
        message: String::new(),
    });
    text(&mut commands, root, (22., 16.), 590., 25., "ASSEMBLE YOUR PARTY");
    text(
        &mut commands,
        root,
        (22., 51.),
        580.,
        10.,
        "Create up to six adventurers. Nothing is committed until you begin your adventure.",
    );
    for i in 0..6 {
        let label = button(
            &mut commands,
            root,
            (22., 88. + i as f32 * 45.),
            139.,
            "Empty slot",
            Action::Edit(i),
        );
        commands.entity(label).insert(Field::Slot(i));
        button(&mut commands, root, (165., 88. + i as f32 * 45.), 22., "x", Action::Remove(i));
    }
    button(&mut commands, root, (22., 369.), 165., "+ NEW ADVENTURER", Action::Add);
    button(&mut commands, root, (22., 435.), 165., "DISCARD PARTY & BACK", Action::Menu);
    text(
        &mut commands,
        root,
        (205., 445.),
        410.,
        10.,
        "TAB / SHIFT+TAB to navigate / ENTER to activate",
    );
    let message = text(&mut commands, root, (205., 402.), 410., 10., "");
    commands.entity(message).insert(Field::Message);
    for page in [Page::Party, Page::Identity, Page::Attributes, Page::Skills, Page::Review] {
        let panel =
            commands.spawn((absolute(Vec2::new(205., 80.), Vec2::new(413., 310.)), page)).id();
        commands.entity(root).add_child(panel);
        match page {
            Page::Party => {
                text(&mut commands, panel, (0., 4.), 410., 24., "Your fellowship begins here");
                text(
                    &mut commands,
                    panel,
                    (0., 55.),
                    395.,
                    14.,
                    "Choose a race and profession, shape their attributes, and train their skills.\n\nSelect a party member to revise their draft. Canceling an edit keeps the previous version.\n\nOne adventurer is enough to begin. Six gives you more ways to survive.",
                );
                button(&mut commands, panel, (0., 275.), 240., "BEGIN ADVENTURE", Action::Begin);
            }
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
    state: Res<State<Session>>,
    mut focus: ResMut<InputFocus>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    mut roster: ResMut<PartyRoster>,
    mut formation: ResMut<Formation>,
    assets: Res<AssetServer>,
    destination: Res<NewGameDestination>,
    mut names: Query<&mut EditableText, With<NameInput>>,
) {
    let Ok(action) = buttons.get(click.entity) else { return };
    if matches!(
        action,
        Action::NewGame
            | Action::Menu
            | Action::Add
            | Action::Edit(_)
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
        Action::Menu => {
            next.set(Session::MainMenu);
            return;
        }
        _ => {}
    }
    let Ok(mut session) = sessions.single_mut() else { return };
    session.message.clear();
    match *action {
        Action::Add if session.party.len() < 6 => {
            session.editing = Some((None, CharacterDraft::default()));
            session.page = Page::Identity;
        }
        Action::Edit(i) => {
            if let Some(draft) = session.party.get(i).cloned() {
                session.editing = Some((Some(i), draft));
                session.page = Page::Identity;
            }
        }
        Action::Remove(i) => {
            if i < session.party.len() {
                session.party.remove(i);
                // An edit may refer to a shifted slot: discard the editing session.
                session.editing = None;
                session.page = Page::Party;
            }
        }
        Action::Cancel => {
            session.editing = None;
            session.page = Page::Party;
        }
        Action::Page(page) => session.page = page,
        Action::Finish => {
            if let Some((index, draft)) = &session.editing {
                match draft.finish() {
                    Err(message) => session.message = message.into(),
                    Ok(_) => {
                        let index = *index;
                        let (_, draft) = session.editing.take().unwrap();
                        if let Some(i) = index {
                            session.party[i] = draft;
                        } else {
                            session.party.push(draft);
                        }
                        session.page = Page::Party;
                    }
                }
            }
        }
        Action::Begin => {
            if let Err(message) = begin_adventure(
                &mut session.party,
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
            let Some((_, draft)) = session.editing.as_mut() else { return };
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
    if matches!(*action, Action::Add | Action::Edit(_))
        && let Some((_, draft)) = &session.editing
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
    if let Some((_, draft)) = session.editing.as_mut() {
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

fn refresh(
    sessions: Query<Ref<Creation>>,
    focus: Res<InputFocus>,
    mut commands: Commands,
    mut buttons: Query<(Entity, &Action, Has<InteractionDisabled>)>,
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
    let draft = session.editing.as_ref().map(|(_, d)| d);
    let profile = draft.map(CharacterDraft::preview);
    for (entity, action, disabled) in &mut buttons {
        let enabled = match *action {
            Action::Add => session.editing.is_none() && session.party.len() < 6,
            Action::Edit(i) | Action::Remove(i) => {
                session.editing.is_none() && i < session.party.len()
            }
            Action::Begin => session.editing.is_none() && !session.party.is_empty(),
            _ => true,
        };
        crate::presentation::retail_controls::set_enabled(&mut commands, entity, enabled, disabled);
    }

    for (field, mut text) in &mut fields {
        let value = match field {
            Field::Slot(i) => session
                .party
                .get(*i)
                .map(|d| {
                    let name: String = d.name.chars().take(16).collect();
                    format!(
                        "{}{}\n{:?}",
                        name,
                        if d.name.chars().count() > 16 { "..." } else { "" },
                        d.profession()
                    )
                })
                .unwrap_or_else(|| format!("{}  /  Empty", i + 1)),
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
        With<Action>,
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
    use bevy::input_focus::{InputDispatchPlugin, InputFocusPlugin};
    use bevy::ui_widgets::ButtonPlugin;
    use bevy::window::PrimaryWindow;

    fn tap(app: &mut App, window: Entity, key_code: KeyCode) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                state,
                text: None,
                repeat: false,
                window,
            });
            app.update();
        }
    }

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
                party: vec![],
                editing: Some((None, CharacterDraft::default())),
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
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().1.name,
            "Nora"
        );
        for edit in
            [TextEdit::TextStart(false), TextEdit::Right(true), TextEdit::Insert("Z".into())]
        {
            app.world_mut().get_mut::<EditableText>(field).unwrap().queue_edit(edit);
        }
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().1.name,
            "Zora"
        );
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .queue_edit(TextEdit::ImeSetCompose { value: "é".into(), cursor: None });
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().1.name,
            "Zora"
        );
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .queue_edit(TextEdit::ImeCommit { value: "é".into() });
        app.update();
        assert_eq!(
            app.world().get::<Creation>(root).unwrap().editing.as_ref().unwrap().1.name,
            "Zéora"
        );
        assert!(app.world().get::<Creation>(root).unwrap().party.is_empty());
    }

    #[test]
    fn frontend_can_reenter_tab_navigation_after_page_and_name_changes() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::input::InputPlugin,
            bevy::state::app::StatesPlugin,
            InputFocusPlugin,
            InputDispatchPlugin,
            ButtonPlugin,
            bevy::ui_widgets::TextInputPlugin,
            bevy::camera::visibility::VisibilityPlugin,
        ))
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
        tap(&mut app, window, KeyCode::Tab);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert!(matches!(app.world().get::<Action>(focused), Some(Action::NewGame)));
        tap(&mut app, window, KeyCode::Enter);
        app.update();
        assert_eq!(*app.world().resource::<State<Session>>().get(), Session::Creation);
        tap(&mut app, window, KeyCode::Tab);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert!(matches!(app.world().get::<Action>(focused), Some(Action::Add)));
        tap(&mut app, window, KeyCode::Enter);
        app.update();
        // The transition cleared focus. Tab still finds a visible enabled control.
        tap(&mut app, window, KeyCode::Tab);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert!(app.world().get::<Action>(focused).is_some());
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
        assert!(
            app.world()
                .resource::<InputFocus>()
                .get()
                .and_then(|e| app.world().get::<Action>(e))
                .is_some()
        );
    }
}
