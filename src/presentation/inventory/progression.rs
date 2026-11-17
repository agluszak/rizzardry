//! The level-up screen over the character review. What the member is to
//! become is a draft entity ([`crate::game::characters::draft`]); the screen
//! shows its components and figures and asks for changes by events.
use super::{InventoryArt, InventoryScreen};
use crate::content::characters::{Attribute, Profession, Skill};
use crate::content::spells::SpellId;
use crate::game::actors::{Abilities, Skills, Traits, UsableSkills};
use crate::game::characters::draft::{
    Accept, Advance, Allot, Allotment, Allotted, Becomes, Follow, Forced, Pick, Picked, Points,
    attributes_left, before_this_level, may_advance, may_follow, may_learn, may_raise_attribute,
    may_raise_skill, picks_left, skills_left, unfinished,
};
use crate::game::characters::progression::SpellPicks;
use crate::game::characters::{Birth, Calling, Experience, Identity, Training, Vitals};
use crate::game::data::Catalogs;
use crate::game::magic::castable::{CastableSpell, Castables, Learned};
use crate::game::party::roster::PartyMember;
use crate::presentation::retail_controls::{ControlSkin, FocusRing, set_enabled};
use crate::presentation::screen_art::{BitmapText, Picture, absolute};
use bevy::input_focus::{
    InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, Button, ModalDialog, ScrollArea};
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

pub(super) fn plugin(app: &mut App) {
    app.add_input_context::<Editor>()
        .add_systems(Update, (offer, refresh, scroll_to_focus))
        // After the draft's own upkeep, which lets a stale draft go.
        .add_systems(PostUpdate, follow_draft);
}
#[derive(Component)]
struct Open;
/// The screen, over the member gaining a level.
#[derive(Component)]
struct Editor {
    member: Entity,
    message: String,
    page: Page,
    /// A level was accepted: the next, if the member is ready for it, opens
    /// once the accepted draft is gone.
    again: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Attributes,
    Skills,
    Spells,
}
#[derive(InputAction)]
#[action_output(bool)]
struct Cancel;
#[derive(Component, Clone, Copy)]
enum Action {
    Profession,
    Page(Page),
    Attribute(Attribute, bool),
    Skill(Skill, bool),
    Spell(SpellId),
    Accept,
    Cancel,
}
#[derive(Component)]
enum Caption {
    Header,
    Remaining,
    Message,
    Attribute(Attribute),
    Skill(Skill),
    Spell(SpellId),
}
#[derive(Component)]
struct PagePanel(Page);

