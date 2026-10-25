//! The main game screen around the 3D view (`docs/research/main-game-screen.md`,
//! decomp `MainGameScreen.cpp` and `MGS*.cpp`): the roof, the party columns,
//! the bottom panels and the button row, in retail's three layout modes.
//! The 3D view renders into the viewport the layout leaves open.

use bevy::camera::Viewport;
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;

use std::collections::HashMap;

use super::notices::TextLog;
use super::screen_art::{absolute, sprite, text};
use super::sky::SkyCamera;
use crate::content::items::ItemCatalog;
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Race};
use crate::game::combat::attack::{Armour, armour};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::party::body::PartyCamera;
use crate::game::party::roster::PartyRoster;

const SCREEN: Vec2 = Vec2::new(640.0, 480.0);

/// Retail's layout modes (`g_settings.main_ui_mode`; the roof tooltips call
/// them Closed, Wide and Full).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    /// Full portrait columns either side of a narrow view.
    Closed,
    /// Bar strips at the sides; new games start here.
    #[default]
    Wide,
    /// Strips and no bottom panels.
    Full,
}

/// The bottom panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Radar,
    TextBox,
    Formation,
}

/// The screen's layout: its mode and which bottom panels each mode shows
/// (retail keeps a preference per mode, `ApplyMainGameModeFlag`).
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ScreenLayout {
    pub mode: LayoutMode,
    /// Closed shows the radar and formation always; the text box by choice.
    closed_text: bool,
    /// Wide's choices: radar, text box, formation.
    wide: [bool; 3],
}

impl Default for ScreenLayout {
    /// New games start Wide with all three panels.
    fn default() -> Self {
        ScreenLayout { mode: LayoutMode::Wide, closed_text: true, wide: [true; 3] }
    }
}

fn panel_index(panel: Panel) -> usize {
    match panel {
        Panel::Radar => 0,
        Panel::TextBox => 1,
        Panel::Formation => 2,
    }
}

impl ScreenLayout {
    pub fn shows(&self, panel: Panel) -> bool {
        match self.mode {
            LayoutMode::Closed => panel != Panel::TextBox || self.closed_text,
            LayoutMode::Wide => self.wide[panel_index(panel)],
            LayoutMode::Full => false,
        }
    }

    /// The 3D view's rectangle in screen pixels (retail `g_viewport_modes`,
    /// chosen by `GetMainGameViewportMode`): the narrow view in Closed,
    /// reaching down over hidden panels; the wide view over the strips.
    pub fn viewport(&self) -> Rect {
        let all = [Panel::Radar, Panel::TextBox, Panel::Formation].iter().all(|&p| self.shows(p));
        match self.mode {
            LayoutMode::Closed if self.shows(Panel::TextBox) => {
                Rect::new(128.0, 18.0, 512.0, 358.0)
            }
            LayoutMode::Closed => Rect::new(128.0, 18.0, 512.0, 450.0),
            _ if all => Rect::new(23.0, 18.0, 617.0, 358.0),
            _ => Rect::new(23.0, 18.0, 617.0, 450.0),
        }
    }

    /// Backspace: Closed, Wide, Full and round (Shift goes back).
    pub fn cycle(&mut self, back: bool) {
        use LayoutMode::*;
        self.mode = match (self.mode, back) {
            (Closed, false) => Wide,
            (Wide, false) => Full,
            (Full, false) => Closed,
            (Closed, true) => Full,
            (Wide, true) => Closed,
            (Full, true) => Wide,
        };
    }

