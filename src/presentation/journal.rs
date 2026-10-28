//! Retail fact journal: twelve historical transitions per page, newest page first.
use super::screen_art::{BitmapText, spawn_picture};
use crate::content::npcs::FactCatalog;
use crate::game::{
    difficulty::Difficulty,
    npcs::{NpcAssets, conversation::StoryFacts},
    screen::Screen,
};
use bevy::input_focus::{FocusCause, InputFocus, tab_navigation::TabGroup};
use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use bevy_enhanced_input::prelude::*;

#[derive(Component, Clone, Default)]
struct Journal {
    page: usize,
    count: usize,
    mode: u8,
    previous_focus: Option<Entity>,
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
    mut state: ParamSet<(crate::game::screen::Freedom, ResMut<NextState<Screen>>)>,
) {
    if requests.read().count() > 0 && state.p0().free() {
        state.p1().set(Screen::Journal);
    }
}
fn enter(mut commands: Commands, assets: Res<AssetServer>, mut focus: ResMut<InputFocus>) {
    let view = Journal { previous_focus: focus.get(), ..default() };
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
            BitmapText {
                font: assets.load("retail://data/journal/journal_font.sti"),
                at: Vec2::new(if left { 11. } else { 193. }, 25.),
                width: if left { 175. } else { 435. },
                justify: bevy::text::Justify::Center,
                ..default()
            },
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
        BitmapText {
            font: assets.load("retail://data/fonts/opt_detail_font.sti"),
            at: Vec2::new(135., 450.),
            width: 245.,
            justify: bevy::text::Justify::Center,
            ..default()
        },
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
        .observe(
            |_: On<Start<Close>>,
             view: Single<&Journal>,
             mut next: ResMut<NextState<Screen>>,
             mut focus: ResMut<InputFocus>| {
                close(&view, &mut next, &mut focus);
            },
        )
        .observe(|_: On<Start<Previous>>, mut view: Single<&mut Journal>| {
            view.page = view.page.saturating_sub(1);
        })
        .observe(|_: On<Start<Next>>, mut view: Single<&mut Journal>| {
            view.page = (view.page + 1).min(view.count.saturating_sub(1) / 12);
        });
}
fn close(view: &Journal, next: &mut NextState<Screen>, focus: &mut InputFocus) {
    next.set(Screen::World);
    if let Some(e) = view.previous_focus {
        focus.set(e, FocusCause::Navigated);
    } else {
        focus.clear();
    }
}
fn activate(
    event: On<Activate>,
    choices: Query<&Choice>,
    mut view: Option<Single<&mut Journal>>,
    mut next: ResMut<NextState<Screen>>,
    mut focus: ResMut<InputFocus>,
) {
    let (Ok(choice), Some(view)) = (choices.get(event.entity), view.as_mut()) else { return };
    match choice {
        Choice::Previous => view.page = view.page.saturating_sub(1),
        Choice::Next => view.page = (view.page + 1).min(view.count.saturating_sub(1) / 12),
        Choice::Topics => {
            view.mode = (view.mode + 1) % 3;
            view.count = usize::MAX;
        }
        Choice::Close => close(view, &mut next, &mut focus),
    }
}
fn refresh(
    mut view: Option<Single<&mut Journal>>,
    facts: Res<StoryFacts>,
    assets: Res<NpcAssets>,
    catalogs: Res<Assets<FactCatalog>>,
    difficulty: Res<Difficulty>,
    factions: Res<crate::game::factions::Factions>,
    strings: super::screen_art::GameStrings,
    asset_server: Res<AssetServer>,
    mut rows: Query<(&Row, &mut BitmapText), (Without<PageLabel>, Without<Heading>)>,
    mut title: Query<&mut BitmapText, (With<PageLabel>, Without<Heading>)>,
    mut headings: Query<(&Heading, &mut BitmapText), Without<PageLabel>>,
    mut commands: Commands,
    buttons: Query<(Entity, &Choice, Has<bevy::ui::InteractionDisabled>)>,
) {
    let Some(view) = view.as_mut() else { return };
    let Some(catalog) = catalogs.get(&assets.facts) else { return };
    let visibility = match *difficulty {
        Difficulty::Novice => 2,
        Difficulty::Normal => 1,
        Difficulty::Expert => 0,
    };
    let mut entries: Vec<(String, String, bool)> = Vec::new();
    match view.mode {
        0 => {
            for entry in &facts.journal {
                let Some(fact) = catalog.get(entry.fact) else { continue };
                let Some(text) = entry.description(catalog).filter(|text| !text.is_empty()) else {
                    continue;
                };
                if fact.visibility > visibility {
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
        1 => {
            for (category, text) in &facts.topics {
                entries.push((format!("{category:?}"), text.clone(), false));
            }
        }
        _ => {
            for (i, faction) in [4, 5, 6, 7, 8, 9, 12, 11, 13, 15, 16].into_iter().enumerate() {
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
    if count != view.count {
        view.count = count;
        view.page = count.saturating_sub(1) / 12;
    }
    for (row, mut text) in &mut rows {
        let index = view.page * 12 + row.0;
        let entry = entries.get(index);
        text.value = entry
            .map(|(left, right, _)| {
                if row.1 {
                    if view.mode == 0 && row.0 > 0 && entries[index - 1].0 == *left {
                        String::new()
                    } else {
                        left.clone()
                    }
                } else {
                    right.clone()
                }
            })
            .unwrap_or_default();
        text.palette = (!row.1 && entry.is_some_and(|e| e.2))
            .then(|| asset_server.load("retail://data/journal/journal_whitepal.sti"));
    }
    for (heading, mut text) in &mut headings {
        text.value = match (view.mode, heading.0) {
            (0, true) => "Location",
            (0, false) => "Journal",
            (1, true) => "Category",
            (1, false) => "Topic",
            (_, true) => "Disposition",
            (_, false) => "Faction",
        }
        .into();
    }
    for mut title in &mut title {
        title.value = format!("{} / {}", view.page + 1, count.saturating_sub(1) / 12 + 1);
    }
    for (entity, choice, disabled) in &buttons {
        let enabled = match choice {
            Choice::Previous => view.page > 0,
            Choice::Next => view.page < count.saturating_sub(1) / 12,
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
            .init_asset::<FactCatalog>()
            .init_resource::<crate::game::factions::Factions>()
            .insert_resource(Difficulty::Expert)
            .add_systems(Update, refresh);
        let catalog = app.world_mut().resource_mut::<Assets<FactCatalog>>().add(FactCatalog(vec![
            FactDefinition {
                id: FactId(1),
                symbol: "FACT_BURZ_SAID130".into(),
                description: "Before".into(),
                alternate_description: "After".into(),
                visibility: 0,
                highlight_when_true: false,
            },
            FactDefinition {
                id: FactId(2),
                symbol: "FACT_TEMPLAR".into(),
                description: "Novice hint".into(),
                alternate_description: String::new(),
                visibility: 2,
                highlight_when_true: false,
            },
        ]));
        app.insert_resource(NpcAssets { catalog: default(), facts: catalog, scripts: default() });
        let mut facts = StoryFacts::default();
        for index in 0..13 {
            facts.journal.push(JournalEntry {
                fact: FactId(1),
                value: u8::from(index == 12),
                level: String::new(),
            });
        }
        facts.journal.push(JournalEntry { fact: FactId(2), value: 0, level: String::new() });
        facts.journal.push(JournalEntry { fact: FactId(2), value: 1, level: String::new() });
        app.insert_resource(facts);
        let view = app.world_mut().spawn(Journal::default()).id();
        let row = app.world_mut().spawn((Row(0, false), BitmapText::default())).id();
        app.update();
        let journal = app.world().get::<Journal>(view).unwrap();
        assert_eq!((journal.page, journal.count), (1, 13));
        assert_eq!(app.world().get::<BitmapText>(row).unwrap().value, "After");
        *app.world_mut().resource_mut::<Difficulty>() = Difficulty::Novice;
        app.update();
        assert_eq!(
            app.world().get::<Journal>(view).unwrap().count,
            14,
            "empty alternate text is excluded"
        );
        app.world_mut().resource_mut::<StoryFacts>().journal.clear();
        app.update();
        let journal = app.world().get::<Journal>(view).unwrap();
        assert_eq!((journal.page, journal.count), (0, 0));
        assert!(app.world().get::<BitmapText>(row).unwrap().value.is_empty());
    }
}