fn label(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<crate::content::ui::SpriteSheet>,
    text: &str,
) -> Entity {
    commands
        .spawn((
            BitmapText { font: font.clone(), value: text.into(), ..default() },
            Node { position_type: PositionType::Relative, ..default() },
            ChildOf(parent),
        ))
        .id()
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    art: &InventoryArt,
    text: &str,
    action: Action,
) -> Entity {
    let id = commands
        .spawn((
            Button,
            TabIndex(0),
            FocusRing,
            action,
            Node {
                min_height: px(21),
                flex_shrink: 0.,
                padding: UiRect::axes(px(8), px(3)),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.22, 0.17, 0.11)),
            BorderColor::all(Color::srgb(0.5, 0.4, 0.25)),
            ChildOf(parent),
        ))
        .observe(press)
        .id();
    label(commands, id, &art.font, text);
    id
}
/// Put the offer in the review header, with live eligibility rather than a
/// latched readiness flag. The button survives member/stat changes.
pub(super) fn build(commands: &mut Commands, root: Entity, art: &InventoryArt) {
    commands
        .spawn((
            Open,
            Button,
            // Picture defaults to non-interactive; this image is the control.
            Pickable::default(),
            TabIndex(-1),
            FocusRing,
            // Retail CreateRcsLevelUpPanel: an 18x18 icon at (233, 63),
            // beside the portrait, never over the character's level text.
            Picture {
                sheet: art.level_up.clone(),
                frame: 0,
                at: Vec2::new(233., 63.),
                bounds: Some(0),
            },
            ControlSkin::new(0, 1, 2, 4, 3),
            Node { display: Display::None, ..absolute(Vec2::new(233., 63.), Vec2::splat(18.)) },
            InteractionDisabled,
            ChildOf(root),
        ))
        .observe(open);
}
#[allow(clippy::type_complexity)]
fn offer(
    mut commands: Commands,
    screen: Option<Single<&InventoryScreen>>,
    mut buttons: Query<(Entity, Has<InteractionDisabled>, &mut Node), With<Open>>,
    members: Query<(&Calling, &Experience, &Vitals, Has<PartyMember>)>,
    busy: Query<
        (),
        Or<(
            With<crate::game::world::level::Destination>,
            With<crate::game::npcs::conversation::Conversation>,
            With<crate::game::world::level::TravelQuestion>,
        )>,
    >,
    battle: Option<Res<crate::game::combat::Battle>>,
    editors: Query<(), With<Editor>>,
    mut focus: ResMut<InputFocus>,
) {
    let busy = battle.is_some() || !busy.is_empty();
    let ready = |member| {
        members.get(member).is_ok_and(|(calling, experience, vitals, seated)| {
            may_advance(busy, seated, (calling, experience, vitals)).is_ok()
        })
    };
    let enabled = editors.is_empty() && screen.is_some_and(|s| ready(s.member));
    for (id, disabled, mut node) in &mut buttons {
        set_enabled(&mut commands, id, enabled, disabled);
        // Retail hides the offer altogether when advancement is unavailable.
        crate::presentation::screen_art::display_if(&mut node, enabled);
        if !enabled && focus.get() == Some(id) {
            focus.clear();
        }
    }
}
fn open(
    _: On<Activate>,
    mut commands: Commands,
    screen: Single<(Entity, &InventoryScreen)>,
    editors: Query<(), With<Editor>>,
    content: If<Res<Catalogs>>,
    art: Res<InventoryArt>,
    mut focus: ResMut<InputFocus>,
) {
    if !editors.is_empty() {
        return;
    }
    let (parent, screen) = *screen;
    let member = screen.member;
    commands.trigger(Advance { member });
    let spells = &content.spells;
    let root = commands.spawn_scene(bsn! {
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        GlobalZIndex(20) BackgroundColor(Color::srgba(0.,0.,0.,0.95))
        TabGroup::modal() ModalDialog
        ContextPriority::<Editor>::new(3)
        Actions<Editor> [
            bevy_enhanced_input::prelude::Action::<Cancel>::new()
            ActionSettings { consume_input: true }
            Bindings [ Binding(KeyCode::Escape) -- Binding(KeyCode::KeyI) ]
        ]
        on(cancel)
    }).insert((
        Editor { member, message: String::new(), page: Page::Attributes, again: false },
        ChildOf(parent),
    )).id();
    let panel = commands
        .spawn((
            Node {
                width: px(620),
                height: px(460),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(10)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.045, 0.035, 0.025)),
            ChildOf(root),
        ))
        .id();
    let heading = label(&mut commands, panel, &art.font, "");
    commands.entity(heading).insert(Caption::Header);
    let profession = button(
        &mut commands,
        panel,
        &art,
        "Change profession (resets allocations)",
        Action::Profession,
    );
    let tabs = commands.spawn((Node { column_gap: px(8), ..default() }, ChildOf(panel))).id();
    for (page, name) in
        [(Page::Attributes, "Attributes"), (Page::Skills, "Skills"), (Page::Spells, "Spells")]
    {
        button(&mut commands, tabs, &art, name, Action::Page(page));
    }
    let remaining = label(&mut commands, panel, &art.font, "");
    commands.entity(remaining).insert(Caption::Remaining);
    for page in [Page::Attributes, Page::Skills, Page::Spells] {
        let list = commands
            .spawn((
                PagePanel(page),
                ScrollArea,
                Node {
                    height: px(260),
                    min_height: px(260),
                    overflow: Overflow::scroll_y(),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(3),
                    ..default()
                },
                ChildOf(panel),
            ))
            .id();
        match page {
            Page::Attributes => {
                for a in EnumSet::<Attribute>::all() {
                    let row = commands
                        .spawn((
                            Node {
                                align_items: AlignItems::Center,
                                column_gap: px(8),
                                min_height: px(27),
                                ..default()
                            },
                            ChildOf(list),
                        ))
                        .id();
                    let cell =
                        commands.spawn((Node { width: px(330), ..default() }, ChildOf(row))).id();
                    let text = label(&mut commands, cell, &art.font, "");
                    commands.entity(text).insert(Caption::Attribute(a));
                    button(&mut commands, row, &art, "-", Action::Attribute(a, false));
                    button(&mut commands, row, &art, "+", Action::Attribute(a, true));
                }
            }
            Page::Skills => {
                for s in EnumSet::<Skill>::all() {
                    let row = commands
                        .spawn((
                            Node {
                                align_items: AlignItems::Center,
                                column_gap: px(8),
                                min_height: px(27),
                                ..default()
                            },
                            ChildOf(list),
                        ))
                        .id();
                    let cell =
                        commands.spawn((Node { width: px(330), ..default() }, ChildOf(row))).id();
                    let text = label(&mut commands, cell, &art.font, "");
                    commands.entity(text).insert(Caption::Skill(s));
                    button(&mut commands, row, &art, "-", Action::Skill(s, false));
                    button(&mut commands, row, &art, "+", Action::Skill(s, true));
                }
            }
            Page::Spells => {
                for spell in spells
                    .0
                    .iter()
                    .filter(|s| s.id.0 > 0 && s.id.0 < SpellId::LEARNABLE && !s.name.is_empty())
                {
                    let id =
                        button(&mut commands, list, &art, &spell.name, Action::Spell(spell.id));
                    commands.entity(id).insert(Name::new(spell.name.clone()));
                    let text = label(&mut commands, id, &art.font, "");
                    commands.entity(text).insert(Caption::Spell(spell.id));
                }
            }
        }
    }
    let error = label(&mut commands, panel, &art.font, "");
    commands.entity(error).insert(Caption::Message);
    let actions = commands.spawn((Node { column_gap: px(12), ..default() }, ChildOf(panel))).id();
    button(&mut commands, actions, &art, "Accept level", Action::Accept);
    button(&mut commands, actions, &art, "Cancel", Action::Cancel);
    focus.set(profession, bevy::input_focus::FocusCause::Navigated);
}
/// The draft of the member the screen is over, if there is one.
fn draft_of(drafts: &Query<(Entity, &Becomes)>, member: Entity) -> Option<Entity> {
    drafts.iter().find(|(_, becomes)| becomes.0 == member).map(|(draft, _)| draft)
}
fn cancel(
    event: On<Start<Cancel>>,
    mut commands: Commands,
    editors: Query<&Editor>,
    drafts: Query<(Entity, &Becomes)>,
    mut focus: ResMut<InputFocus>,
    offer: Option<Single<Entity, With<Open>>>,
) {
    if let Some(button) = offer.map(Single::into_inner) {
        focus.set(button, bevy::input_focus::FocusCause::Navigated);
    }
    if let Ok(editor) = editors.get(event.context)
        && let Some(draft) = draft_of(&drafts, editor.member)
    {
        commands.entity(draft).despawn();
    }
    commands.entity(event.context).despawn();
}
#[allow(clippy::type_complexity)]
fn press(
    event: On<Activate>,
    mut commands: Commands,
    actions: Query<&Action>,
    editor: Single<(Entity, &mut Editor)>,
    drafts: Query<(Entity, &Becomes)>,
    sheets: Query<(
        (&Points, &Forced, &Allotted),
        (&Abilities, &Training, &UsableSkills),
        &Calling,
    )>,
    members: Query<(&Birth, &Abilities, &Calling)>,
    mut focus: ResMut<InputFocus>,
    offer: Option<Single<Entity, With<Open>>>,
) -> Result {
    let action = actions.get(event.entity)?;
    let (id, mut editor) = editor.into_inner();
    editor.message.clear();
    let Some(draft) = draft_of(&drafts, editor.member) else { return Ok(()) };
    match *action {
        Action::Cancel => {
            if let Some(button) = offer.map(Single::into_inner) {
                focus.set(button, bevy::input_focus::FocusCause::Navigated);
            }
            commands.entity(draft).despawn();
            commands.entity(id).despawn();
        }
        Action::Accept => {
            let (spent, sheet, _) = sheets.get(draft)?;
            if let Some(error) = unfinished(spent, sheet) {
                editor.message = error.into();
                return Ok(());
            }
            editor.again = true;
            editor.page = Page::Attributes;
            commands.trigger(Accept { draft });
        }
        Action::Page(page) => editor.page = page,
        Action::Profession => {
            let (_, _, following) = sheets.get(draft)?;
            let member = members.get(editor.member)?;
            let current = following.profession as usize;
            let all: Vec<_> = EnumSet::<Profession>::all().iter().collect();
            if let Some(profession) = (1..=all.len())
                .map(|i| all[(current + i) % all.len()])
                .find(|&p| may_follow(p, member))
            {
                commands.trigger(Follow { draft, profession });
            }
        }
        Action::Attribute(a, up) => {
            commands.trigger(Allot { draft, what: Allotment::Attribute(a), up });
        }
        Action::Skill(s, up) => {
            commands.trigger(Allot { draft, what: Allotment::Skill(s), up });
        }
        Action::Spell(spell) => commands.trigger(Pick { draft, spell }),
    }
    Ok(())
}
/// The screen follows its draft: once the draft is gone (accepted, or let
/// go because the member changed meanwhile), the member's next level opens
/// if one was accepted and they are ready for another; otherwise the screen
/// closes.
#[allow(clippy::type_complexity)]
fn follow_draft(
    mut commands: Commands,
    mut editors: Query<(Entity, &mut Editor)>,
    drafts: Query<(Entity, &Becomes)>,
    members: Query<(&Calling, &Experience, &Vitals, Has<PartyMember>)>,
    busy: Query<
        (),
        Or<(
            With<crate::game::world::level::Destination>,
            With<crate::game::npcs::conversation::Conversation>,
            With<crate::game::world::level::TravelQuestion>,
        )>,
    >,
    battle: Option<Res<crate::game::combat::Battle>>,
    mut focus: ResMut<InputFocus>,
    offer: Option<Single<Entity, With<Open>>>,
) {
    let busy = battle.is_some() || !busy.is_empty();
    for (id, mut editor) in &mut editors {
        if draft_of(&drafts, editor.member).is_some() {
            continue;
        }
        let member = editor.member;
        let ready = members.get(member).is_ok_and(|(calling, experience, vitals, seated)| {
            may_advance(busy, seated, (calling, experience, vitals)).is_ok()
        });
        if std::mem::take(&mut editor.again) && ready {
            commands.trigger(Advance { member });
            continue;
        }
        if let Some(button) = offer.as_deref() {
            focus.set(*button, bevy::input_focus::FocusCause::Navigated);
        }
        commands.entity(id).despawn();
    }
}
/// Keep the focused control in view.
fn scroll_to_focus(
    mut commands: Commands,
    focus: Res<InputFocus>,
    actions: Query<(), With<Action>>,
) {
    if focus.is_changed()
        && let Some(entity) = focus.get()
        && actions.contains(entity)
    {
        commands.trigger(bevy::ui_widgets::ScrollIntoView { entity });
    }
}

