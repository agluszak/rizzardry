//! Retail party assembly: the candidate list and the ordered party are separate.
use super::{Action, CharacterDraft, Creation, Page, absolute};
use crate::content::ui::SpriteSheet;
use crate::presentation::retail_controls::{ControlSkin, FocusHighlight, set_enabled};
use crate::presentation::screen_art::{BitmapText, Picture, spawn_picture as picture};
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
#[derive(Component)]
pub(super) struct ToggleLabel;
#[derive(Component)]
pub(super) struct PartyMessage;
#[derive(Component)]
pub(super) struct RowBackground(u32);
#[derive(Component)]
pub(super) struct SelectedDetail;
#[derive(Component)]
pub(super) struct SlotFrame {
    candidate: u32,
    highlight: bool,
}
#[derive(Component)]
pub(super) enum Portrait {
    Candidate(u32),
    Member(u32),
    Selected,
}
#[derive(Component)]
pub(super) enum Label {
    CandidateName(u32),
    CandidateClass(u32),
    CandidateRace(u32),
    Member(u32),
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
    let load = |path: &str| assets.load(format!("retail://data/{path}.sti"));
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

/// Candidate IDs are stable across deletion and party reordering. Controls keep
/// those IDs for their entire lifetime; only their positions change.
pub(super) fn sync_views(
    mut commands: Commands,
    session: Option<Single<&Creation>>,
    page: Query<Entity, With<AssemblyPage>>,
    mut views: Query<(Entity, &CandidateView, &mut Node)>,
    assets: Res<AssetServer>,
) {
    let (Some(session), Ok(page)) = (session, page.single()) else { return };
    for (entity, view, _) in &views {
        if candidate(&session, view.id).is_none()
            || (view.joined && !session.party.contains(&view.id))
        {
            commands.entity(entity).despawn();
        }
    }
    let load = |path: &str| assets.load(format!("retail://data/{path}.sti"));
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
            let root = commands
                .spawn((
                    Control,
                    CandidateView { id, joined },
                    Button,
                    TabIndex(0),
                    Hovered::default(),
                    Action::SelectCandidate(Some(id)),
                    absolute(at, if joined { Vec2::new(90., 71.) } else { Vec2::new(158., 40.) }),
                    Pickable::default(),
                    ChildOf(page),
                ))
                .id();
            if joined {
                let portrait = picture(&mut commands, root, default(), 0, Vec2::ZERO);
                commands.entity(portrait).insert(Portrait::Member(id));
                let frame = picture(
                    &mut commands,
                    root,
                    load("party generation/portraitborder"),
                    0,
                    Vec2::new(-13., -11.),
                );
                commands.entity(frame).insert(SlotFrame { candidate: id, highlight: false });
                let frame = picture(
                    &mut commands,
                    root,
                    load("party generation/portraithighlight"),
                    1,
                    Vec2::ZERO,
                );
                commands.entity(frame).insert(SlotFrame { candidate: id, highlight: true });
                let name = label(
                    &mut commands,
                    root,
                    load("fonts/wiz_text_font"),
                    "",
                    Vec2::new(-10., 79.),
                    110.,
                );
                commands.entity(name).insert(Label::Member(id));
            } else {
                let background =
                    picture(&mut commands, root, load("party generation/listboxes"), 0, Vec2::ZERO);
                commands.entity(background).insert(RowBackground(id));
                let portrait = picture(&mut commands, root, default(), 0, Vec2::new(2., 2.));
                commands.entity(portrait).insert(Portrait::Candidate(id));
                for (kind, y) in [
                    (Label::CandidateName(id), 3.),
                    (Label::CandidateClass(id), 14.),
                    (Label::CandidateRace(id), 25.),
                ] {
                    let text = label(
                        &mut commands,
                        root,
                        load("fonts/wiz_text_font"),
                        "",
                        Vec2::new(53., y),
                        0.,
                    );
                    commands.entity(text).insert(kind);
                }
            }
        }
    }
}