    /// A layout arrow: Closed toggles only the text box; hiding Wide's last
    /// panel goes Full; showing one in Full goes Wide with only that one.
    pub fn toggle(&mut self, panel: Panel) {
        let i = panel_index(panel);
        match self.mode {
            LayoutMode::Closed if panel == Panel::TextBox => self.closed_text = !self.closed_text,
            LayoutMode::Closed => {}
            LayoutMode::Wide => {
                self.wide[i] = !self.wide[i];
                if self.wide == [false; 3] {
                    self.mode = LayoutMode::Full;
                }
            }
            LayoutMode::Full => {
                self.wide = [false; 3];
                self.wide[i] = true;
                self.mode = LayoutMode::Wide;
            }
        }
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
    formation_back: Handle<SpriteSheet>,
    roof_buttons: Handle<SpriteSheet>,
    layout_arrows: Handle<SpriteSheet>,
    scroll_arrows: Handle<SpriteSheet>,
    icons: Handle<SpriteSheet>,
    options: Handle<SpriteSheet>,
    radar_disc: Handle<SpriteSheet>,
    radar_rim: Handle<SpriteSheet>,
    view_cone: Handle<SpriteSheet>,
    combat_toggle: Handle<SpriteSheet>,
    column_info: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    small_font: Handle<SpriteSheet>,
}

pub fn load_main_screen_art(mut commands: Commands, assets: Res<AssetServer>) {
    let load = |file: &str| assets.load(format!("retail://data/main interface/{file}.sti"));
    commands.insert_resource(MainScreenArt {
        roof: load("main_roof"),
        columns: load("main_columns_normal"),
        bottom_bar: load("main_bottom_bar"),
        radar_back: load("main_radar_back"),
        textbox_back: load("main_textbox_back"),
        textbox_tabs: load("main_textbox_tabs"),
        formation_back: load("small_formation_back"),
        roof_buttons: load("main_roof_buttons"),
        layout_arrows: load("main_layout_arrows"),
        scroll_arrows: load("main_scroll_arrows"),
        icons: load("icons_standard"),
        options: load("options_disk"),
        radar_disc: assets.load("retail://data/radar compass/formation_dot_small.sti"),
        radar_rim: assets.load("retail://data/radar compass/compass_rim.sti"),
        view_cone: assets.load("retail://data/party formation/small_formation_cone.sti"),
        combat_toggle: load("icon_combat_toggle"),
        column_info: load("main_column_info_normal"),
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        small_font: assets.load("retail://data/fonts/smfnt.sti"),
    });
}

/// The window is cleared black first, the 3D views draw into the viewport,
/// and the interface draws over the whole window last.
pub fn spawn_screen_cameras(mut commands: Commands) {
    commands.spawn((
        Name::new("Clear camera"),
        Camera2d,
        Camera { order: -10, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
    ));
    commands.spawn((
        Name::new("Interface camera"),
        Camera2d,
        Camera { order: 10, clear_color: ClearColorConfig::None, ..default() },
        IsDefaultUiCamera,
    ));
}

/// Fit the 3D cameras to the layout's viewport, in window pixels, with the
/// 640×480 screen scaled and centred as the interface is.
pub fn fit_viewport(
    layout: Res<ScreenLayout>,
    windows: Query<&Window>,
    scale: Res<UiScale>,
    mut cameras: Query<&mut Camera, Or<(With<PartyCamera>, With<SkyCamera>)>>,
) {
    let Ok(window) = windows.single() else { return };
    let origin = (window.size() - SCREEN * scale.0) / 2.0;
    let rect = layout.viewport();
    let physical = |v: Vec2| ((origin + v * scale.0) * window.scale_factor()).round().as_uvec2();
    let viewport = Viewport {
        physical_position: physical(rect.min),
        physical_size: (physical(rect.max) - physical(rect.min)).max(UVec2::ONE),
        ..default()
    };
    for mut camera in &mut cameras {
        if camera.viewport.as_ref().is_none_or(|v| {
            v.physical_position != viewport.physical_position
                || v.physical_size != viewport.physical_size
        }) {
            camera.viewport = Some(viewport.clone());
        }
    }
}

/// Backspace cycles the layout (Shift+Backspace backwards).
pub fn cycle_layout(
    keys: Res<ButtonInput<KeyCode>>,
    automap: Option<Res<super::automap::Automap>>,
    inventory: Option<Res<super::inventory::InventoryScreen>>,
    mut layout: ResMut<ScreenLayout>,
) {
    if keys.just_pressed(KeyCode::Backspace) && automap.is_none() && inventory.is_none() {
        let back = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        layout.cycle(back);
    }
}

/// The member the buttons and keys act for (retail's selected character).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct SelectedMember(pub usize);

/// The screen's art, rebuilt when what it shows changes.
#[derive(Component)]
pub struct MainScreen;

#[derive(Component, Clone, Copy)]
enum ScreenButton {
    Mode(LayoutMode),
    Arrow(Panel),
    Portrait(usize),
}

fn press(
    click: On<PointerClick>,
    buttons: Query<&ScreenButton>,
    mut layout: ResMut<ScreenLayout>,
    mut selected: ResMut<SelectedMember>,
    mut commands: Commands,
    roster: Res<PartyRoster>,
    time: Res<Time>,
    mut last_click: Local<Option<(usize, f32)>>,
) {
    match buttons.get(click.entity) {
        Ok(ScreenButton::Mode(mode)) => layout.mode = *mode,
        Ok(ScreenButton::Arrow(panel)) => layout.toggle(*panel),
        Ok(ScreenButton::Portrait(slot)) => {
            selected.0 = *slot;
            // A double click opens the member's review screen.
            let now = time.elapsed_secs();
            if last_click.is_some_and(|(s, at)| s == *slot && now - at < DOUBLE_CLICK)
                && let Some(&member) = roster.0.get(*slot)
            {
                commands.insert_resource(super::inventory::InventoryScreen::new(member));
            }
            *last_click = Some((*slot, now));
        }
        Err(_) => {}
    }
}

const DOUBLE_CLICK: f32 = 0.4;

/// Keys 1-6 select a member; retail binds them to PC 1-6.
pub fn select_with_keys(
    keys: Res<ButtonInput<KeyCode>>,
    roster: Res<PartyRoster>,
    mut selected: ResMut<SelectedMember>,
) {
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ];
    if let Some(slot) = digits.iter().position(|&k| keys.just_pressed(k))
        && slot < roster.0.len()
    {
        selected.0 = slot;
    }
}

/// The radar's compass rim, turned with the party.
#[derive(Component)]
pub struct RadarRim;

/// Holds the radar's blips.
#[derive(Component)]
pub struct RadarBlips;

/// Retail `g_radar_blip_colors`: per class (neutral, hostile, friendly,
/// unseen, item, missile), dim, level and bright, for things more than 3 m
/// below, within 3 m, and above the party.
const BLIP_COLOURS: [[f32; 3]; 18] = [
    [0.5, 0.5, 0.0],
    [0.7, 0.7, 0.0],
    [1.0, 1.0, 0.0],
    [0.5, 0.0, 0.0],
    [0.7, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 0.5, 0.0],
    [0.0, 0.7, 0.0],
    [0.0, 1.0, 0.0],
    [0.2, 0.2, 0.2],
    [0.5, 0.5, 0.5],
    [0.7, 0.7, 0.7],
    [0.5, 0.5, 0.5],
    [0.7, 0.7, 0.7],
    [1.0, 1.0, 1.0],
    [0.0, 0.0, 0.5],
    [0.0, 0.0, 0.7],
    [0.0, 0.0, 1.0],
];

/// Where a blip `delta` away from the party goes on the radar, for a party
/// facing `yaw` (retail `PlaceRadarBlip`, zoomed out: 2 pixels at the
/// party's radius, growing to the 38-pixel rim at extreme range).
fn blip_position(delta: Vec3, yaw: f32) -> Vec2 {
    const SCALE: f32 = 2.0;
    const RIM: f32 = 38.0;
    let inner = 1.0;
    let outer = 50.0 + inner;
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let flat = Vec2::new(delta.dot(right), delta.dot(forward));
    let distance = flat.length();
    let pixels = if distance <= inner {
        SCALE
    } else {
        (SCALE + (distance - inner) * (RIM - SCALE) / (outer - inner)).min(RIM)
    };
    let unit = flat.normalize_or_zero();
    Vec2::new(75.0 + unit.x * pixels, 404.0 - unit.y * pixels)
}

/// Turn the compass rim with the party, and mark the monsters within the
/// far clip: hostile red, others yellow, brighter above and dimmer below.
pub fn update_radar(
    mut commands: Commands,
    party: Query<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>,
    camera: Query<&Projection, With<PartyCamera>>,
    mut rims: Query<&mut UiTransform, With<RadarRim>>,
    layers: Query<Entity, With<RadarBlips>>,
    monsters: Query<(&crate::game::world::population::Monster, &GlobalTransform)>,
    data: Option<Res<GameData>>,
    species: Res<Assets<crate::content::species::MonsterCatalog>>,
) {
    let (Ok((position, body)), Ok(layer)) = (party.single(), layers.single()) else { return };
    for mut rim in &mut rims {
        // North stays put while the view turns.
        rim.rotation = Rot2::radians(body.yaw);
    }
    let far = match camera.single() {
        Ok(Projection::Perspective(p)) => p.far,
        _ => 0.0,
    };
    let catalog = data.and_then(|d| species.get(&d.species));
    commands.entity(layer).despawn_related::<Children>();
    for (monster, at) in &monsters {
        let delta = at.translation() - position.0;
        if monster.is_dead() || delta.length() > far {
            continue;
        }
        let hostile =
            catalog.and_then(|c| c.0.get(monster.species as usize)).is_some_and(|s| !s.npc);
        let ring = if delta.y.abs() < 3.0 {
            1
        } else if delta.y > 0.0 {
            2
        } else {
            0
        };
        let [r, g, b] = BLIP_COLOURS[usize::from(hostile) * 3 + ring];
        commands.spawn((
            absolute(blip_position(delta, body.yaw) - Vec2::ONE, Vec2::splat(2.0)),
            BackgroundColor(Color::srgb(r, g, b)),
            Pickable::IGNORE,
            ChildOf(layer),
        ));
    }
}

/// A working bank button.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Bank {
    Camp,
    Formation,
}

