//! Main game screen around the 3D view, based on the decomp's
//! `MainGameScreen.cpp` and `MGS*.cpp`: roof, party columns, bottom panels
//! and button row, in retail's three layout modes.
//! The 3D view renders into the viewport the layout leaves open.
//!
//! The frame holds what the layout alone decides and is built again when
//! the layout changes. Each part showing game state (the party's slots, the
//! text box, the radar, the formation board, the button bank) has its own
//! node in the frame. Controls and markers persist; their source data and
//! geometry change in place.

mod bank;
mod formation;
pub(crate) mod keys;
mod layout;
mod magic_view;
mod party;
mod radar;
mod text_box;

use bevy::input_focus::{InputFocus, InputFocusSystems, tab_navigation::TabGroup};
use bevy::prelude::*;

use super::screen_art::{SCREEN, absolute, centred, screen, sprite};
use crate::content::ui::SpriteSheet;
pub use layout::{LayoutMode, Panel, ScreenLayout};

pub struct MainScreenPlugin;

impl Plugin for MainScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<bevy::picking::PickingSettings>();
        app.world_mut().resource_mut::<bevy::picking::PickingSettings>().multi_click_interval =
            std::time::Duration::from_millis(400);
        app.init_resource::<ScreenLayout>()
            .init_resource::<SelectedMember>()
            .add_plugins(keys::plugin)
            .add_systems(
                Startup,
                (
                    load_main_screen_art,
                    magic_view::load_magic_view_art,
                    layout::spawn_screen_cameras,
                ),
            )
            .add_systems(PreUpdate, route_hud_focus.before(InputFocusSystems::Dispatch))
            .add_systems(
                Update,
                (
                    layout::fit_viewport,
                    build_frame.run_if(in_state(crate::game::session::Session::Playing)),
                    (
                        (
                            party::build_party_parts,
                            party::place_party_buttons,
                            party::open_held_reviews,
                            party::draw_party,
                        )
                            .chain(),
                        (party::record_splats, party::play_splats).chain(),
                        (text_box::draw_text_box, text_box::sync_channels, text_box::repeat_arrows)
                            .chain(),
                        (radar::track_radar_items, radar::update_radar)
                            .chain()
                            .after(crate::game::GameplaySet::LevelActivation)
                            .after(crate::game::world::search::search_pulse),
                        formation::draw_formation,
                        bank::draw_bank,
                        bank::draw_caption,
                        bank::show_round_progress,
                        magic_view::refresh_magic_view,
                    ),
                )
                    .chain()
                    .after(super::automap::scale_ui)
                    .after(super::notices::queue_notices),
            );
    }
}

/// The member the buttons and keys act for (retail's selected character).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct SelectedMember(pub Option<Entity>);

impl SelectedMember {
    /// An explicit selection follows its actor through roster reordering.
    /// With no current selection, display the first remaining member.
    pub fn member(&self, roster: &crate::game::party::roster::PartyRoster) -> Option<Entity> {
        self.0.filter(|m| roster.0.contains(m)).or_else(|| roster.0.first().copied())
    }
}

#[derive(Resource)]
pub struct MainScreenArt {
    roof: Handle<SpriteSheet>,
    columns: Handle<SpriteSheet>,
    bottom_bar: Handle<SpriteSheet>,
    radar_back: Handle<SpriteSheet>,
    textbox_back: Handle<SpriteSheet>,
    textbox_tabs: Handle<SpriteSheet>,
    text_scroll: Handle<SpriteSheet>,
    formation_back: Handle<SpriteSheet>,
    roof_buttons: Handle<SpriteSheet>,
    layout_arrows: Handle<SpriteSheet>,
    scroll_arrows: Handle<SpriteSheet>,
    icons: Handle<SpriteSheet>,
    options: Handle<SpriteSheet>,
    radar_disc: Handle<SpriteSheet>,
    formation_cage: Handle<SpriteSheet>,
    formation_symbols: Handle<SpriteSheet>,
    formation_jewels: Handle<SpriteSheet>,
    radar_rim: Handle<SpriteSheet>,
    view_cone: Handle<SpriteSheet>,
    combat_toggle: Handle<SpriteSheet>,
    attack_confirm: Handle<SpriteSheet>,
    cont_toggle: Handle<SpriteSheet>,
    combat_stop: Handle<SpriteSheet>,
    cont_start: Handle<SpriteSheet>,
    progress: Handle<SpriteSheet>,
    column_info: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    small_font: Handle<SpriteSheet>,
    bold_font: Handle<SpriteSheet>,
    damage_splat: Handle<SpriteSheet>,
    damage_splat2: Handle<SpriteSheet>,
    death_splat: Handle<SpriteSheet>,
    /// All of the above, to know when they have loaded.
    all: Vec<Handle<SpriteSheet>>,
}