/// Show the draft, read off its components and figures. It is redrawn only
/// when the screen, the draft, the content or the strings change or its
/// controls are new.
#[allow(clippy::type_complexity)]
fn refresh(
    mut commands: Commands,
    editor: Single<Ref<Editor>>,
    drafts: Query<(
        &Becomes,
        (Ref<Points>, Ref<Forced>, Ref<Allotted>, Ref<Picked>),
        (Ref<Calling>, Ref<Abilities>, Ref<Training>, Ref<UsableSkills>),
        (Ref<SpellPicks>, Option<Ref<Castables>>, &Traits, &Identity),
        Ref<crate::game::actors::CasterLevels>,
    )>,
    members: Query<(Option<&Castables>, &Skills)>,
    known_spells: Query<&CastableSpell, With<Learned>>,
    content: If<Res<Catalogs>>,
    strings: crate::presentation::screen_art::GameStrings,
    mut captions: Query<(&Caption, &mut BitmapText)>,
    mut pages: Query<(&PagePanel, &mut Node, &mut Visibility)>,
    mut actions: Query<(Entity, &Action, Has<InteractionDisabled>, &mut BackgroundColor)>,
    added: Query<(), Or<(Added<Caption>, Added<Action>, Added<PagePanel>)>>,
) {
    let Some((_, spent, sheet, learning, levels)) =
        drafts.iter().find(|(becomes, ..)| becomes.0 == editor.member)
    else {
        return;
    };
    let (points, forced, allotted, picked) = spent;
    let (calling, abilities, training, usable) = sheet;
    let (granted, learned, traits, identity) = learning;
    let draft_changed = points.is_changed()
        || forced.is_changed()
        || allotted.is_changed()
        || picked.is_changed()
        || calling.is_changed()
        || abilities.is_changed()
        || training.is_changed()
        || usable.is_changed()
        || granted.is_changed()
        || learned.as_ref().is_none_or(|learned| learned.is_changed())
        || levels.is_changed();
    let changed = draft_changed
        || editor.is_changed()
        || content.is_changed()
        || strings.is_changed()
        || !added.is_empty();
    if !changed {
        return;
    }
    let (owned, standing) = members.get(editor.member).expect("the one edited is a member");
    let ids = |owned: Option<&Castables>| -> Vec<crate::content::spells::SpellId> {
        let owned = owned.into_iter().flat_map(|owned| owned.iter());
        known_spells.iter_many(owned).flatten().map(|spell| spell.0.id).collect()
    };
    let (known, learned) = (ids(owned), ids(learned.as_deref()));
    let spells = &content.spells;
    let picks =
        picks_left(&granted, &calling, &picked.0, &content).map_or(0, |p| p.available(&calling));
    // Spells are judged by the skills as they were before this level's
    // points (retail `CountRemainingSpellPoints`).
    let before = before_this_level(&training, &allotted);
    let name = |index, fallback: String| {
        strings.table().and_then(|t| t.get(index)).map_or(fallback, str::to_owned)
    };
    for (caption, mut text) in &mut captions {
        let value = match *caption {
            Caption::Header => format!(
                "{} - level {} {} (profession level {})",
                identity.name,
                calling.level(),
                name(
                    crate::content::ui::PROFESSION_NAMES + calling.profession as usize,
                    format!("{:?}", calling.profession)
                ),
                calling.levels[calling.profession]
            ),
            Caption::Remaining => format!(
                "Remaining: {} attributes, {} skills, {} spell picks",
                attributes_left(&points, &forced, &allotted),
                skills_left(&points, &allotted),
                picks
            ),
            Caption::Message => editor.message.clone(),
            Caption::Attribute(a) => {
                format!("{a:?}: {} (+{})", abilities.0[a], forced.0[a] + allotted.attributes[a])
            }
            Caption::Skill(s) => format!(
                "{}: {} (+{})",
                name(crate::content::ui::SKILL_NAMES + s as usize, format!("{s:?}")),
                training.0[s],
                allotted.skills[s]
            ),
            Caption::Spell(s) => {
                if learned.contains(&s) && !known.contains(&s) {
                    "  Selected".into()
                } else {
                    String::new()
                }
            }
        };
        if text.value != value {
            text.value = value;
        }
    }
    for (page, mut node, mut visibility) in &mut pages {
        let shown = page.0 == editor.page;
        crate::presentation::screen_art::display_if(&mut node, shown);
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (id, action, disabled, mut color) in &mut actions {
        let enabled = match *action {
            Action::Attribute(a, true) => {
                may_raise_attribute((&points, &forced, &allotted), &abilities, a)
            }
            Action::Attribute(a, false) => allotted.attributes[a] > 0,
            Action::Skill(s, true) => {
                may_raise_skill((&points, &allotted), (&training, &usable), s)
            }
            Action::Skill(s, false) => allotted.skills[s] > 0,
            Action::Spell(s) => {
                (learned.contains(&s) && !known.contains(&s))
                    || (picks > 0
                        && spells.get(s).is_some_and(|spell| {
                            let caster = (levels.followed, levels.caster_level(spell));
                            !known.contains(&s)
                                && may_learn((&calling, traits.0, &before), standing, caster, spell)
                        }))
            }
            _ => true,
        };
        set_enabled(&mut commands, id, enabled, disabled);
        color.set_if_neq(BackgroundColor(if enabled {
            Color::srgb(0.22, 0.17, 0.11)
        } else {
            Color::srgb(0.08, 0.065, 0.045)
        }));
    }
}
