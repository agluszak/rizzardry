//! Retail party assembly: the candidate list and the ordered party are separate.
use super::{Action, CharacterDraft, Creation, Page, absolute};
use crate::content::paths::{PortraitSize, portrait as path};
use crate::content::ui::{GENDER_NAMES, PROFESSION_NAMES, RACE_NAMES, SpriteSheet};
use crate::game::characters::{Character, Gender, Profession, Race};
use crate::presentation::retail_controls::{ControlSkin, FocusHighlight, FocusRing, set_enabled};
use crate::presentation::screen_art::visible_if;
use crate::presentation::screen_art::{BitmapText, GameStrings, Picture, spawn_picture as picture};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Button;

#[derive(Component)]
pub(super) struct Control;
#[derive(Component)]
pub(super) struct CandidateView {
    id: u32,
    joined: bool,
}
#[derive(Component)]
pub(super) struct AssemblyPage;
/// The selected candidate's portrait, details and summary, shown together.
#[derive(Component)]
pub(super) struct SelectedPanel;
#[derive(Component)]
pub(super) struct SelectedPortrait;
/// A row's background, lit while its candidate is selected.
#[derive(Component)]
pub(super) struct RowBackground;
/// A party slot's highlight, shown while its member is selected.
#[derive(Component)]
pub(super) struct Highlight;
/// The page's text that follows the session.
#[derive(Component)]
pub(super) enum SessionText {
    Message,
    Toggle,
    Joined,
    Summary(usize),
}

fn label(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    value: &str,
    at: Vec2,
    width: f32,
) -> Entity {
    commands
        .spawn((
            BitmapText { value: value.into(), ..BitmapText::centred(font, at, width) },
            ChildOf(parent),
        ))
        .id()
}
fn action_button(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    at: Vec2,
    frame: usize,
    size: Vec2,
    text: &str,
    action: Option<Action>,
) -> Entity {
    let mut button = commands.spawn((
        Control,
        Button,
        TabIndex(0),
        FocusHighlight,
        ControlSkin::new(frame, frame + 1, frame + 2, frame + 2, frame + 3),
        Picture { sheet, frame, at, bounds: Some(frame) },
        absolute(at, size),
        Pickable::default(),
        ChildOf(parent),
    ));
    if let Some(action) = action {
        button.insert(action);
    } else {
        button.insert((InteractionDisabled, TabIndex(-1)));
    }
    let button = button.id();
    if !text.is_empty() {
        label(commands, button, font, text, Vec2::new(0., 7.), size.x);
    }
    button
}