fn press_bank(
    click: On<PointerClick>,
    buttons: Query<&Bank>,
    mut camp: MessageWriter<crate::game::camp::CampRequest>,
    mut layout: ResMut<ScreenLayout>,
) {
    match buttons.get(click.entity) {
        Ok(Bank::Camp) => {
            camp.write(crate::game::camp::CampRequest);
        }
        // The Party Formation button opens and closes the formation panel.
        Ok(Bank::Formation) => layout.toggle(Panel::Formation),
        Err(_) => {}
    }
}

/// A text box channel tab.
#[derive(Component, Clone, Copy)]
struct Channel(usize);

fn select_channel(click: On<PointerClick>, tabs: Query<&Channel>, mut log: ResMut<TextLog>) {
    if let Ok(tab) = tabs.get(click.entity) {
        log.selected = tab.0;
    }
}

/// The bottom row's previous and next character buttons.
#[derive(Component, Clone, Copy)]
struct Step(i32);

fn step_selection(
    click: On<PointerClick>,
    steps: Query<&Step>,
    roster: Res<PartyRoster>,
    mut selected: ResMut<SelectedMember>,
) {
    if let (Ok(step), n @ 1..) = (steps.get(click.entity), roster.0.len() as i32) {
        selected.0 = (selected.0 as i32 + step.0).rem_euclid(n) as usize;
    }
}

