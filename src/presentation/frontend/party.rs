//! Retail party assembly: the candidate list and the ordered party are separate.
use super::{Action, CharacterDraft, Creation, Page, absolute};
use crate::content::ui::SpriteSheet;
use crate::presentation::retail_controls::set_enabled;
use crate::presentation::screen_art::{BitmapText, Picture, spawn_picture as picture};
use bevy::input_focus::{InputFocus, tab_navigation::TabIndex};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::Button;

#[derive(Component)]
pub(super) struct Control;
#[derive(Component)]
pub(super) struct CandidateRow(usize);
#[derive(Component)]
pub(super) struct PartySlot(usize);
#[derive(Component)]
pub(super) struct ButtonArt {
    normal: usize,
}
#[derive(Component)]
pub(super) struct ToggleLabel;
#[derive(Component)]
pub(super) struct PartyMessage;
#[derive(Component)]
pub(super) struct RowBackground(usize);
#[derive(Component)]
pub(super) struct SelectedDetail;
#[derive(Component)]
pub(super) struct SlotFrame {
    index: usize,
    highlight: bool,
}
#[derive(Component)]
pub(super) enum Portrait {
    Candidate(usize),
    Member(usize),
    Selected,
}
#[derive(Component)]
pub(super) enum Label {
    CandidateName(usize),
    CandidateClass(usize),
    CandidateRace(usize),
    Member(usize),
    Joined,
    Summary(u8),
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
            BitmapText {
                font,
                value: value.into(),
                at,
                width,
                justify: bevy::text::Justify::Center,
                ..default()
            },
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
        Hovered::default(),
        ButtonArt { normal: frame },
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
    picture(commands, button, sheet, frame, Vec2::ZERO);
    if !text.is_empty() {
        label(commands, button, font, text, Vec2::new(0., 7.), size.x);
    }
    button
}

