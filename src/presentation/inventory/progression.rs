//! Actor-bound advancement editor over the existing character review.
use super::{InventoryArt, InventoryScreen};
use crate::content::characters::{Attribute, Profession, Skill};
use crate::content::spells::SpellId;
use crate::game::characters::progression::{Advancement, Progression};
use crate::game::data::Catalogs;
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
    app.add_input_context::<Editor>().add_systems(Update, (offer, refresh, scroll_to_focus));
}
#[derive(Component)]
struct Open;
#[derive(Component)]
struct Editor {
    draft: Advancement,
    message: String,
    page: Page,
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
            TabIndex(0),
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
fn offer(
    mut commands: Commands,
    screens: Query<&InventoryScreen>,
    mut buttons: Query<(Entity, Has<InteractionDisabled>, &mut Node), With<Open>>,
    progression: Progression,
    editors: Query<(), With<Editor>>,
    mut focus: ResMut<InputFocus>,
) {
    let enabled = editors.is_empty()
        && screens.single().is_ok_and(|s| progression.eligibility(s.member).is_ok());
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
    screens: Query<(Entity, &InventoryScreen)>,
    editors: Query<(), With<Editor>>,
    progression: Progression,
    content: If<Res<Catalogs>>,
    art: Res<InventoryArt>,
    mut focus: ResMut<InputFocus>,
) {
    if !editors.is_empty() {
        return;
    }
    let Ok((parent, screen)) = screens.single() else { return };
    let Ok(draft) = progression.begin(screen.member) else { return };
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
    }).insert((Editor { draft, message: String::new(), page: Page::Attributes }, ChildOf(parent))).id();
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
fn cancel(
    event: On<Start<Cancel>>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
    offers: Query<Entity, With<Open>>,
) {
    if let Ok(button) = offers.single() {
        focus.set(button, bevy::input_focus::FocusCause::Navigated);
    }
    commands.entity(event.context).despawn();
}
fn press(
    event: On<Activate>,
    mut commands: Commands,
    actions: Query<&Action>,
    mut editors: Query<(Entity, &mut Editor)>,
    content: If<Res<Catalogs>>,
    mut progression: Progression,
    mut focus: ResMut<InputFocus>,
    offers: Query<Entity, With<Open>>,
) {
    let Ok(action) = actions.get(event.entity) else { return };
    let Ok((id, mut editor)) = editors.single_mut() else { return };
    let spells = &content.spells;
    editor.message.clear();
    match *action {
        Action::Cancel | Action::Accept => {
            if matches!(action, Action::Accept)
                && let Err(error) = progression.accept(&editor.draft)
            {
                editor.message = error.into();
                return;
            }
            // Experience enough for another level opens its review straight away.
            if matches!(action, Action::Accept)
                && let Ok(next) = progression.begin(editor.draft.member())
            {
                editor.draft = next;
                editor.page = Page::Attributes;
                return;
            }
            if let Ok(button) = offers.single() {
                focus.set(button, bevy::input_focus::FocusCause::Navigated);
            }
            commands.entity(id).despawn();
        }
        Action::Page(page) => editor.page = page,
        Action::Profession => {
            let current = editor.draft.profession() as usize;
            let all: Vec<_> = EnumSet::<Profession>::all().iter().collect();
            if let Some(next) = (1..=all.len())
                .map(|i| all[(current + i) % all.len()])
                .find(|&p| editor.draft.eligible_profession(p))
            {
                editor.draft.choose_profession(next);
            }
        }
        Action::Attribute(a, up) => {
            editor.draft.adjust_attribute(a, up);
        }
        Action::Skill(s, up) => {
            editor.draft.adjust_skill(s, up, spells);
        }
        Action::Spell(s) => {
            editor.draft.toggle_spell(s, spells);
        }
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

/// Show the draft. It is redrawn only when the editor, the content or the
/// strings change or its controls are new, and the draft is worked out once
/// for all of its rows.
fn refresh(
    mut commands: Commands,
    editors: Query<Ref<Editor>>,
    content: If<Res<Catalogs>>,
    strings: crate::presentation::screen_art::GameStrings,
    mut captions: Query<(&Caption, &mut BitmapText)>,
    mut pages: Query<(&PagePanel, &mut Node, &mut Visibility)>,
    mut actions: Query<(Entity, &Action, Has<InteractionDisabled>, &mut BackgroundColor)>,
    added: Query<(), Or<(Added<Caption>, Added<Action>, Added<PagePanel>)>>,
) {
    let Ok(editor) = editors.single() else { return };
    let changed =
        editor.is_changed() || content.is_changed() || strings.is_changed() || !added.is_empty();
    if !changed {
        return;
    }
    let spells = &content.spells;
    let draft = &editor.draft;
    let evaluated = draft.evaluate();
    let c = &evaluated.preview;
    let spell_picks = draft.spell_remaining(spells);
    let name = |index, fallback: String| {
        strings.table().and_then(|t| t.get(index)).map_or(fallback, str::to_owned)
    };
    for (caption, mut text) in &mut captions {
        let value = match *caption {
            Caption::Header => format!(
                "{} - level {} {} (profession level {})",
                c.name,
                c.level(),
                name(
                    crate::content::ui::PROFESSION_NAMES + c.profession as usize,
                    format!("{:?}", c.profession)
                ),
                c.profession_levels[c.profession]
            ),
            Caption::Remaining => format!(
                "Remaining: {} attributes, {} skills, {} spell picks",
                draft.attribute_remaining(),
                draft.skill_remaining(),
                spell_picks
            ),
            Caption::Message => editor.message.clone(),
            Caption::Attribute(a) => {
                format!("{a:?}: {} (+{})", c.attributes[a], draft.allocated_attribute(a))
            }
            Caption::Skill(s) => {
                format!(
                    "{}: {} (+{})",
                    name(crate::content::ui::SKILL_NAMES + s as usize, format!("{s:?}")),
                    c.skill_points[s],
                    draft.allocated_skill(s)
                )
            }
            Caption::Spell(s) => {
                if draft.selected_spell(s) {
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
            Action::Attribute(a, up) => draft.attribute_adjustable(&evaluated, a, up),
            Action::Skill(s, up) => draft.skill_adjustable(&evaluated, s, up, spells),
            Action::Spell(s) => {
                draft.selected_spell(s)
                    || (spell_picks > 0 && draft.learnable(&evaluated, s, spells))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        items::ItemCatalog,
        spells::SpellCatalog,
        ui::{SpriteSheet, TextTable},
    };
    use crate::game::{
        characters::{Character, Gender, Race, Vitals},
        inventory::Carried,
        magic::{LearnedSpells, SpellPoints},
        party::roster,
    };
    use bevy::ecs::system::RunSystemOnce;

    /// An inventory review of the first of `party`, with its level-up button.
    fn review(party: &[Character]) -> (App, Vec<Entity>, Entity, Entity) {
        let mut app = crate::presentation::test_input::input_app();
        app.init_asset::<SpriteSheet>()
            .init_asset::<TextTable>()
            .init_resource::<InventoryArt>()
            .init_resource::<crate::game::inventory::ItemIds>()
            .init_resource::<crate::game::rng::GameRng>()
            .add_message::<crate::game::inventory::ItemNews>()
            .add_plugins(plugin)
            .init_state::<crate::presentation::screen::Screen>()
            .init_resource::<crate::presentation::main_screen::SelectedMember>()
            .add_input_context::<super::super::InventoryRoot>()
            .add_systems(
                OnEnter(crate::presentation::screen::Screen::Inventory),
                super::super::enter_inventory,
            );
        let spells = SpellCatalog::default();
        let items = ItemCatalog::default();
        app.insert_resource(crate::game::data::Catalogs { items, spells, ..default() });
        let members = party
            .iter()
            .map(|c| {
                app.world_mut()
                    .spawn((
                        c.clone(),
                        Vitals::new(c),
                        Carried::default(),
                        LearnedSpells::default(),
                        SpellPoints::default(),
                    ))
                    .id()
            })
            .collect::<Vec<_>>();
        roster::test_party(app.world_mut(), &members);
        app.world_mut()
            .resource_mut::<NextState<crate::presentation::screen::Screen>>()
            .set(crate::presentation::screen::Screen::Inventory);
        app.finish();
        app.cleanup();
        app.update();
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<InventoryScreen>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .run_system_once(move |mut commands: Commands, art: Res<InventoryArt>| {
                super::build(&mut commands, root, &art);
            })
            .unwrap();
        app.update();
        let open =
            app.world_mut().query_filtered::<Entity, With<Open>>().single(app.world()).unwrap();
        (app, members, root, open)
    }

    #[test]
    fn advancement_offer_tracks_eligibility_and_releases_focus_when_hidden() {
        let c = Character::new("A", Race::Human, Gender::Female, Profession::Fighter);
        let (mut app, members, _, open) = review(&[c]);
        assert_eq!(app.world().get::<Node>(open).unwrap().display, Display::None);
        assert!(app.world().get::<InteractionDisabled>(open).is_some());
        {
            let mut c = app.world_mut().get_mut::<Character>(members[0]).unwrap();
            c.experience = c.experience_goal();
        }
        app.update();
        assert_eq!(app.world().get::<Node>(open).unwrap().display, Display::Flex);
        assert!(app.world().get::<InteractionDisabled>(open).is_none());
        let pickable = app.world().get::<Pickable>(open).unwrap();
        assert!(pickable.is_hoverable && pickable.should_block_lower);
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(open, bevy::input_focus::FocusCause::Navigated);
        app.world_mut().get_mut::<Character>(members[0]).unwrap().experience = 0;
        app.update();
        assert_eq!(app.world().get::<Node>(open).unwrap().display, Display::None);
        assert!(app.world().get::<InteractionDisabled>(open).is_some());
        assert_ne!(app.world().resource::<InputFocus>().get(), Some(open));
    }

    #[test]
    fn editor_keeps_actor_and_rows_and_escape_discards_without_closing_review() {
        let mut c = Character::new("A", Race::Human, Gender::Female, Profession::Fighter);
        c.experience = c.experience_goal();
        let mut second = c.clone();
        second.name = "B".into();
        let (mut app, members, root, open) = review(&[c.clone(), second.clone()]);
        let (a, b) = (members[0], members[1]);
        app.world_mut().trigger(Activate { entity: open });
        app.update();
        let editor =
            app.world_mut().query_filtered::<Entity, With<Editor>>().single(app.world()).unwrap();
        let increase = app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
            .find_map(|(e, a)| {
                matches!(a, Action::Attribute(Attribute::Strength, true)).then_some(e)
            })
            .unwrap();
        let rows = app
            .world_mut()
            .query_filtered::<Entity, With<Caption>>()
            .iter(app.world())
            .collect::<Vec<_>>();
        app.world_mut().trigger(Activate { entity: increase });
        app.update();
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<Caption>>()
                .iter(app.world())
                .collect::<Vec<_>>(),
            rows
        );
        app.world_mut().get_mut::<InventoryScreen>(root).unwrap().member = b;
        assert_eq!(app.world().get::<Editor>(editor).unwrap().draft.preview().name, "A");
        crate::presentation::test_input::tap(&mut app, Entity::PLACEHOLDER, KeyCode::Escape);
        assert!(app.world().get_entity(editor).is_err());
        assert!(app.world().get_entity(root).is_ok());
        assert_eq!(*app.world().get::<Character>(a).unwrap(), c);
        assert_eq!(*app.world().get::<Character>(b).unwrap(), second);
        assert_eq!(
            *app.world().resource::<State<crate::presentation::screen::Screen>>().get(),
            crate::presentation::screen::Screen::Inventory
        );
        app.world_mut().trigger(Activate { entity: open });
        app.update();
        let editor =
            app.world_mut().query_filtered::<Entity, With<Editor>>().single(app.world()).unwrap();
        app.world_mut().despawn(root);
        assert!(app.world().get_entity(editor).is_err(), "the draft lives with the review");
    }

    #[test]
    fn experience_for_another_level_reopens_the_review_after_accepting() {
        let mut c = Character::new("A", Race::Human, Gender::Female, Profession::Fighter);
        let mut next = c.clone();
        next.experience_previous_goal = c.experience_goal();
        next.profession_levels[next.profession] += 1;
        c.experience = next.experience_goal();
        let (mut app, members, _, open) = review(&[c.clone()]);
        let a = members[0];
        app.world_mut().trigger(Activate { entity: open });
        app.update();
        let accept = app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
            .find_map(|(e, a)| matches!(a, Action::Accept).then_some(e))
            .unwrap();
        let level_up = |app: &mut App| {
            let spells = SpellCatalog::default();
            let mut editor =
                app.world_mut().query::<&mut Editor>().single_mut(app.world_mut()).unwrap();
            let draft = &mut editor.draft;
            while let Some(a) =
                EnumSet::<Attribute>::all().iter().find(|&a| draft.can_adjust_attribute(a, true))
            {
                draft.adjust_attribute(a, true);
            }
            while let Some(s) =
                EnumSet::<Skill>::all().iter().find(|&s| draft.can_adjust_skill(s, true, &spells))
            {
                draft.adjust_skill(s, true, &spells);
            }
            app.world_mut().trigger(Activate { entity: accept });
            app.update();
        };
        level_up(&mut app);
        assert_eq!(app.world().get::<Character>(a).unwrap().level(), c.level() + 1);
        let editor =
            app.world_mut().query::<&Editor>().single(app.world()).expect("a second level waits");
        assert_eq!(editor.draft.preview().level(), c.level() + 2);
        assert!(editor.message.is_empty());
        level_up(&mut app);
        assert_eq!(app.world().get::<Character>(a).unwrap().level(), c.level() + 2);
        assert!(app.world_mut().query::<&Editor>().iter(app.world()).next().is_none());
    }
}