/// What one party slot shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotView {
    portrait: String,
    race: Race,
    dead: bool,
    hp_fill: u32,
    stamina_fill: u32,
    name: String,
    armour: i32,
    /// The main and off hand's item pictures, or the race's empty hand.
    hands: [Option<String>; 2],
    two_handed: bool,
}

/// What the whole screen shows; it is drawn again when this changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    layout: ScreenLayout,
    slots: Vec<Option<SlotView>>,
    selected: usize,
    /// The text box's channel and the lines it shows.
    text: (usize, Vec<String>),
    fighting: bool,
}

/// A bar's fill, 0 to 45 pixels, at least 1 while any is left (retail
/// `SyncPartyPortraitVitalsBars`).
fn fill(now: u32, max: u32) -> u32 {
    if now == 0 || max == 0 { 0 } else { (now * 45 / max).max(1) }
}

/// The dead's medium skull by race (retail `g_dead_portrait_catalog_ids`).
fn skull(race: Race) -> &'static str {
    match race {
        Race::Lizardman => "mskull_liz",
        Race::Dracon => "mskull_drac",
        Race::Felpurr => "mskull_felp",
        Race::Rawulf => "mskull_raw",
        Race::Mook => "mskull_mook",
        Race::Trynnie => "mskull_tryn",
        Race::TRang => "mskull_trang",
        Race::Umpani => "mskull_ump",
        Race::Rapax => "mskull_rap",
        Race::Android => "mskull_and",
        _ => "mskull",
    }
}

/// A race's empty hands, upper then lower (retail
/// `g_empty_hand_catalog_ids`).
fn empty_hands(race: Race) -> [&'static str; 2] {
    match race {
        Race::Faerie => ["fhandright", "fhandleft"],
        Race::Lizardman | Race::Dracon | Race::TRang | Race::Rapax => ["clawright", "clawleft"],
        Race::Felpurr | Race::Rawulf | Race::Mook | Race::Trynnie => ["pawright", "pawleft"],
        Race::Android => ["rhandright", "rhandleft"],
        _ => ["handright", "handleft"],
    }
}