pub(super) fn build(commands: &mut Commands, root: Entity, assets: &AssetServer) {
    let load = |path: &str| assets.load(format!("retail://data/{path}.sti"));
    let back = load("party generation/back");
    let list = load("party generation/listboxes");
    let border = load("party generation/portraitborder");
    let highlight = load("party generation/portraithighlight");
    let buttons = load("party generation/listbuttons");
    let detail = load("party generation/detailbox");
    let bottom = load("char generation/cg_bottombuttons");
    let font = load("fonts/wiz_text_font_bold");
    let small = load("fonts/wiz_text_font");
    let scroll = load("main interface/main_scroll");
    let page = commands
        .spawn((
            Page::Party,
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
    for i in 0..6 {
        let x = if i % 2 == 0 { 14. } else { 537. };
        let y = 58. + (i / 2) as f32 * 130.;
        let slot = commands
            .spawn((
                Control,
                PartySlot(i),
                Button,
                TabIndex(0),
                Hovered::default(),
                Action::SelectCandidate(None),
                InteractionDisabled,
                absolute(Vec2::new(x, y), Vec2::new(90., 71.)),
                Pickable::default(),
                ChildOf(page),
            ))
            .id();
        let portrait = picture(commands, slot, Handle::default(), 0, Vec2::ZERO);
        commands.entity(portrait).insert((Portrait::Member(i), Visibility::Hidden));
        let frame = picture(commands, slot, border.clone(), 0, Vec2::new(-13., -11.));
        commands
            .entity(frame)
            .insert((SlotFrame { index: i, highlight: false }, Visibility::Hidden));
        let frame = picture(commands, slot, highlight.clone(), 1, Vec2::ZERO);
        commands
            .entity(frame)
            .insert((SlotFrame { index: i, highlight: true }, Visibility::Hidden));
        let name = label(commands, page, small.clone(), "", Vec2::new(x - 10., y + 79.), 110.);
        commands.entity(name).insert(Label::Member(i));
    }
    for i in 0..6 {
        let at = Vec2::new(328., 49. + i as f32 * 42.);
        let row = commands
            .spawn((
                Control,
                CandidateRow(i),
                Button,
                TabIndex(0),
                Hovered::default(),
                Action::SelectCandidate(None),
                InteractionDisabled,
                absolute(at, Vec2::new(158., 40.)),
                Pickable::default(),
                ChildOf(page),
            ))
            .id();
        let background = picture(commands, row, list.clone(), 0, Vec2::ZERO);
        commands.entity(background).insert((RowBackground(i), Visibility::Hidden));
        let portrait = picture(commands, row, Handle::default(), 0, Vec2::new(2., 2.));
        commands.entity(portrait).insert((Portrait::Candidate(i), Visibility::Hidden));
        for (part, y) in [
            (Label::CandidateName(i), 3.),
            (Label::CandidateClass(i), 14.),
            (Label::CandidateRace(i), 25.),
        ] {
            let text = label(commands, row, small.clone(), "", Vec2::new(53., y), 0.);
            commands.entity(text).insert(part);
        }
    }
    let portrait = picture(commands, page, Handle::default(), 0, Vec2::new(133., 48.));
    commands.entity(portrait).insert((Portrait::Selected, Visibility::Hidden));
    let joined =
        label(commands, page, load("fonts/opt_title_font"), "", Vec2::new(133., 166.), 180.);
    commands.entity(joined).insert(Label::Joined);
    let detail = picture(commands, page, detail, 0, Vec2::new(127., 197.));
    commands.entity(detail).insert((SelectedDetail, Visibility::Hidden));
    for (index, at, width) in [
        (0, Vec2::new(132., 214.), 190.),
        (1, Vec2::new(132., 240.), 190.),
        (2, Vec2::new(132., 254.), 190.),
        (3, Vec2::new(150., 295.), 0.),
        (4, Vec2::new(150., 309.), 0.),
        (5, Vec2::new(150., 323.), 0.),
        (6, Vec2::new(150., 337.), 0.),
        (7, Vec2::new(150., 351.), 0.),
        (8, Vec2::new(150., 365.), 0.),
        (9, Vec2::new(150., 379.), 0.),
        (10, Vec2::new(227., 295.), 0.),
        (11, Vec2::new(227., 309.), 0.),
        (12, Vec2::new(227., 323.), 0.),
        (13, Vec2::new(227., 337.), 0.),
    ] {
        let summary = label(commands, page, small.clone(), "", at, width);
        commands.entity(summary).insert(Label::Summary(index));
    }
    let message = label(commands, page, small, "", Vec2::new(132., 421.), 230.);
    commands.entity(message).insert(PartyMessage);
    for (y, text, action) in [
        (311., "Create Character", Some(Action::Add)),
        (337., "Add to Party", Some(Action::ToggleCandidate)),
        (363., "Delete Character", Some(Action::DeleteCandidate)),
        (389., "Review Character", None),
    ] {
        let button = action_button(
            commands,
            page,
            buttons.clone(),
            font.clone(),
            Vec2::new(325., y),
            0,
            Vec2::new(186., 24.),
            text,
            action,
        );
        if y == 337. {
            commands.entity(button).insert(ToggleLabel);
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

pub(super) fn refresh(
    session: Option<Single<Ref<Creation>>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    mut controls: Query<
        (Entity, &mut Action, Option<&CandidateRow>, Option<&PartySlot>, Has<InteractionDisabled>),
        With<Control>,
    >,
    mut portraits: Query<(&Portrait, &mut Picture, &mut Visibility), Without<RowBackground>>,
    mut rows: Query<(&RowBackground, &mut Picture, &mut Visibility), Without<Portrait>>,
    mut details: Query<
        &mut Visibility,
        (With<SelectedDetail>, Without<Portrait>, Without<RowBackground>, Without<SlotFrame>),
    >,
    mut frames: Query<
        (&SlotFrame, &mut Visibility),
        (Without<Portrait>, Without<RowBackground>, Without<SelectedDetail>),
    >,
    mut labels: Query<(&Label, &mut BitmapText), Without<PartyMessage>>,
    mut messages: Query<&mut BitmapText, (With<PartyMessage>, Without<Label>)>,
    mut toggles: Query<&Children, With<ToggleLabel>>,
    mut toggle_text: Query<&mut BitmapText, (Without<Label>, Without<PartyMessage>)>,
) {
    let Some(session) = session else { return };
    if !session.is_changed() {
        return;
    }
    for (entity, mut action, row, slot, disabled) in &mut controls {
        if let Some(row) = row {
            *action = Action::SelectCandidate(session.candidates.get(row.0).map(|(id, _)| *id));
        } else if let Some(slot) = slot {
            *action = Action::SelectCandidate(session.party.get(slot.0).copied());
        }
        let enabled = match *action {
            Action::Add => session.candidates.len() < 6,
            Action::ToggleCandidate => session
                .selected
                .is_some_and(|id| session.party.contains(&id) || session.party.len() < 6),
            Action::DeleteCandidate => session.selected.is_some(),
            Action::SelectCandidate(id) => id.is_some(),
            Action::Begin => !session.party.is_empty(),
            _ => true,
        };
        set_enabled(&mut commands, entity, enabled, disabled);
    }
    for (portrait, mut picture, mut visible) in &mut portraits {
        let draft = match portrait {
            Portrait::Candidate(i) => session.candidates.get(*i).map(|(_, draft)| draft),
            Portrait::Member(i) => session.party.get(*i).and_then(|id| candidate(&session, *id)),
            Portrait::Selected => session.selected.and_then(|id| candidate(&session, id)),
        };
        visible.set_if_neq(if draft.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(draft) = draft {
            let (directory, prefix) = match portrait {
                Portrait::Candidate(_) => ("small", "s"),
                Portrait::Member(_) => ("medium", "m"),
                Portrait::Selected => ("large", "l"),
            };
            picture.sheet = assets.load(format!(
                "retail://data/portraits/{directory}/{prefix}{}.sti",
                draft.portrait
            ));
        }
    }
    for (row, mut picture, mut visible) in &mut rows {
        let id = session.candidates.get(row.0).map(|(id, _)| *id);
        visible.set_if_neq(if id.is_some() { Visibility::Inherited } else { Visibility::Hidden });
        picture.frame = usize::from(id.is_some() && id == session.selected);
    }
    for mut visible in &mut details {
        visible.set_if_neq(if session.selected.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for (frame, mut visible) in &mut frames {
        let id = session.party.get(frame.index).copied();
        let shown = id.is_some() && (!frame.highlight || id == session.selected);
        visible.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (field, mut text) in &mut labels {
        let value = match field {
            Label::CandidateName(i) => {
                session.candidates.get(*i).map(|(_, draft)| draft.name.clone())
            }
            Label::CandidateClass(i) => session
                .candidates
                .get(*i)
                .map(|(_, draft)| format!("Level 1 {:?}", draft.profession())),
            Label::CandidateRace(i) => session
                .candidates
                .get(*i)
                .map(|(_, draft)| format!("{:?} {:?}", draft.gender(), draft.race())),
            Label::Member(i) => session
                .party
                .get(*i)
                .and_then(|id| candidate(&session, *id))
                .map(|draft| draft.name.clone()),
            Label::Joined => session
                .selected
                .filter(|id| session.party.contains(id))
                .map(|_| "Joined".to_owned()),
            Label::Summary(line) => {
                session.selected.and_then(|id| candidate(&session, id)).map(|draft| {
                    let profile = draft.preview();
                    use crate::content::characters::Attribute::*;
                    match line {
                        0 => draft.name.clone(),
                        1 => format!("Level 1 {:?}", draft.profession()),
                        2 => format!("{:?} {:?}", draft.gender(), draft.race()),
                        3 => format!("Str: {}", profile.attribute(Strength)),
                        4 => format!("Int: {}", profile.attribute(Intelligence)),
                        5 => format!("Pie: {}", profile.attribute(Piety)),
                        6 => format!("Vit: {}", profile.attribute(Vitality)),
                        7 => format!("Dex: {}", profile.attribute(Dexterity)),
                        8 => format!("Spd: {}", profile.attribute(Speed)),
                        9 => format!("Sen: {}", profile.attribute(Senses)),
                        10 => format!("HP: {}", profile.max_hp()),
                        11 => String::new(),
                        12 => format!("Stam: {}", profile.max_stamina()),
                        13 => format!("Carry: {}", profile.capacity() / 10),
                        _ => String::new(),
                    }
                })
            }
        }
        .unwrap_or_default();
        if text.value != value {
            text.value = value;
        }
    }
    for mut message in &mut messages {
        if message.value != session.message {
            message.value.clone_from(&session.message);
        }
    }
    for children in &mut toggles {
        for child in children.iter() {
            if let Ok(mut text) = toggle_text.get_mut(child) {
                let value = if session.selected.is_some_and(|id| session.party.contains(&id)) {
                    "Remove from Party"
                } else {
                    "Add to Party"
                };
                if text.value != value {
                    text.value = value.into();
                }
            }
        }
    }
}

pub(super) fn paint(
    focus: Res<InputFocus>,
    assets: Res<AssetServer>,
    mut gray: Local<Option<Handle<SpriteSheet>>>,
    buttons: Query<(
        Entity,
        &ButtonArt,
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        &Children,
    )>,
    mut pictures: Query<&mut Picture>,
    mut text: Query<&mut BitmapText>,
) {
    let gray = gray.get_or_insert_with(|| assets.load("retail://data/fonts/palettegray.sti"));
    for (entity, art, hovered, pressed, disabled, children) in &buttons {
        let frame = art.normal
            + if disabled {
                3
            } else if pressed {
                2
            } else if hovered.get() || focus.get() == Some(entity) {
                1
            } else {
                0
            };
        for child in children.iter() {
            if let Ok(mut picture) = pictures.get_mut(child)
                && picture.frame != frame
            {
                picture.frame = frame;
            }
            if let Ok(mut label) = text.get_mut(child) {
                let palette = disabled.then(|| gray.clone());
                if label.palette != palette {
                    label.palette = palette;
                }
            }
        }
    }
}