pub(super) fn refresh(
    session: Option<Single<&Creation>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    controls: Query<(Entity, &Action, Has<InteractionDisabled>), With<Control>>,
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
    for (entity, action, disabled) in &controls {
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
            Portrait::Candidate(id) | Portrait::Member(id) => candidate(&session, *id),
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
            let sheet = assets.load(format!(
                "retail://data/portraits/{directory}/{prefix}{}.sti",
                draft.portrait
            ));
            if picture.sheet != sheet {
                picture.sheet = sheet;
            }
        }
    }
    for (row, mut picture, mut visible) in &mut rows {
        let id = candidate(&session, row.0).map(|_| row.0);
        visible.set_if_neq(if id.is_some() { Visibility::Inherited } else { Visibility::Hidden });
        let frame = usize::from(id.is_some() && id == session.selected);
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
    for mut visible in &mut details {
        visible.set_if_neq(if session.selected.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for (frame, mut visible) in &mut frames {
        let id = session.party.contains(&frame.candidate).then_some(frame.candidate);
        let shown = id.is_some() && (!frame.highlight || id == session.selected);
        visible.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    let preview =
        session.selected.and_then(|id| candidate(&session, id)).map(CharacterDraft::preview);
    for (field, mut text) in &mut labels {
        let value = match field {
            Label::CandidateName(id) | Label::Member(id) => {
                candidate(&session, *id).map(|draft| draft.display_name().to_owned())
            }
            Label::CandidateClass(id) => {
                candidate(&session, *id).map(|draft| format!("Level 1 {:?}", draft.profession()))
            }
            Label::CandidateRace(id) => candidate(&session, *id)
                .map(|draft| format!("{:?} {:?}", draft.gender(), draft.race())),
            Label::Joined => session
                .selected
                .filter(|id| session.party.contains(id))
                .map(|_| "Joined".to_owned()),
            Label::Summary(line) => preview.as_ref().map(|profile| {
                use crate::content::characters::Attribute::*;
                match line {
                    0 => profile.name.clone(),
                    1 => format!("Level 1 {:?}", profile.profession),
                    2 => format!("{:?} {:?}", profile.gender, profile.race),
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
            }),
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

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn deleting_a_candidate_moves_surviving_views_without_rebinding_controls() {
        let mut app = crate::presentation::test_input::input_app();
        app.init_asset::<SpriteSheet>().add_systems(Update, sync_views);
        let session = app
            .world_mut()
            .spawn(Creation {
                candidates: vec![(1, CharacterDraft::default()), (2, CharacterDraft::default())],
                party: vec![1, 2],
                selected: Some(2),
                next_id: 3,
                mode: super::super::CreationMode::Party,
                choices: default(),
                message: String::new(),
            })
            .id();
        app.world_mut().spawn((AssemblyPage, ChildOf(session)));
        app.finish();
        app.cleanup();
        app.update();
        let controls: Vec<_> = app
            .world_mut()
            .query::<(Entity, &CandidateView)>()
            .iter(app.world())
            .filter(|(_, view)| view.id == 2)
            .map(|(entity, view)| (entity, view.joined))
            .collect();
        assert_eq!(controls.len(), 2);
        {
            let mut creation = app.world_mut().get_mut::<Creation>(session).unwrap();
            creation.candidates.remove(0);
            creation.party.remove(0);
        }
        app.update();
        for (entity, joined) in controls {
            assert!(matches!(
                app.world().get::<Action>(entity),
                Some(Action::SelectCandidate(Some(2)))
            ));
            let node = app.world().get::<Node>(entity).unwrap();
            assert_eq!(
                (node.left, node.top),
                if joined { (px(14.), px(58.)) } else { (px(328.), px(49.)) }
            );
        }
        assert!(
            app.world_mut().query::<&CandidateView>().iter(app.world()).all(|view| view.id == 2)
        );
    }
}