pub fn load_main_screen_art(mut commands: Commands, assets: Res<AssetServer>) {
    let mut all = vec![];
    let mut load = |path: &str| {
        let handle: Handle<SpriteSheet> = assets.load(format!("retail://data/{path}.sti"));
        all.push(handle.clone());
        handle
    };
    let mut art = MainScreenArt {
        roof: load("main interface/main_roof"),
        columns: load("main interface/main_columns_normal"),
        bottom_bar: load("main interface/main_bottom_bar"),
        radar_back: load("main interface/main_radar_back"),
        textbox_back: load("main interface/main_textbox_back"),
        textbox_tabs: load("main interface/main_textbox_tabs"),
        text_scroll: load("main interface/main_scroll"),
        formation_back: load("main interface/small_formation_back"),
        roof_buttons: load("main interface/main_roof_buttons"),
        layout_arrows: load("main interface/main_layout_arrows"),
        scroll_arrows: load("main interface/main_scroll_arrows"),
        icons: load("main interface/icons_standard"),
        options: load("main interface/options_disk"),
        radar_disc: load("radar compass/formation_dot_small"),
        formation_cage: load("party formation/small_formation_cage"),
        formation_symbols: load("party formation/small_formation_symbol"),
        formation_jewels: load("party formation/small_formation_jewels"),
        radar_rim: load("radar compass/compass_rim"),
        view_cone: load("party formation/small_formation_cone"),
        combat_toggle: load("main interface/icon_combat_toggle"),
        attack_confirm: load("main interface/attack_confirm"),
        cont_toggle: load("main interface/cont_toggle"),
        combat_stop: load("main interface/combat_stop"),
        cont_start: load("main interface/cont_start"),
        progress: load("main interface/progressbar"),
        column_info: load("main interface/main_column_info_normal"),
        font: load("fonts/wiz_text_font"),
        small_font: load("fonts/smfnt"),
        bold_font: load("fonts/wiz_text_font_bold"),
        damage_splat: load("main interface/damage_splat_anim"),
        damage_splat2: load("main interface/damage_splat_anim2"),
        death_splat: load("main interface/death_splat_anim"),
        all: vec![],
    };
    art.all = all;
    commands.insert_resource(art);
}

/// One of the screen's sheets. The frame is built only once all of them
/// have loaded, and its parts draw only inside it.
fn loaded<'a>(sheets: &'a Assets<SpriteSheet>, handle: &Handle<SpriteSheet>) -> &'a SpriteSheet {
    sheets.get(handle).expect("the main screen's art loads before its frame is built")
}

/// An empty node whose children are placed in screen pixels: a part's
/// node in the frame.
fn layer() -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute, width: px(0), height: px(0) }
        Pickable::IGNORE
    }
}

/// A sheet's frame that is a button: it catches the pointer.
fn button(sheet: Handle<SpriteSheet>, frame: usize, at: Vec2) -> impl Scene {
    crate::presentation::retail_controls::image_button(
        sheet,
        at,
        crate::presentation::retail_controls::ControlSkin::new(frame, frame, frame, frame, frame),
    )
}

/// The screen's root, with the layout its frame was built for.
#[derive(Component, Clone, Default)]
pub struct MainScreen(ScreenLayout);