pub(super) fn build(commands: &mut Commands, root: Entity, assets: &AssetServer) {
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let back = load("party generation/back");
    let buttons = load("party generation/listbuttons");
    let detail = load("party generation/detailbox");
    let bottom = load("char generation/cg_bottombuttons");
    let font = load("fonts/wiz_text_font_bold");
    let small = load("fonts/wiz_text_font");
    let scroll = load("main interface/main_scroll");
    let page = commands
        .spawn((
            Page::Party,
            AssemblyPage,
            absolute(Vec2::ZERO, Vec2::new(640., 480.)),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    picture(commands, page, back, 0, Vec2::ZERO);
    label(commands, page, font.clone(), "Gather Your Party", Vec2::new(123., 9.), 394.);
    commands.spawn((
        absolute(Vec2::new(489., 65.), Vec2::new(16., 218.)),
        BackgroundColor(Color::BLACK),
        Pickable::IGNORE,
        ChildOf(page),
    ));
    picture(commands, page, scroll.clone(), 3, Vec2::new(489., 49.));
    picture(commands, page, scroll, 11, Vec2::new(489., 283.));
    let panel = commands
        .spawn((
            SelectedPanel,
            absolute(Vec2::ZERO, Vec2::new(640., 480.)),
            Pickable::IGNORE,
            Visibility::Hidden,
            ChildOf(page),
        ))
        .id();
    let portrait = picture(commands, panel, Handle::default(), 0, Vec2::new(133., 48.));
    commands.entity(portrait).insert(SelectedPortrait);
    let joined =
        label(commands, panel, load("fonts/opt_title_font"), "", Vec2::new(133., 166.), 180.);
    commands.entity(joined).insert(SessionText::Joined);
    picture(commands, panel, detail, 0, Vec2::new(127., 197.));
    for (index, (at, width)) in [
        (Vec2::new(132., 214.), 190.),
        (Vec2::new(132., 240.), 190.),
        (Vec2::new(132., 254.), 190.),
        (Vec2::new(150., 295.), 0.),
        (Vec2::new(150., 309.), 0.),
        (Vec2::new(150., 323.), 0.),
        (Vec2::new(150., 337.), 0.),
        (Vec2::new(150., 351.), 0.),
        (Vec2::new(150., 365.), 0.),
        (Vec2::new(150., 379.), 0.),
        (Vec2::new(227., 295.), 0.),
        (Vec2::new(227., 323.), 0.),
        (Vec2::new(227., 337.), 0.),
    ]
    .into_iter()
    .enumerate()
    {
        let summary = label(commands, panel, small.clone(), "", at, width);
        commands.entity(summary).insert(SessionText::Summary(index));
    }
    let message = label(commands, page, small, "", Vec2::new(132., 421.), 230.);
    commands.entity(message).insert(SessionText::Message);
    for (y, text, action) in [
        (311., "Create Character", Some(Action::Add)),
        (337., "", Some(Action::ToggleCandidate)),
        (363., "Delete Character", Some(Action::DeleteCandidate)),
        (389., "Review Character", None),
    ] {
        let size = Vec2::new(186., 24.);
        let toggles = matches!(action, Some(Action::ToggleCandidate));
        let button = action_button(
            commands,
            page,
            buttons.clone(),
            font.clone(),
            Vec2::new(325., y),
            0,
            size,
            text,
            action,
        );
        if toggles {
            let label = label(commands, button, font.clone(), "", Vec2::new(0., 7.), size.x);
            commands.entity(label).insert(SessionText::Toggle);
        }
    }
    action_button(
        commands,
        page,
        bottom.clone(),
        font.clone(),
        Vec2::new(420., 437.),
        0,
        Vec2::new(44., 30.),
        "",
        Some(Action::Begin),
    );
    action_button(
        commands,
        page,
        bottom,
        font,
        Vec2::new(464., 437.),
        4,
        Vec2::new(44., 30.),
        "",
        Some(Action::Menu),
    );
}

fn candidate(session: &Creation, id: u32) -> Option<&CharacterDraft> {
    session.candidates.iter().find_map(|(candidate, draft)| (*candidate == id).then_some(draft))
}

/// "Level 1 Fighter", as a candidate's description reads.
fn profession_line(strings: &GameStrings, profession: Profession) -> String {
    format!("Level 1 {}", strings.get(PROFESSION_NAMES + profession as usize))
}

/// "Male Human", as a candidate's description reads.
fn kind_line(strings: &GameStrings, gender: Gender, race: Race) -> String {
    let gender = strings.get(GENDER_NAMES + gender as usize);
    format!("{gender} {}", strings.get(RACE_NAMES + race as usize))
}

/// A joined member's portrait, frames and name.
fn spawn_member_parts(
    commands: &mut Commands,
    root: Entity,
    draft: &CharacterDraft,
    assets: &AssetServer,
) {
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let portrait = assets.load(path(PortraitSize::Medium, &draft.portrait));
    picture(commands, root, portrait, 0, Vec2::ZERO);
    let border = load("party generation/portraitborder");
    picture(commands, root, border, 0, Vec2::new(-13., -11.));
    let highlight = load("party generation/portraithighlight");
    let highlight = picture(commands, root, highlight, 1, Vec2::ZERO);
    commands.entity(highlight).insert((Highlight, Visibility::Hidden));
    let font = load("fonts/wiz_text_font");
    label(commands, root, font, draft.display_name(), Vec2::new(-10., 79.), 110.);
}

/// A candidate's list row: its portrait and description.
fn spawn_row_parts(
    commands: &mut Commands,
    root: Entity,
    draft: &CharacterDraft,
    assets: &AssetServer,
    strings: &GameStrings,
) {
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let background = picture(commands, root, load("party generation/listboxes"), 0, Vec2::ZERO);
    commands.entity(background).insert(RowBackground);
    let portrait = assets.load(path(PortraitSize::Small, &draft.portrait));
    picture(commands, root, portrait, 0, Vec2::new(2., 2.));
    for (text, y) in [
        (draft.display_name().to_owned(), 3.),
        (profession_line(strings, draft.profession()), 14.),
        (kind_line(strings, draft.gender(), draft.race()), 25.),
    ] {
        label(commands, root, load("fonts/wiz_text_font"), &text, Vec2::new(53., y), 0.);
    }
}

/// Candidate IDs are stable across deletion and party reordering, and a
/// candidate does not change once admitted: a view is made for one candidate
/// and written once. Only its position changes.
pub(super) fn sync_views(
    mut commands: Commands,
    session: Option<Single<&Creation>>,
    page: Query<Entity, With<AssemblyPage>>,
    mut views: Query<(Entity, &CandidateView, &mut Node)>,
    assets: Res<AssetServer>,
    strings: GameStrings,
) {
    let (Some(session), Ok(page), Some(_)) = (session, page.single(), strings.table()) else {
        return;
    };
    for (entity, view, _) in &views {
        if candidate(&session, view.id).is_none()
            || (view.joined && !session.party.contains(&view.id))
        {
            commands.entity(entity).despawn();
        }
    }
    for joined in [false, true] {
        let ids: Vec<_> = if joined {
            session.party.clone()
        } else {
            session.candidates.iter().map(|(id, _)| *id).collect()
        };
        for (index, id) in ids.into_iter().enumerate() {
            let at = if joined {
                Vec2::new(if index % 2 == 0 { 14. } else { 537. }, 58. + (index / 2) as f32 * 130.)
            } else {
                Vec2::new(328., 49. + index as f32 * 42.)
            };
            if let Some((_, _, mut node)) =
                views.iter_mut().find(|(_, v, _)| v.id == id && v.joined == joined)
            {
                if node.left != px(at.x) {
                    node.left = px(at.x);
                }
                if node.top != px(at.y) {
                    node.top = px(at.y);
                }
                continue;
            }
            let Some(draft) = candidate(&session, id) else { continue };
            let root = commands
                .spawn((
                    Control,
                    CandidateView { id, joined },
                    Button,
                    TabIndex(0),
                    // Keyboard focus shows, as on the page's buttons.
                    FocusRing,
                    Hovered::default(),
                    Action::SelectCandidate(Some(id)),
                    absolute(at, if joined { Vec2::new(90., 71.) } else { Vec2::new(158., 40.) }),
                    Pickable::default(),
                    ChildOf(page),
                ))
                .id();
            if joined {
                spawn_member_parts(&mut commands, root, draft, &assets);
            } else {
                spawn_row_parts(&mut commands, root, draft, &assets, &strings);
            }
        }
    }
}

/// Whether the page's views have something new to show.
pub(super) fn outdated(session: Option<Single<Ref<Creation>>>, strings: GameStrings) -> bool {
    strings.is_changed() || session.is_some_and(|session| session.is_changed())
}

pub(super) fn refresh_controls(
    session: Single<&Creation>,
    controls: Query<(Entity, &Action, Has<InteractionDisabled>), With<Control>>,
    mut commands: Commands,
) {
    for (entity, action, disabled) in &controls {
        let enabled = match *action {
            Action::Add => session.candidates.len() < 6,
            Action::ToggleCandidate => session.selected.is_some_and(|id| {
                session.party.contains(&id)
                    || session.party.len() < crate::game::characters::creation::PARTY_LIMIT
            }),
            Action::DeleteCandidate => session.selected.is_some(),
            Action::SelectCandidate(id) => id.is_some(),
            Action::Begin => !session.party.is_empty(),
            _ => true,
        };
        set_enabled(&mut commands, entity, enabled, disabled);
    }
}

/// Mark the selected candidate's list row and party slot.
pub(super) fn refresh_selection_marks(
    session: Single<&Creation>,
    views: Query<&CandidateView>,
    mut rows: Query<(&ChildOf, &mut Picture), With<RowBackground>>,
    mut highlights: Query<(&ChildOf, &mut Visibility), With<Highlight>>,
) {
    let selected = |part: &ChildOf| {
        views.get(part.parent()).is_ok_and(|view| session.selected == Some(view.id))
    };
    for (part, mut picture) in &mut rows {
        let frame = usize::from(selected(part));
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
    for (part, mut visibility) in &mut highlights {
        visibility.set_if_neq(visible_if(selected(part)));
    }
}

/// What the summary panel says of a candidate, line by line.
fn summary_lines(profile: &Character, strings: &GameStrings) -> [String; 13] {
    use crate::content::characters::Attribute::*;
    let stats = profile.base_stats();
    [
        profile.name.clone(),
        profession_line(strings, profile.profession),
        kind_line(strings, profile.gender, profile.race),
        format!("Str: {}", stats.attribute(Strength)),
        format!("Int: {}", stats.attribute(Intelligence)),
        format!("Pie: {}", stats.attribute(Piety)),
        format!("Vit: {}", stats.attribute(Vitality)),
        format!("Dex: {}", stats.attribute(Dexterity)),
        format!("Spd: {}", stats.attribute(Speed)),
        format!("Sen: {}", stats.attribute(Senses)),
        format!("HP: {}", stats.max_hp()),
        format!("Stam: {}", stats.max_stamina()),
        format!("Carry: {}", stats.capacity() / 10),
    ]
}

/// Show the selected candidate and the session's messages.
pub(super) fn refresh_selected(
    session: Single<&Creation>,
    assets: Res<AssetServer>,
    strings: GameStrings,
    mut panels: Query<&mut Visibility, With<SelectedPanel>>,
    mut portraits: Query<&mut Picture, With<SelectedPortrait>>,
    mut texts: Query<(&SessionText, &mut BitmapText)>,
) {
    let selected = session.selected.and_then(|id| candidate(&session, id));
    for mut visibility in &mut panels {
        visibility.set_if_neq(visible_if(selected.is_some()));
    }
    if let Some(draft) = selected {
        for mut picture in &mut portraits {
            let portrait = path(PortraitSize::Large, &draft.portrait);
            crate::presentation::screen_art::show_sheet(&mut picture, &assets, portrait);
        }
    }
    let joined = session.selected.is_some_and(|id| session.party.contains(&id));
    let summary = selected.map(|draft| summary_lines(&draft.preview(), &strings));
    for (kind, mut text) in &mut texts {
        let value = match kind {
            SessionText::Message => session.message.as_str(),
            SessionText::Toggle if joined => "Remove from Party",
            SessionText::Toggle => "Add to Party",
            SessionText::Joined if joined => "Joined",
            SessionText::Joined => "",
            SessionText::Summary(line) => summary.as_ref().map_or("", |lines| &lines[*line]),
        };
        if text.value != value {
            text.value = value.to_owned();
        }
    }
}

pub(super) fn paint_labels(
    assets: Res<AssetServer>,
    mut gray: Local<Option<Handle<SpriteSheet>>>,
    buttons: Query<(Has<InteractionDisabled>, &Children), (With<Control>, With<ControlSkin>)>,
    mut text: Query<&mut BitmapText>,
) {
    let gray = gray.get_or_insert_with(|| assets.load("retail://data/fonts/palettegray.sti"));
    for (disabled, children) in &buttons {
        for child in children.iter() {
            if let Ok(mut label) = text.get_mut(child) {
                let palette = disabled.then(|| gray.clone());
                if label.palette != palette {
                    label.palette = palette;
                }
            }
        }
    }
}