/// The displayed armour class: the locations' values weighted 15/40/30/10/5
/// (head, body, legs, hands, feet), rounded (`characters.md` §4.8).
fn average_armour(armour: &Armour) -> i32 {
    let weights = [15, 40, 30, 10, 5];
    let total: i32 = armour.by_location.iter().zip(weights).map(|(a, w)| a * w).sum();
    (total as f32 / 100.0).round() as i32
}

#[allow(clippy::too_many_arguments)]
pub fn draw_main_screen(
    mut commands: Commands,
    layout: Res<ScreenLayout>,
    selected: Res<SelectedMember>,
    art: Res<MainScreenArt>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    roster: Res<PartyRoster>,
    members: Query<(&Character, Option<&Carried>)>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    screens: Query<Entity, With<MainScreen>>,
    log: Res<TextLog>,
    battle: Option<Res<crate::game::combat::Battle>>,
    mut drawn: Local<Option<Shown>>,
    mut loading: Local<HashMap<String, Handle<SpriteSheet>>>,
) {
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    let slots: Vec<Option<SlotView>> = (0..8)
        .map(|slot| {
            let (character, carried) = members.get(*roster.0.get(slot)?).ok()?;
            let item = |equip| {
                let def = carried?.worn(equip).and_then(|i| catalog?.get(i.item))?;
                Some((def.image.clone(), def.two_handed))
            };
            let (main, off) = (item(EquipSlot::Primary), item(EquipSlot::Secondary));
            let armour = match (carried, catalog) {
                (Some(carried), Some(catalog)) => {
                    average_armour(&armour(character, carried, catalog))
                }
                _ => 0,
            };
            Some(SlotView {
                portrait: character.portrait.clone(),
                race: character.race,
                dead: character.is_dead(),
                hp_fill: fill(character.hp, character.max_hp()),
                stamina_fill: fill(character.stamina, character.max_stamina()),
                name: character.name.clone(),
                armour,
                two_handed: main.as_ref().is_some_and(|m| m.1),
                hands: [main.map(|m| m.0), off.map(|o| o.0)],
            })
        })
        .collect();
    let shown = Shown {
        layout: layout.clone(),
        slots,
        selected: selected.0,
        text: (log.selected, log.visible().to_vec()),
        fighting: battle.is_some(),
    };
    if drawn.as_ref() == Some(&shown) && !screens.is_empty() {
        return;
    }
    let get = |h: &Handle<SpriteSheet>| sheets.get(h);
    let (
        Some(roof),
        Some(columns),
        Some(info),
        Some(bar),
        Some(radar),
        Some(textbox),
        Some(tabs),
        Some(formation),
        Some(roof_buttons),
        Some(arrows),
        Some(scroll_arrows),
        Some(icons),
        Some(options),
        Some(radar_disc),
        Some(radar_rim),
        Some(view_cone),
        Some(combat_toggle),
        Some(font),
        Some(small_font),
    ) = (
        get(&art.roof),
        get(&art.columns),
        get(&art.column_info),
        get(&art.bottom_bar),
        get(&art.radar_back),
        get(&art.textbox_back),
        get(&art.textbox_tabs),
        get(&art.formation_back),
        get(&art.roof_buttons),
        get(&art.layout_arrows),
        get(&art.scroll_arrows),
        get(&art.icons),
        get(&art.options),
        get(&art.radar_disc),
        get(&art.radar_rim),
        get(&art.view_cone),
        get(&art.combat_toggle),
        get(&art.font),
        get(&art.small_font),
    )
    else {
        return;
    };
    // Sheets loaded here are kept, or dropping the handle would cancel them.
    let mut complete = true;
    let mut sheet = |path: String| {
        let handle =
            loading.entry(path).or_insert_with_key(|p| assets.load(format!("retail://{p}")));
        let loaded = sheets.get(&*handle);
        complete &= loaded.is_some();
        loaded
    };
    for screen in &screens {
        commands.entity(screen).despawn();
    }
    let root = commands
        .spawn((
            MainScreen,
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
    let put = |commands: &mut Commands, part: Option<(ImageNode, Node, Pickable)>| {
        if let Some(part) = part {
            commands.spawn((part, ChildOf(frame)));
        }
    };
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
    let row_y = |slot: usize| 18.0 + 85.0 * (slot / 2) as f32;
    let menu_x = |slot: usize| if slot.is_multiple_of(2) { 0.0 } else { 512.0 };
    // Portraits sit under the column art, which frames them.
    if closed {
        for (slot, view) in shown.slots.iter().enumerate() {
            let at = Vec2::new(menu_x(slot) + 20.0, row_y(slot));
            match view {
                None => {
                    put(
                        &mut commands,
                        sheet("data/main interface/pback.sti".into())
                            .and_then(|s| sprite(s, 0, at)),
                    );
                    put(
                        &mut commands,
                        sheet("data/main interface/dragons.sti".into())
                            .and_then(|s| sprite(s, 0, at + Vec2::X * 3.0)),
                    );
                }
                Some(view) if view.dead => {
                    let path = format!("data/portraits/medium/{}.sti", skull(view.race));
                    if let Some((mut image, node, pick)) =
                        sheet(path).and_then(|s| sprite(s, 0, at))
                    {
                        // Mirrored on the right.
                        image.flip_x = slot % 2 == 1;
                        commands.spawn(((image, node, pick), ChildOf(frame)));
                    }
                }
                Some(view) => {
                    let path = format!("data/portraits/medium/m{}.sti", view.portrait);
                    put(&mut commands, sheet(path).and_then(|s| sprite(s, 0, at)));
                }
            }
        }
    }
    let mut parts = vec![sprite(roof, 0, Vec2::ZERO)];
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
        parts.push(sprite(radar, 0, Vec2::new(23.0, 358.0)));
    }
    if layout.shows(Panel::TextBox) {
        parts.push(sprite(textbox, 0, Vec2::new(128.0, 358.0)));
        parts.push(sprite(tabs, shown.text.0, Vec2::new(138.0, 360.0)));
    }
    if layout.shows(Panel::Formation) {
        parts.push(sprite(formation, 0, Vec2::new(512.0, 358.0)));
    }
    // The button row's sockets.
    for (frame, x) in
        [(2, 0.0), (4, 30.0), (8, 196.0), (5, 292.0), (6, 343.0), (7, 528.0), (3, 610.0)]
    {
        parts.push(sprite(bar, frame, Vec2::new(x, 450.0)));
    }
    for part in parts {
        put(&mut commands, part);
    }
    // Each member's hands, bars, name and armour (retail
    // `RedrawPartyPortraitOverlay`); Wide and Full show only the bars.
    for (slot, view) in shown.slots.iter().enumerate() {
        let Some(view) = view else { continue };
        let (y, left) = (row_y(slot), slot % 2 == 0);
        let bars_x = match (closed, left) {
            (true, true) => 105.0,
            (true, false) => 512.0,
            (false, true) => 0.0,
            (false, false) => 617.0,
        };
        if closed {
            let hands_x = if left { 2.0 } else { 619.0 };
            put(
                &mut commands,
                sprite(info, if view.two_handed { 2 } else { 0 }, Vec2::new(hands_x, y)),
            );
            if !view.dead {
                let [upper, lower] = empty_hands(view.race);
                for (hand, (dy, empty)) in view.hands.iter().zip([(23.0, upper), (47.0, lower)]) {
                    if dy == 47.0 && view.two_handed {
                        continue;
                    }
                    let part = match hand {
                        Some(path) => sheet(path.clone())
                            .and_then(|s| sprite(s, 2, Vec2::new(hands_x + 3.0, y + dy))),
                        None => sheet(format!("data/main interface/{empty}.sti"))
                            .and_then(|s| sprite(s, 0, Vec2::new(hands_x + 4.0, y + dy))),
                    };
                    put(&mut commands, part);
                }
                put(
                    &mut commands,
                    Some(text(
                        small_font,
                        &mut images,
                        &view.armour.to_string(),
                        Vec2::new(hands_x + 3.0, y + 3.0),
                        13.0,
                    )),
                );
            }
            put(
                &mut commands,
                Some(text(
                    font,
                    &mut images,
                    &view.name,
                    Vec2::new(menu_x(slot) + 21.0, y + 73.0),
                    84.0,
                )),
            );
        }
        put(&mut commands, sprite(info, 1, Vec2::new(bars_x, y)));
        if !view.dead {
            // The full bar, then its empty part blacked out from the top.
            for (file, dx, fill) in
                [("healthbar", 6.0, view.hp_fill), ("staminabar", 10.0, view.stamina_fill)]
            {
                let at = Vec2::new(bars_x + dx, y + 23.0);
                put(
                    &mut commands,
                    sheet(format!("data/main interface/{file}.sti")).and_then(|s| sprite(s, 0, at)),
                );
                commands.spawn((
                    absolute(at, Vec2::new(3.0, (45 - fill) as f32)),
                    BackgroundColor(Color::srgb_u8(1, 1, 1)),
                    Pickable::IGNORE,
                    ChildOf(frame),
                ));
            }
        }
        // The selected member's rim.
        if slot == shown.selected {
            let part = if closed {
                sheet("data/main interface/charselect.sti".into())
                    .and_then(|s| sprite(s, 0, Vec2::new(menu_x(slot) + 23.0, y)))
            } else {
                sheet("data/main interface/edgeselect.sti".into())
                    .and_then(|s| sprite(s, 0, Vec2::new(bars_x + 1.0, y + 2.0)))
            };
            put(&mut commands, part);
        }
        // The portrait (or strip) selects; a double click reviews.
        let (at, size) = if closed {
            (Vec2::new(menu_x(slot) + 23.0, y), Vec2::new(82.0, 72.0))
        } else {
            (Vec2::new(if left { 0.0 } else { 617.0 }, y), Vec2::new(22.0, 85.0))
        };
        commands
            .spawn((absolute(at, size), ScreenButton::Portrait(slot), ChildOf(frame)))
            .observe(press);
    }
    // Roof buttons: the current mode is pressed.
    for (i, (mode, x)) in
        [(LayoutMode::Closed, 10.0), (LayoutMode::Wide, 39.0), (LayoutMode::Full, 68.0)]
            .into_iter()
            .enumerate()
    {
        let image = if mode == layout.mode { 3 + i } else { i };
        if let Some((node_image, node, _)) = sprite(roof_buttons, image, Vec2::new(x, 1.0)) {
            commands
                .spawn((node_image, node, ScreenButton::Mode(mode), ChildOf(frame)))
                .observe(press);
        }
    }
    // Layout arrows on the left post: hide while shown, show while hidden;
    // Closed keeps the radar and formation (grayed).
    for (panel, y, hide, show, grayed) in [
        (Panel::Radar, 371.0, 0, 3, 6),
        (Panel::TextBox, 394.0, 8, 11, 14),
        (Panel::Formation, 417.0, 16, 19, 22),
    ] {
        let locked = closed && panel != Panel::TextBox;
        let image = match (locked, layout.shows(panel)) {
            (true, _) => grayed,
            (false, true) => hide,
            (false, false) => show,
        };
        if let Some((node_image, node, _)) = sprite(arrows, image, Vec2::new(0.0, y)) {
            let mut arrow = commands.spawn((node_image, node, ChildOf(frame)));
            if !locked {
                arrow.insert(ScreenButton::Arrow(panel)).observe(press);
            }
        }
    }
    // The text box: its lines centred in the body, 11 pixels apart, and its
    // channel tabs (retail `RedrawTextBoxBody`, `TextBoxChannelTabRegionEvent`).
    if layout.shows(Panel::TextBox) {
        for (i, line) in shown.text.1.iter().enumerate() {
            let at = Vec2::new(166.0, 362.0 + 11.0 * i as f32);
            put(&mut commands, Some(text(font, &mut images, line, at, 290.0)));
        }
        for (channel, y) in [(0, 362.0), (1, 383.0), (2, 404.0)] {
            commands
                .spawn((
                    absolute(Vec2::new(140.0, y), Vec2::new(24.0, 16.0)),
                    Channel(channel),
                    ChildOf(frame),
                ))
                .observe(select_channel);
        }
    }
    // The radar: its disc and view cone, the blips, and the compass rim on
    // top, 91 pixels square from (30, 359) (retail `MGSRadarMap.cpp`).
    if layout.shows(Panel::Radar) {
        let at = Vec2::new(30.0, 359.0);
        put(&mut commands, sprite(radar_disc, 0, at));
        put(&mut commands, sprite(view_cone, 0, at));
        commands.spawn((
            RadarBlips,
            absolute(Vec2::ZERO, Vec2::ZERO),
            Pickable::IGNORE,
            ChildOf(frame),
        ));
        if let Some(rim) = sprite(radar_rim, 0, at) {
            commands.spawn((rim, RadarRim, UiTransform::default(), ChildOf(frame)));
        }
    }
    // The button bank (retail `pc.cpp` positions and frames: grayed, off,
    // ..., on). Camp and the formation panel work; the journal, search,
    // items, spells and options are grayed until they exist.
    let fighting = shown.fighting;
    let bank: [(&SpriteSheet, f32, f32, usize, usize, Option<Bank>); 8] = [
        (options, 0.0, 450.0, 3, 0, None),
        (icons, 40.0, 452.0, 0x2b, 0x28, None),
        (icons, 70.0, 452.0, 3, 0, Some(Bank::Camp)),
        (icons, 100.0, 452.0, 8, 5, None),
        (icons, 130.0, 452.0, 0x17, 0x14, None),
        (icons, 160.0, 452.0, 0x1c, 0x19, None),
        (combat_toggle, 199.0, 452.0, 3, 0, None),
        (
            icons,
            571.0,
            452.0,
            0x26,
            if layout.shows(Panel::Formation) { 0x25 } else { 0x23 },
            Some(Bank::Formation),
        ),
    ];
    for (sheet, x, y, grayed, normal, action) in bank {
        // The journal, camp and the combat toggle are grayed in a fight.
        let usable = action.is_some() && !(fighting && action == Some(Bank::Camp));
        let image = if usable { normal } else { grayed };
        if let Some((node_image, node, _)) = sprite(sheet, image, Vec2::new(x, y)) {
            let mut button = commands.spawn((node_image, node, ChildOf(frame)));
            if let (true, Some(action)) = (usable, action) {
                button.insert(action).observe(press_bank);
            }
        }
    }
    // Previous and next character (retail `main_scroll_arrows`).
    for (image, x, step) in [(0, 300.0, -1), (4, 323.0, 1)] {
        if let Some((node_image, node, _)) = sprite(scroll_arrows, image, Vec2::new(x, 456.0)) {
            commands.spawn((node_image, node, Step(step), ChildOf(frame))).observe(step_selection);
        }
    }
    // The selected member's caption: "Name - Profession" (retail
    // `DrawSubMenuCharacterAction`).
    if let Some(Some(view)) = shown.slots.get(shown.selected)
        && let Some(Ok((character, _))) = roster.0.get(shown.selected).map(|&m| members.get(m))
    {
        let caption = format!("{} - {:?}", view.name, character.profession);
        put(
            &mut commands,
            Some(text(small_font, &mut images, &caption, Vec2::new(343.0, 454.0), 185.0)),
        );
    }
    *drawn = complete.then_some(shown);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blips_sit_ahead_and_beyond_on_the_rim() {
        // Ahead of a party facing -Z, 10 m off: straight up from the centre.
        let ahead = blip_position(Vec3::new(0.0, 0.0, -10.0), 0.0);
        assert_eq!(ahead.x, 75.0);
        assert!(ahead.y < 404.0 && ahead.y > 404.0 - 38.0);
        // Far beyond extreme range: on the 38-pixel rim, to the right.
        assert_eq!(blip_position(Vec3::new(200.0, 0.0, 0.0), 0.0), Vec2::new(113.0, 404.0));
    }

    #[test]
    fn the_viewport_follows_the_mode_and_panels() {
        let mut layout = ScreenLayout::default();
        assert_eq!(layout.viewport(), Rect::new(23.0, 18.0, 617.0, 358.0));
        layout.toggle(Panel::Radar);
        assert_eq!(layout.viewport(), Rect::new(23.0, 18.0, 617.0, 450.0));
        layout.mode = LayoutMode::Closed;
        assert_eq!(layout.viewport(), Rect::new(128.0, 18.0, 512.0, 358.0));
        layout.toggle(Panel::Radar);
        assert!(layout.shows(Panel::Radar), "Closed keeps the radar");
        layout.toggle(Panel::TextBox);
        assert_eq!(layout.viewport(), Rect::new(128.0, 18.0, 512.0, 450.0));
    }

    #[test]
    fn hiding_the_last_panel_goes_full_and_showing_one_goes_wide() {
        let mut layout = ScreenLayout::default();
        layout.toggle(Panel::Radar);
        layout.toggle(Panel::TextBox);
        layout.toggle(Panel::Formation);
        assert_eq!(layout.mode, LayoutMode::Full);
        layout.toggle(Panel::TextBox);
        assert_eq!(layout.mode, LayoutMode::Wide);
        assert!(layout.shows(Panel::TextBox) && !layout.shows(Panel::Radar));
        layout.cycle(false);
        assert_eq!(layout.mode, LayoutMode::Full);
        layout.cycle(false);
        assert_eq!(layout.mode, LayoutMode::Closed);
    }
}
