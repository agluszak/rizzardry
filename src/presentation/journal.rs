//! Retail fact journal: twelve historical transitions per page, newest page first.
use super::screen_art::{BitmapText, spawn_picture};
use crate::content::npcs::JournalVisibility;
use crate::game::{difficulty::Difficulty, npcs::conversation::StoryFacts};
use crate::presentation::retail_controls::RestoreFocus;
use crate::presentation::screen::Screen;
use bevy::input_focus::{FocusCause, InputFocus, tab_navigation::TabGroup};
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use bevy_enhanced_input::prelude::*;

#[derive(Component, Clone, Default)]
struct Journal {
    /// Pages back from the newest page; zero follows the newest entries.
    pages_back: usize,
    mode: JournalMode,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum JournalMode {
    #[default]
    Events,
    Topics,
    Factions,
}
impl JournalMode {
    fn next(self) -> Self {
        match self {
            Self::Events => Self::Topics,
            Self::Topics => Self::Factions,
            Self::Factions => Self::Events,
        }
    }
}

#[derive(Component)]
struct Row(usize, bool);
#[derive(Component)]
struct PageLabel;
#[derive(Component)]
struct Heading(bool);
#[derive(Component, Clone, Copy)]
enum Choice {
    Previous,
    Next,
    Topics,
    Close,
}
#[derive(Message)]
pub struct OpenJournal;
#[derive(InputAction)]
#[action_output(bool)]
struct Close;
#[derive(InputAction)]
#[action_output(bool)]
struct Previous;
#[derive(InputAction)]
#[action_output(bool)]
struct Next;

pub fn plugin(app: &mut App) {
    app.add_message::<OpenJournal>()
        .add_input_context::<Journal>()
        .add_systems(Update, (request, refresh).chain())
        .add_systems(OnEnter(Screen::Journal), enter)
        .add_observer(activate);
}
fn request(
    mut requests: MessageReader<OpenJournal>,
    mut state: ParamSet<(super::screen::WorldInput, ResMut<NextState<Screen>>)>,
) {
    if requests.read().count() > 0 && state.p0().free() {
        state.p1().set(Screen::Journal);
    }
}
fn enter(mut commands: Commands, assets: Res<AssetServer>, mut focus: ResMut<InputFocus>) {
    let view = (Journal::default(), RestoreFocus(focus.get()));
    let outer = commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), align_items: AlignItems::Center,
            justify_content: JustifyContent::Center }
        GlobalZIndex(60) TabGroup::modal() DespawnOnExit::<Screen>(Screen::Journal)
        Actions<Journal> [
            Action::<Close>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [Binding(KeyCode::Escape)]
            -- Action::<Previous>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [Binding(KeyCode::ArrowLeft)]
            -- Action::<Next>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [Binding(KeyCode::ArrowRight)]
        ]
    }).insert(view).id();
    let root = commands
        .spawn((Node { width: px(640), height: px(480), ..default() }, ChildOf(outer)))
        .id();
    // The parchment's authored size is the retail 640x436 content region.
    spawn_picture(
        &mut commands,
        root,
        assets.load("retail://data/journal/journal_parchment.sti"),
        0,
        Vec2::ZERO,
    );
    spawn_picture(
        &mut commands,
        root,
        assets.load("retail://data/journal/journal_base.sti"),
        1,
        Vec2::new(0., 436.),
    );
    for i in 0..12 {
        for left in [true, false] {
            commands.spawn((
                Row(i, left),
                BitmapText {
                    font: assets.load("retail://data/journal/journal_font.sti"),
                    at: Vec2::new(if left { 16. } else { 198. }, 57. + i as f32 * 30.),
                    width: if left { 164. } else { 420. },
                    ..default()
                },
                super::screen_art::BitmapWrap {
                    width: if left { 164 } else { 420 },
                    line_spacing: 0,
                },
                ChildOf(root),
            ));
        }
    }
    for left in [true, false] {
        commands.spawn((
            Heading(left),
            BitmapText::centred(
                assets.load("retail://data/journal/journal_font.sti"),
                Vec2::new(if left { 11. } else { 193. }, 25.),
                if left { 175. } else { 435. },
            ),
            ChildOf(root),
        ));
    }
    spawn_picture(
        &mut commands,
        root,
        assets.load("retail://data/options/options_pagingbase.sti"),
        0,
        Vec2::new(102., 443.),
    );
    commands.spawn((
        PageLabel,
        BitmapText::centred(
            assets.load("retail://data/fonts/opt_detail_font.sti"),
            Vec2::new(135., 450.),
            245.,
        ),
        ChildOf(root),
    ));
    use super::retail_controls::{ControlSkin, image_button};
    for (path, at, skin, choice) in [
        (
            "options/options_navigation_arrows",
            Vec2::new(105., 446.),
            ControlSkin::new(0, 2, 1, 1, 3),
            Choice::Previous,
        ),
        (
            "options/options_navigation_arrows",
            Vec2::new(389., 446.),
            ControlSkin::new(4, 6, 5, 5, 7),
            Choice::Next,
        ),
        (
            "journal/faction_ratings_button",
            Vec2::new(534., 441.),
            ControlSkin::new(0, 2, 1, 2, 3),
            Choice::Topics,
        ),
        (
            "char generation/cg_bottombuttons",
            Vec2::new(592., 441.),
            ControlSkin::new(16, 17, 18, 18, 19),
            Choice::Close,
        ),
    ] {
        let e = commands
            .spawn_scene(image_button(assets.load(format!("retail://data/{path}.sti")), at, skin))
            .insert((choice, ChildOf(root)))
            .id();
        if matches!(choice, Choice::Close) {
            focus.set(e, FocusCause::Navigated);
        }
    }
    commands
        .entity(outer)
        .insert(ContextPriority::<Journal>::new(10))
        .observe(|_: On<Start<Close>>, mut next: ResMut<NextState<Screen>>| next.set(Screen::World))
        .observe(|_: On<Start<Previous>>, mut view: Single<&mut Journal>| {
            view.pages_back = view.pages_back.saturating_add(1);
        })
        .observe(|_: On<Start<Next>>, mut view: Single<&mut Journal>| {
            view.pages_back = view.pages_back.saturating_sub(1);
        });
}
fn activate(
    event: On<Activate>,
    choices: Query<&Choice>,
    mut view: Option<Single<&mut Journal>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let (Ok(choice), Some(view)) = (choices.get(event.entity), view.as_mut()) else { return };
    match choice {
        Choice::Previous => view.pages_back = view.pages_back.saturating_add(1),
        Choice::Next => view.pages_back = view.pages_back.saturating_sub(1),
        Choice::Topics => {
            view.mode = view.mode.next();
            view.pages_back = 0;
        }
        Choice::Close => next.set(Screen::World),
    }
}
/// Show the journal's page. It is redrawn only when what it shows changes:
/// the view's page or mode, the history, the difficulty, the factions, or
/// the fact and string tables it reads.
#[allow(clippy::too_many_arguments)]
fn refresh(
    mut view: Option<Single<&mut Journal>>,
    (facts, difficulty, factions): (
        Res<StoryFacts>,
        Res<Difficulty>,
        Res<crate::game::factions::Factions>,
    ),
    (catalogs, strings, asset_server): (
        If<Res<crate::game::data::Catalogs>>,
        super::screen_art::GameStrings,
        Res<AssetServer>,
    ),
    mut rows: Query<(&Row, &mut BitmapText), (Without<PageLabel>, Without<Heading>)>,
    mut title: Query<&mut BitmapText, (With<PageLabel>, Without<Heading>)>,
    mut headings: Query<(&Heading, &mut BitmapText), Without<PageLabel>>,
    added: Query<(), Or<(Added<Row>, Added<Heading>, Added<PageLabel>)>>,
    mut commands: Commands,
    buttons: Query<(Entity, &Choice, Has<bevy::ui::InteractionDisabled>)>,
) {
    let Some(view) = view.as_mut() else { return };
    let changed = view.is_changed()
        || facts.is_changed()
        || difficulty.is_changed()
        || factions.is_changed()
        || catalogs.is_changed()
        || strings.is_changed()
        || !added.is_empty();
    if !changed {
        return;
    }
    let catalog = &catalogs.facts;
    let mut entries: Vec<(String, String, bool)> = Vec::new();
    match view.mode {
        JournalMode::Events => {
            for entry in &facts.journal {
                let fact = &catalog[entry.fact];
                let text = entry.description(catalog);
                if text.is_empty() {
                    continue;
                }
                if !match fact.visibility {
                    JournalVisibility::AllDifficulties => true,
                    JournalVisibility::NoviceAndNormal => *difficulty != Difficulty::Expert,
                    JournalVisibility::NoviceOnly => *difficulty == Difficulty::Novice,
                    JournalVisibility::Hidden => false,
                } {
                    continue;
                }
                let location = crate::content::levels::LevelInfo::for_path(&entry.level)
                    .and_then(|level| level.name_string())
                    .map(|id| strings.get(id))
                    .unwrap_or("Unknown location");
                entries.push((
                    location.to_owned(),
                    text.to_owned(),
                    fact.highlight_when_true
                        && facts.get(entry.fact, catalog).is_ok_and(|v| v != 0),
                ));
            }
        }
        JournalMode::Topics => {
            for (category, text) in &facts.topics {
                entries.push((format!("{category:?}"), text.clone(), false));
            }
        }
        JournalMode::Factions => {
            use crate::game::factions::*;
            for (i, faction) in [
                UMPANI,
                TRANG,
                MOOK,
                RATTKIN_COMMON,
                RATTKIN_MAFIA,
                BROTHERHOOD,
                HIGARDI_COMMON,
                HIGARDI_LUNAR_LEGION,
                TRYNNIE,
                RAPAX_COMMON,
                RAPAX_TEMPLAR,
            ]
            .into_iter()
            .enumerate()
            {
                if factions.encountered(faction) {
                    use crate::game::factions::Band;
                    let id = match factions.band(faction) {
                        Band::Hostile => 0x200,
                        Band::Neutral => 0x1ff,
                        Band::Friendly => 0x201,
                    };
                    entries.push((
                        strings.get(id).to_owned(),
                        strings.get(0x6e0 + i).to_owned(),
                        false,
                    ));
                }
            }
        }
    }
    let count = entries.len();
    let last_page = count.saturating_sub(1) / 12;
    if view.pages_back > last_page {
        view.pages_back = last_page;
    }
    let selected_page = last_page - view.pages_back;
    for (row, mut text) in &mut rows {
        let index = selected_page * 12 + row.0;
        let entry = entries.get(index);
        let value = entry
            .map(|(left, right, _)| {
                if row.1 {
                    if view.mode == JournalMode::Events
                        && row.0 > 0
                        && entries[index - 1].0 == *left
                    {
                        String::new()
                    } else {
                        left.clone()
                    }
                } else {
                    right.clone()
                }
            })
            .unwrap_or_default();
        if text.value != value {
            text.value = value;
        }
        let palette = (!row.1 && entry.is_some_and(|e| e.2))
            .then(|| asset_server.load("retail://data/journal/journal_whitepal.sti"));
        if text.palette != palette {
            text.palette = palette;
        }
    }
    for (heading, mut text) in &mut headings {
        let value = match (view.mode, heading.0) {
            (JournalMode::Events, true) => "Location",
            (JournalMode::Events, false) => "Journal",
            (JournalMode::Topics, true) => "Category",
            (JournalMode::Topics, false) => "Topic",
            (JournalMode::Factions, true) => "Disposition",
            (JournalMode::Factions, false) => "Faction",
        };
        if text.value != value {
            text.value = value.into();
        }
    }
    let page = format!("{} / {}", selected_page + 1, last_page + 1);
    for mut title in &mut title {
        if title.value != page {
            title.value = page.clone();
        }
    }
    for (entity, choice, disabled) in &buttons {
        let enabled = match choice {
            Choice::Previous => selected_page > 0,
            Choice::Next => selected_page < last_page,
            _ => true,
        };
        super::retail_controls::set_enabled(&mut commands, entity, enabled, disabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::npcs::{FactDefinition, FactId};
    use crate::game::npcs::conversation::JournalEntry;

    #[test]
    fn filtering_uses_historical_text_and_pages_follow_visible_history() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<crate::game::factions::Factions>()
            .insert_resource(Difficulty::Expert)
            .init_resource::<InputFocus>()
            .init_resource::<NextState<Screen>>()
            .add_observer(activate)
            .add_systems(Update, refresh);
        let facts = crate::content::npcs::FactCatalog(vec![
            FactDefinition {
                id: FactId(1),
                symbol: "FACT_BURZ_SAID130".into(),
                description: "Before".into(),
                alternate_description: "After".into(),
                visibility: crate::content::npcs::JournalVisibility::AllDifficulties,
                highlight_when_true: false,
            },
            FactDefinition {
                id: FactId(2),
                symbol: "FACT_TEMPLAR".into(),
                description: "Novice hint".into(),
                alternate_description: String::new(),
                visibility: crate::content::npcs::JournalVisibility::NoviceOnly,
                highlight_when_true: false,
            },
        ]);
        app.insert_resource(crate::game::data::Catalogs { facts, ..default() });
        let mut facts = StoryFacts::default();
        for index in 0..13 {
            facts.journal.push(JournalEntry {
                fact: FactId(1),
                was_true: index == 12,
                level: String::new(),
            });
        }
        facts.journal.push(JournalEntry { fact: FactId(2), was_true: false, level: String::new() });
        facts.journal.push(JournalEntry { fact: FactId(2), was_true: true, level: String::new() });
        app.insert_resource(facts);
        let view = app.world_mut().spawn(Journal::default()).id();
        let row = app.world_mut().spawn((Row(0, false), BitmapText::default())).id();
        let title = app.world_mut().spawn((PageLabel, BitmapText::default())).id();
        app.update();
        let journal = app.world().get::<Journal>(view).unwrap();
        assert_eq!(journal.pages_back, 0);
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "2 / 2");
        assert_eq!(app.world().get::<BitmapText>(row).unwrap().value, "After");
        *app.world_mut().resource_mut::<Difficulty>() = Difficulty::Novice;
        app.update();
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "2 / 2");
        let previous = app.world_mut().spawn(Choice::Previous).id();
        let next = app.world_mut().spawn(Choice::Next).id();
        let mode = app.world_mut().spawn(Choice::Topics).id();
        app.world_mut().trigger(Activate { entity: previous });
        app.update();
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "1 / 2");
        app.world_mut().trigger(Activate { entity: next });
        app.update();
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "2 / 2");
        app.world_mut().trigger(Activate { entity: previous });
        // Switching to equally long history still selects its newest page.
        for i in 0..14 {
            app.world_mut()
                .resource_mut::<StoryFacts>()
                .topics
                .push((crate::content::npcs::TopicCategory::Misc, format!("Topic {i:02}")));
        }
        app.world_mut().trigger(Activate { entity: mode });
        app.update();
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "2 / 2");
        assert_eq!(app.world().get::<BitmapText>(row).unwrap().value, "Topic 12");
        app.world_mut().trigger(Activate { entity: previous });
        // Exact-page shrink clamps the selected offset, without a fake count.
        app.world_mut()
            .resource_mut::<StoryFacts>()
            .topics
            .retain(|(_, t)| t.as_str() < "Topic 12");
        app.update();
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "1 / 1");
        assert!(app.world().get::<bevy::ui::InteractionDisabled>(previous).is_some());
        assert!(app.world().get::<bevy::ui::InteractionDisabled>(next).is_some());
        // Return through Factions to Events, then empty it.
        app.world_mut().trigger(Activate { entity: mode });
        app.world_mut().trigger(Activate { entity: mode });
        app.world_mut().resource_mut::<StoryFacts>().journal.clear();
        app.update();
        let journal = app.world().get::<Journal>(view).unwrap();
        assert_eq!(journal.pages_back, 0);
        assert_eq!(app.world().get::<BitmapText>(title).unwrap().value, "1 / 1");
        assert!(app.world().get::<BitmapText>(row).unwrap().value.is_empty());
        // With nothing it shows changing, the page is left alone.
        app.update();
        app.update();
        let text = app.world_mut().query::<Ref<BitmapText>>().get(app.world(), row).unwrap();
        assert!(!text.is_changed(), "an idle journal does not redraw");
    }
}