fn leave_hud_focus(
    mut event: On<bevy::input_focus::FocusedInput<bevy::input::keyboard::KeyboardInput>>,
    mut focus: ResMut<InputFocus>,
) {
    if event.input.state.is_pressed() && event.input.key_code == KeyCode::Escape {
        event.propagate(false);
        focus.clear();
    }
}

/// Other screens own keyboard navigation while covering the HUD. A modal tab
/// group is excluded from global navigation, without hiding the visible HUD.
fn route_hud_focus(
    screen: Res<State<crate::game::screen::Screen>>,
    next: Res<NextState<crate::game::screen::Screen>>,
    questions: Query<(), With<crate::game::world::level::TravelQuestion>>,
    roots: Query<Entity, With<MainScreen>>,
    parents: Query<&ChildOf>,
    mut focus: ResMut<InputFocus>,
) {
    use crate::game::screen::Screen;
    let active = *screen.get() == Screen::World
        && matches!(*next, NextState::Unchanged | NextState::Pending(Screen::World))
        && questions.is_empty();
    for root in &roots {
        if !active && focus.get().is_some_and(|e| parents.iter_ancestors(e).any(|p| p == root)) {
            focus.clear();
        }
    }
}

/// Build the frame for the layout, once the art has loaded: black around
/// the viewport, the roof, the columns, the bottom posts and panels, and a
/// node for each part. Building it again gives every part an empty node.
fn build_frame(
    mut commands: Commands,
    layout: Res<ScreenLayout>,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    screens: Query<(Entity, &MainScreen)>,
) {
    if screens.iter().any(|(_, built)| built.0 == *layout)
        || !art.all.iter().all(|h| sheets.contains(h))
    {
        return;
    }
    for (screen, _) in &screens {
        commands.entity(screen).despawn();
    }
    let sheet = |handle| loaded(&sheets, handle);
    // Black wherever the 3D view is not: outside the screen (the sky
    // camera's clear covers the whole window) and behind the art.
    let view = layout.viewport();
    const FAR: f32 = 10_000.0;
    let masks: Vec<_> = [
        (Vec2::new(-FAR, -FAR), Vec2::new(FAR, SCREEN.y + 2.0 * FAR)),
        (Vec2::new(SCREEN.x, -FAR), Vec2::new(FAR, SCREEN.y + 2.0 * FAR)),
        (Vec2::new(0.0, -FAR), Vec2::new(SCREEN.x, FAR + view.min.y)),
        (Vec2::new(0.0, view.max.y), Vec2::new(SCREEN.x, FAR)),
        (Vec2::new(0.0, view.min.y), Vec2::new(view.min.x, view.height())),
        (Vec2::new(view.max.x, view.min.y), Vec2::new(SCREEN.x - view.max.x, view.height())),
    ]
    .into_iter()
    .map(|(at, size)| {
        let node = absolute(at, size);
        bsn! { node BackgroundColor(Color::BLACK) Pickable::IGNORE }
    })
    .collect();
    let closed = layout.mode == LayoutMode::Closed;
    let (columns, bar) = (sheet(&art.columns), sheet(&art.bottom_bar));
    let mut parts = vec![sprite(sheet(&art.roof), 0, Vec2::ZERO)];
    // Party columns: four rows of 85 pixels from y 18.
    for row in 0..4 {
        let y = 18.0 + 85.0 * row as f32;
        if closed {
            parts.push(sprite(columns, 1 + 4 * row, Vec2::new(0.0, y)));
            parts.push(sprite(columns, 2 + 4 * row, Vec2::new(512.0, y)));
            // The arch over each portrait.
            parts.push(sprite(columns, 16, Vec2::new(23.0, y)));
            parts.push(sprite(columns, 16, Vec2::new(535.0, y)));
        } else {
            parts.push(sprite(columns, 4 * row, Vec2::new(0.0, y)));
            parts.push(sprite(columns, 3 + 4 * row, Vec2::new(617.0, y)));
        }
    }
    // The bottom posts and panels.
    parts.push(sprite(bar, 0, Vec2::new(0.0, 358.0)));
    parts.push(sprite(bar, 1, Vec2::new(617.0, 358.0)));
    for (panel, back, x) in [
        (Panel::Radar, &art.radar_back, 23.0),
        (Panel::TextBox, &art.textbox_back, 128.0),
        (Panel::Formation, &art.formation_back, 512.0),
    ] {
        if layout.shows(panel) {
            parts.push(sprite(sheet(back), 0, Vec2::new(x, 358.0)));
        }
    }
    // The button row's sockets.
    for (frame, x) in
        [(2, 0.0), (4, 30.0), (8, 196.0), (5, 292.0), (6, 343.0), (7, 528.0), (3, 610.0)]
    {
        parts.push(sprite(bar, frame, Vec2::new(x, 450.0)));
    }
    let shown = |panel| layout.shows(panel).then_some(());
    commands.spawn_scene(bsn! {
        @centred()
        MainScreen({layout.clone()})
        TabGroup::modal()
        on(leave_hud_focus)
        GlobalZIndex(-5)
        Pickable::IGNORE
        Children [
            @screen()
            Pickable::IGNORE
            Children [
                {masks}
                // Portraits sit under the column art, which frames them.
                -- {closed.then(party::portraits)}
                -- {parts}
                -- {party::slots(closed)}
                -- {layout::buttons(&layout, &art, &sheets)}
                -- {shown(Panel::TextBox).map(|()| text_box::panel())}
                -- {shown(Panel::Radar).map(|()| radar::panel(&art, &sheets))}
                -- {shown(Panel::Formation).map(|()| formation::panel(&art, &sheets))}
                -- {bank::row(&art, &sheets)}
            ]
        ]
    });
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use crate::presentation::test_input::tap;
    use bevy::input_focus::tab_navigation::TabIndex;
    use bevy::input_focus::{InputDispatchPlugin, tab_navigation::TabNavigationPlugin};
    use bevy::ui_widgets::Button;
    use bevy::ui_widgets::ButtonPlugin;
    use bevy::window::PrimaryWindow;
    use bevy_enhanced_input::prelude::*;

    #[test]
    fn f6_enters_hud_without_stealing_the_worlds_map_binding() {
        use crate::presentation::{
            automap::{MapKey, OpenAutomap, spawn_map_key},
            notices::TextLog,
        };
        let mut app = crate::presentation::test_input::input_app();
        app.add_plugins((InputDispatchPlugin, TabNavigationPlugin, ButtonPlugin))
            .init_state::<crate::game::screen::Screen>()
            .init_resource::<ScreenLayout>()
            .init_resource::<SelectedMember>()
            .init_resource::<crate::game::party::roster::PartyRoster>()
            .init_resource::<crate::game::combat::CombatSettings>()
            .init_resource::<TextLog>()
            .add_message::<crate::game::combat::StartRound>()
            .add_message::<OpenAutomap>()
            .add_input_context::<MapKey>()
            .add_systems(Startup, spawn_map_key)
            .add_plugins(keys::plugin);
        let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        let root = app
            .world_mut()
            .spawn((MainScreen::default(), TabGroup::modal()))
            .observe(leave_hud_focus)
            .id();
        let first = app.world_mut().spawn((Button, TabIndex(0), ChildOf(root))).id();
        let second = app.world_mut().spawn((Button, TabIndex(0), ChildOf(root))).id();
        app.finish();
        app.cleanup();
        app.update();
        app.update();
        tap(&mut app, window, KeyCode::Tab);
        assert_eq!(app.world_mut().resource_mut::<Messages<OpenAutomap>>().drain().count(), 1);
        tap(&mut app, window, KeyCode::F6);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(first));
        tap(&mut app, window, KeyCode::Tab);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(second));
        assert_eq!(app.world_mut().resource_mut::<Messages<OpenAutomap>>().drain().count(), 0);
        tap(&mut app, window, KeyCode::Escape);
        assert_ne!(app.world().resource::<InputFocus>().get(), Some(second));
        tap(&mut app, window, KeyCode::Tab);
        assert_eq!(app.world_mut().resource_mut::<Messages<OpenAutomap>>().drain().count(), 1);
    }
}
