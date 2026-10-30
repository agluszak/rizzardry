//! Main game screen around the 3D view, based on the decomp's
//! `MainGameScreen.cpp` and `MGS*.cpp`: roof, party columns, bottom panels
//! and button row, in retail's three layout modes.
//! The 3D view renders into the viewport the layout leaves open.
//!
//! Layout changes update the retained frame and controls in place.

mod bank;
mod formation;
pub(crate) mod keys;
mod layout;
pub(super) mod magic_view;
mod party;
mod radar;
mod text_box;

use bevy::input_focus::{InputFocus, InputFocusSystems, tab_navigation::TabGroup};
use bevy::prelude::*;

use super::screen_art::{SCREEN, absolute, centred, screen, sprite};
use crate::content::ui::SpriteSheet;
use crate::game::GameplaySet;
pub use layout::{LayoutMode, Panel, ScreenLayout};

pub struct MainScreenPlugin;

impl Plugin for MainScreenPlugin {
    fn build(&self, app: &mut App) {
        use crate::game::world::items::WorldItem;
        // Presentation history is attached immediately, even before the HUD exists.
        app.register_required_components::<WorldItem, radar::RadarItem>();
        app.init_resource::<ScreenLayout>()
            .init_resource::<SelectedMember>()
            .add_plugins(keys::plugin)
            .add_plugins(magic_view::plugin)
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
                    build_frame.run_if(in_state(crate::presentation::frontend::Session::Playing)),
                    update_frame,
                    layout::draw_buttons,
                    (
                        (
                            party::build_party_parts,
                            party::place_party_buttons,
                            party::open_held_reviews,
                            party::draw_party,
                        )
                            .chain()
                            .after(GameplaySet::Stats),
                        (party::record_splats, party::play_splats).chain(),
                        (text_box::draw_text_box, text_box::sync_channels, text_box::repeat_arrows)
                            .chain(),
                        (radar::track_radar_items, radar::update_radar)
                            .chain()
                            .after(GameplaySet::LevelActivation)
                            .after(crate::game::world::search::search_pulse),
                        formation::draw_formation,
                        bank::draw_bank,
                        bank::draw_caption,
                        bank::show_round_progress,
                        magic_view::refresh_magic_view,
                    ),
                )
                    .chain()
                    .after(super::screen_art::scale_ui)
                    .after(super::notices::QueueNotices),
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
        self.0.filter(|m| roster.contains(m)).or_else(|| roster.first().copied())
    }
}

#[derive(Resource)]
#[cfg_attr(test, derive(Default))]
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
        let handle: Handle<SpriteSheet> = assets.load(crate::content::paths::sheet(path));
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

/// The retained screen root.
#[derive(Component, Clone, Default)]
pub struct MainScreen;

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
    screen: Res<State<crate::presentation::screen::Screen>>,
    next: Res<NextState<crate::presentation::screen::Screen>>,
    questions: Query<(), With<crate::game::world::level::TravelQuestion>>,
    roots: Query<Entity, With<MainScreen>>,
    parents: Query<&ChildOf>,
    mut focus: ResMut<InputFocus>,
) {
    use crate::presentation::screen::Screen;
    let active = *screen.get() == Screen::World
        && matches!(*next, NextState::Unchanged | NextState::Pending(Screen::World))
        && questions.is_empty();
    for root in &roots {
        if !active && focus.get().is_some_and(|e| parents.iter_ancestors(e).any(|p| p == root)) {
            focus.clear();
        }
    }
}

#[derive(Component, Clone)]
enum FrameVisibility {
    Closed(bool),
    Panel(Panel),
}

#[derive(Component, Clone, Default)]
struct ViewMask(usize);

fn masks(layout: &ScreenLayout) -> [(Vec2, Vec2); 6] {
    let view = layout.viewport();
    const FAR: f32 = 10_000.0;
    [
        (Vec2::new(-FAR, -FAR), Vec2::new(FAR, SCREEN.y + 2.0 * FAR)),
        (Vec2::new(SCREEN.x, -FAR), Vec2::new(FAR, SCREEN.y + 2.0 * FAR)),
        (Vec2::new(0.0, -FAR), Vec2::new(SCREEN.x, FAR + view.min.y)),
        (Vec2::new(0.0, view.max.y), Vec2::new(SCREEN.x, FAR)),
        (Vec2::new(0.0, view.min.y), Vec2::new(view.min.x, view.height())),
        (Vec2::new(view.max.x, view.min.y), Vec2::new(SCREEN.x - view.max.x, view.height())),
    ]
}

