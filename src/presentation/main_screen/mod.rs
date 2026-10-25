//! The main game screen around the 3D view (`docs/research/main-game-screen.md`,
//! decomp `MainGameScreen.cpp` and `MGS*.cpp`): the roof, the party columns,
//! the bottom panels and the button row, in retail's three layout modes.
//! The 3D view renders into the viewport the layout leaves open.
//!
//! The frame holds what the layout alone decides and is built again when
//! the layout changes. Each part showing game state (the party's slots, the
//! text box, the radar, the formation board, the button bank) has its own
//! node in the frame and redraws only that, when what it shows changes.

mod bank;
mod formation;
mod layout;
mod party;
mod radar;
mod text_box;

use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use super::screen_art::{absolute, sprite};
use crate::content::ui::SpriteSheet;
pub use layout::{LayoutMode, Panel, ScreenLayout};

const SCREEN: Vec2 = Vec2::new(640.0, 480.0);

pub struct MainScreenPlugin;

impl Plugin for MainScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScreenLayout>()
            .init_resource::<SelectedMember>()
            .add_systems(Startup, (load_main_screen_art, layout::spawn_screen_cameras))
            .add_systems(
                Update,
                (
                    layout::cycle_layout,
                    party::select_with_keys,
                    bank::combat_keys,
                    text_box::text_box_keys,
                    paint_hover_art,
                    layout::fit_viewport,
                    build_frame,
                    (
                        party::draw_party,
                        party::play_splats,
                        text_box::draw_text_box,
                        radar::update_radar,
                        formation::draw_formation,
                        bank::draw_bank,
                        bank::draw_caption,
                        bank::show_round_progress,
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
pub struct SelectedMember(pub usize);

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

/// Spawn a sheet's frame under `parent`, if the sheet has that frame.
fn put<'a>(
    commands: &'a mut Commands,
    parent: Entity,
    part: Option<(ImageNode, Node, Pickable)>,
) -> Option<EntityCommands<'a>> {
    part.map(|part| commands.spawn((part, ChildOf(parent))))
}

/// An empty node under `parent` whose children are placed in screen pixels.
fn layer(commands: &mut Commands, parent: Entity, part: impl Bundle) -> Entity {
    commands.spawn((part, absolute(Vec2::ZERO, Vec2::ZERO), Pickable::IGNORE, ChildOf(parent))).id()
}

/// The screen's root, with the layout its frame was built for.
#[derive(Component)]
pub struct MainScreen(ScreenLayout);

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
    let root = commands
        .spawn((
            MainScreen(layout.clone()),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(-5),
            Pickable::IGNORE,
        ))
        .id();
    let frame = commands
        .spawn((
            Node { width: Val::Px(SCREEN.x), height: Val::Px(SCREEN.y), ..default() },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    // Black wherever the 3D view is not: outside the screen (the sky
    // camera's clear covers the whole window) and behind the art.
    let view = layout.viewport();
    const FAR: f32 = 10_000.0;
    for (at, size) in [
        (Vec2::new(-FAR, -FAR), Vec2::new(FAR, SCREEN.y + 2.0 * FAR)),
        (Vec2::new(SCREEN.x, -FAR), Vec2::new(FAR, SCREEN.y + 2.0 * FAR)),
        (Vec2::new(0.0, -FAR), Vec2::new(SCREEN.x, FAR + view.min.y)),
        (Vec2::new(0.0, view.max.y), Vec2::new(SCREEN.x, FAR)),
        (Vec2::new(0.0, view.min.y), Vec2::new(view.min.x, view.height())),
        (Vec2::new(view.max.x, view.min.y), Vec2::new(SCREEN.x - view.max.x, view.height())),
    ] {
        commands.spawn((
            absolute(at, size),
            BackgroundColor(Color::BLACK),
            Pickable::IGNORE,
            ChildOf(frame),
        ));
    }
    let closed = layout.mode == LayoutMode::Closed;
    // Portraits sit under the column art, which frames them.
    if closed {
        party::spawn_portraits(&mut commands, frame);
    }
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
    if layout.shows(Panel::Radar) {
        parts.push(sprite(sheet(&art.radar_back), 0, Vec2::new(23.0, 358.0)));
    }
    if layout.shows(Panel::TextBox) {
        parts.push(sprite(sheet(&art.textbox_back), 0, Vec2::new(128.0, 358.0)));
    }
    if layout.shows(Panel::Formation) {
        parts.push(sprite(sheet(&art.formation_back), 0, Vec2::new(512.0, 358.0)));
    }
    // The button row's sockets.
    for (frame, x) in
        [(2, 0.0), (4, 30.0), (8, 196.0), (5, 292.0), (6, 343.0), (7, 528.0), (3, 610.0)]
    {
        parts.push(sprite(bar, frame, Vec2::new(x, 450.0)));
    }
    for part in parts {
        put(&mut commands, frame, part);
    }
    party::spawn_slots(&mut commands, frame, closed);
    layout::spawn_buttons(&mut commands, frame, &layout, &art, &sheets);
    if layout.shows(Panel::TextBox) {
        text_box::spawn(&mut commands, frame);
    }
    if layout.shows(Panel::Radar) {
        radar::spawn(&mut commands, frame, &art, &sheets);
    }
    if layout.shows(Panel::Formation) {
        formation::spawn(&mut commands, frame, &art, &sheets);
    }
    bank::spawn(&mut commands, frame, &art, &sheets);
}

/// A button's look while hovered (retail's buttons have a frame for it).
#[derive(Component, Clone)]
#[require(Hovered)]
pub struct HoverArt {
    normal: Handle<Image>,
    hovered: Handle<Image>,
}

fn hover_art(sheet: &SpriteSheet, normal: usize, hovered: usize) -> HoverArt {
    let image = |frame: usize| sheet.images.get(frame).cloned().unwrap_or_default();
    HoverArt { normal: image(normal), hovered: image(hovered) }
}

/// Show each button's hover frame while the pointer is over it.
fn paint_hover_art(mut buttons: Query<(&Hovered, &HoverArt, &mut ImageNode), Changed<Hovered>>) {
    for (hovered, art, mut image) in &mut buttons {
        image.image = if hovered.get() { art.hovered.clone() } else { art.normal.clone() };
    }
}