fn update_frame(
    layout: Res<ScreenLayout>,
    mut nodes: Query<
        (Entity, &mut Node, &mut Visibility, Option<&FrameVisibility>, Option<&ViewMask>),
        Or<(With<FrameVisibility>, With<ViewMask>)>,
    >,
    parents: Query<&ChildOf>,
    mut focus: ResMut<InputFocus>,
) {
    let masks = masks(&layout);
    for (entity, mut node, mut visible, visibility, mask) in &mut nodes {
        if let Some(visibility) = visibility {
            let shown = match visibility {
                FrameVisibility::Closed(closed) => *closed == (layout.mode == LayoutMode::Closed),
                FrameVisibility::Panel(panel) => layout.shows(*panel),
            };
            visible.set_if_neq(crate::presentation::screen_art::visible_if(shown));
            if !shown
                && focus.get().is_some_and(|focused| {
                    focused == entity
                        || parents.iter_ancestors(focused).any(|parent| parent == entity)
                })
            {
                focus.clear();
            }
        }
        if let Some(mask) = mask {
            let (at, size) = masks[mask.0];
            node.set_if_neq(absolute(at, size));
        }
    }
}

/// Build once. Decorations may be hidden, but controls retain their identities.
fn build_frame(
    mut commands: Commands,
    layout: Res<ScreenLayout>,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    screens: Query<(), With<MainScreen>>,
) {
    if !screens.is_empty() || !art.all.iter().all(|h| sheets.contains(h)) {
        return;
    }
    let sheet = |handle| loaded(&sheets, handle);
    let masks: Vec<_> = masks(&layout)
        .into_iter()
        .enumerate()
        .map(|(index, (at, size))| {
            let node = absolute(at, size);
            bsn! { node ViewMask({index}) BackgroundColor(Color::BLACK) Pickable::IGNORE }
        })
        .collect();
    let columns: Vec<_> = [true, false].into_iter().map(|closed| {
        let mut parts = vec![];
        for row in 0..4 {
            let y = 18.0 + 85.0 * row as f32;
            let columns = sheet(&art.columns);
            if closed {
                parts.push(sprite(columns, 1 + 4 * row, Vec2::new(0.0, y)));
                parts.push(sprite(columns, 2 + 4 * row, Vec2::new(512.0, y)));
                parts.push(sprite(columns, 16, Vec2::new(23.0, y)));
                parts.push(sprite(columns, 16, Vec2::new(535.0, y)));
            } else {
                parts.push(sprite(columns, 4 * row, Vec2::new(0.0, y)));
                parts.push(sprite(columns, 3 + 4 * row, Vec2::new(617.0, y)));
            }
        }
        bsn! { @layer() template(move |_| Ok(FrameVisibility::Closed(closed))) Children [{parts}] }
    }).collect();
    let bar = sheet(&art.bottom_bar);
    let mut parts = vec![
        sprite(sheet(&art.roof), 0, Vec2::ZERO),
        sprite(bar, 0, Vec2::new(0.0, 358.0)),
        sprite(bar, 1, Vec2::new(617.0, 358.0)),
    ];
    for (frame, x) in
        [(2, 0.0), (4, 30.0), (8, 196.0), (5, 292.0), (6, 343.0), (7, 528.0), (3, 610.0)]
    {
        parts.push(sprite(bar, frame, Vec2::new(x, 450.0)));
    }
    commands.spawn_scene(bsn! {
        @centred() MainScreen TabGroup::modal() on(leave_hud_focus) GlobalZIndex(-5) Pickable::IGNORE
        Children [
            @screen() Pickable::IGNORE Children [
                {masks}
                -- @layer() template(|_| Ok(FrameVisibility::Closed(true))) Children [{party::portraits()}]
                -- {columns}
                -- {parts}
                -- {party::slots()}
                -- @layer() template(|_| Ok(FrameVisibility::Closed(true))) Children [@layer() party::Splats]
                -- {layout::buttons(&art)}
                -- @layer() template(|_| Ok(FrameVisibility::Panel(Panel::TextBox))) Children [
                    {vec![sprite(sheet(&art.textbox_back), 0, Vec2::new(128.0,358.0))]}
                    -- {text_box::panel()}
                ]
                -- @layer() template(|_| Ok(FrameVisibility::Panel(Panel::Radar))) Children [
                    {vec![sprite(sheet(&art.radar_back), 0, Vec2::new(23.0,358.0))]}
                    -- {radar::panel(&art,&sheets)}
                ]
                -- @layer() template(|_| Ok(FrameVisibility::Panel(Panel::Formation))) Children [
                    {vec![sprite(sheet(&art.formation_back), 0, Vec2::new(512.0,358.0))]}
                    -- {formation::panel(&art,&sheets)}
                ]
                -- {bank::row(&art,&sheets)}
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
    fn layout_changes_keep_hud_controls_and_focus() {
        use crate::game::characters::{Character, Gender, Profession, Race, Vitals};
        let mut app = crate::presentation::test_input::input_app();
        app.init_asset::<SpriteSheet>()
            .init_resource::<ScreenLayout>()
            .init_resource::<MainScreenArt>()
            .add_systems(
                Update,
                (build_frame, update_frame, party::build_party_parts, party::place_party_buttons)
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<Assets<SpriteSheet>>()
            .insert(
                Handle::<SpriteSheet>::default().id(),
                SpriteSheet { frames: vec![], images: vec![], palette: None },
            )
            .unwrap();
        let character = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let member = app.world_mut().spawn((Vitals::new(&character), character)).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.update();
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<MainScreen>>()
            .single(app.world())
            .unwrap();
        let control = app
            .world_mut()
            .query_filtered::<Entity, With<party::SlotButton>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(control, bevy::input_focus::FocusCause::Navigated);
        let panels: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<FrameVisibility>>()
            .iter(app.world())
            .collect();
        for mode in [LayoutMode::Closed, LayoutMode::Full, LayoutMode::Wide] {
            app.world_mut().resource_mut::<ScreenLayout>().mode = mode;
            app.update();
            assert!(app.world().get_entity(root).is_ok());
            assert_eq!(app.world().resource::<InputFocus>().get(), Some(control));
            assert!(panels.iter().all(|&e| app.world().get_entity(e).is_ok()));
            let node = app.world().get::<Node>(control).unwrap();
            assert_eq!(node.width, px(if mode == LayoutMode::Closed { 82. } else { 22. }));
        }
        let panel = app
            .world_mut()
            .query::<(Entity, &FrameVisibility)>()
            .iter(app.world())
            .find_map(|(e, v)| matches!(v, FrameVisibility::Panel(Panel::TextBox)).then_some(e))
            .unwrap();
        let child = app.world_mut().spawn((Button, TabIndex(0), ChildOf(panel))).id();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(child, bevy::input_focus::FocusCause::Navigated);
        app.world_mut().resource_mut::<ScreenLayout>().mode = LayoutMode::Full;
        app.update();
        assert_eq!(app.world().get::<Visibility>(panel), Some(&Visibility::Hidden));
        assert_eq!(app.world().resource::<InputFocus>().get(), None);
        assert!(app.world().get_entity(child).is_ok());
    }

    #[test]
    fn f6_enters_hud_without_stealing_the_worlds_map_binding() {
        use crate::presentation::{
            automap::{MapKey, OpenAutomap, spawn_map_key},
            notices::TextLog,
        };
        let mut app = crate::presentation::test_input::input_app();
        app.add_plugins((InputDispatchPlugin, TabNavigationPlugin, ButtonPlugin))
            .init_state::<crate::presentation::screen::Screen>()
            .init_resource::<ScreenLayout>()
            .init_resource::<SelectedMember>()
            .init_resource::<crate::game::combat::CombatSettings>()
            .init_resource::<TextLog>()
            .add_message::<crate::game::combat::StartRound>()
            .add_message::<crate::game::combat::GiveOrder>()
            .add_message::<OpenAutomap>()
            .add_input_context::<MapKey>()
            .add_systems(Startup, spawn_map_key)
            .add_plugins(keys::plugin);
        let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        let root =
            app.world_mut().spawn((MainScreen, TabGroup::modal())).observe(leave_hud_focus).id();
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
