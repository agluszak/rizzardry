//! The main game screen around the 3D view (`docs/research/main-game-screen.md`,
//! decomp `MainGameScreen.cpp` and `MGS*.cpp`): the roof, the party columns,
//! the bottom panels and the button row, in retail's three layout modes.
//! The 3D view renders into the viewport the layout leaves open.

use bevy::camera::Viewport;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;

use std::collections::{HashMap, HashSet};

use super::notices::TextLog;
use super::screen_art::{absolute, sprite, text};
use super::sky::SkyCamera;
use crate::content::items::ItemCatalog;
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Race};
use crate::game::combat::attack::{Armour, armour};
use crate::game::combat::{CombatSettings, Order, PartyOrders, StartRound};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot, Given, ItemNews, PartyInventory};
use crate::game::party::body::PartyCamera;
use crate::game::party::formation::Formation;
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
        text_scroll: load("main_scroll"),
        formation_back: load("small_formation_back"),
        roof_buttons: load("main_roof_buttons"),
        layout_arrows: load("main_layout_arrows"),
        scroll_arrows: load("main_scroll_arrows"),
        icons: load("icons_standard"),
        options: load("options_disk"),
        radar_disc: assets.load("retail://data/radar compass/formation_dot_small.sti"),
        formation_cage: assets.load("retail://data/party formation/small_formation_cage.sti"),
        formation_symbols: assets.load("retail://data/party formation/small_formation_symbol.sti"),
        formation_jewels: assets.load("retail://data/party formation/small_formation_jewels.sti"),
        radar_rim: assets.load("retail://data/radar compass/compass_rim.sti"),
        view_cone: assets.load("retail://data/party formation/small_formation_cone.sti"),
        combat_toggle: load("icon_combat_toggle"),
        attack_confirm: load("attack_confirm"),
        cont_toggle: load("cont_toggle"),
        combat_stop: load("combat_stop"),
        cont_start: load("cont_start"),
        progress: load("progressbar"),
        column_info: load("main_column_info_normal"),
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        small_font: assets.load("retail://data/fonts/smfnt.sti"),
        bold_font: assets.load("retail://data/fonts/wiz_text_font_bold.sti"),
        damage_splat: load("damage_splat_anim"),
        damage_splat2: load("damage_splat_anim2"),
        death_splat: load("death_splat_anim"),
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

/// Fit the 3D cameras to the layout's viewport, in the pixels of what the
/// interface draws into (the window, or an offscreen image), with the
/// 640×480 screen scaled and centred as the interface is.
pub fn fit_viewport(
    layout: Res<ScreenLayout>,
    interface: Query<&Camera, With<IsDefaultUiCamera>>,
    scale: Res<UiScale>,
    mut cameras: Query<
        &mut Camera,
        (Or<(With<PartyCamera>, With<SkyCamera>)>, Without<IsDefaultUiCamera>),
    >,
) {
    let Some((size, factor)) = interface
        .single()
        .ok()
        .and_then(|c| Some((c.logical_target_size()?, c.target_scaling_factor()?)))
    else {
        return;
    };
    let origin = (size - SCREEN * scale.0) / 2.0;
    let rect = layout.viewport();
    let physical = |v: Vec2| ((origin + v * scale.0) * factor).round().as_uvec2();
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

#[allow(clippy::too_many_arguments)]
fn press(
    click: On<PointerClick>,
    buttons: Query<&ScreenButton>,
    mut layout: ResMut<ScreenLayout>,
    mut selected: ResMut<SelectedMember>,
    mut commands: Commands,
    roster: Res<PartyRoster>,
    time: Res<Time>,
    mut last_click: Local<Option<(usize, f32)>>,
    (mut party, mut members, data, catalogs, battle, mut news): (
        ResMut<PartyInventory>,
        Query<(&Character, &mut Carried)>,
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        Option<Res<crate::game::combat::Battle>>,
        MessageWriter<ItemNews>,
    ),
    mut screen: ResMut<crate::game::screen::Screen>,
) {
    match buttons.get(click.entity) {
        Ok(ScreenButton::Mode(mode)) => layout.mode = *mode,
        Ok(ScreenButton::Arrow(panel)) => layout.toggle(*panel),
        // With an item in hand, a click gives it to the member.
        Ok(ScreenButton::Portrait(slot)) if party.held.is_some() => {
            let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
            let Some(Ok((character, mut carried))) =
                roster.0.get(*slot).map(|&m| members.get_mut(m))
            else {
                return;
            };
            let item = party
                .held
                .as_ref()
                .and_then(|h| catalog.get(h.item.item).map(|d| h.item.display_name(d, false)));
            let item = item.unwrap_or_default();
            let eligible = crate::game::camp::can_act(character);
            let member = eligible.then_some(&mut *carried);
            match party.give_held(member, !eligible || battle.is_some(), catalog) {
                Some(Given::Member) => {
                    news.write(ItemNews::Takes { member: Some(character.name.clone()), item });
                }
                Some(Given::Pool) => {
                    news.write(ItemNews::Takes { member: None, item });
                }
                Some(Given::NoRoom) => {
                    news.write(ItemNews::NoRoom);
                }
                None => {}
            }
        }
        Ok(ScreenButton::Portrait(slot)) => {
            selected.0 = *slot;
            // A double click opens the member's review screen.
            let now = time.elapsed_secs();
            if last_click.is_some_and(|(s, at)| s == *slot && now - at < DOUBLE_CLICK)
                && let Some(&member) = roster.0.get(*slot)
            {
                super::inventory::open(&mut commands, &mut screen, member);
            }
            *last_click = Some((*slot, now));
        }
        Err(_) => {}
    }
}

const DOUBLE_CLICK: f32 = 0.4;

/// Combat keys from retail's `mgskeyboard.ini`: Enter starts the round,
/// backslash switches continuous combat, A and D set the selected member to
/// attack or defend.
pub fn combat_keys(
    keys: Res<ButtonInput<KeyCode>>,
    battle: Option<Res<crate::game::combat::Battle>>,
    selected: Res<SelectedMember>,
    mut orders: ResMut<PartyOrders>,
    mut settings: ResMut<CombatSettings>,
    mut round: MessageWriter<StartRound>,
) {
    if keys.just_pressed(KeyCode::Backslash) {
        settings.continuous = !settings.continuous;
    }
    if battle.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        round.write(StartRound);
    }
    if keys.just_pressed(KeyCode::KeyA) {
        orders.set(selected.0, Order::Attack);
    }
    if keys.just_pressed(KeyCode::KeyD) {
        orders.set(selected.0, Order::Defend);
    }
}

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

/// Each formation cell's marker on the board, by row (front, right, rear,
/// left, center) and seat (retail `g_formation_marker_offsets`).
const FORMATION_CELLS: [(f32, f32); 15] = [
    (38.0, 9.0),
    (25.0, 13.0),
    (51.0, 13.0),
    (67.0, 38.0),
    (63.0, 25.0),
    (63.0, 51.0),
    (38.0, 67.0),
    (51.0, 63.0),
    (25.0, 63.0),
    (9.0, 38.0),
    (13.0, 51.0),
    (13.0, 25.0),
    (38.0, 30.0),
    (30.0, 42.0),
    (46.0, 42.0),
];

/// Holds the portraits' damage splats.
#[derive(Component)]
pub struct Splats;

/// A splat playing over a member's portrait.
#[derive(Debug, Clone)]
struct Splat {
    slot: usize,
    started: f32,
    damage: u32,
    death: bool,
    /// Which of retail's two damage splats (`damage_splat_anim` or `_anim2`).
    second: bool,
}

/// Splats being played, and each member's health last seen.
#[derive(Default)]
pub struct SplatState {
    playing: Vec<Splat>,
    health: Vec<u32>,
    noise: u32,
}

/// Each frame of a splat lasts this long (retail: 100 ms).
const SPLAT_FRAME: f32 = 0.1;

/// Play a splat over a member's portrait when their health drops (retail
/// `RecordCharacterDamage`): one of two damage splats for 8 frames with the
/// damage for the first 6, or the 30-frame death splat when the blow kills.
/// Retail picks between the two splats with its gameplay randomness; here
/// presentation's own does.
pub fn play_splats(
    mut commands: Commands,
    time: Res<Time>,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    roster: Res<PartyRoster>,
    members: Query<&Character>,
    layers: Query<Entity, With<Splats>>,
    mut state: Local<SplatState>,
) {
    let now = time.elapsed_secs();
    let health: Vec<u32> = roster.0.iter().map(|&m| members.get(m).map_or(0, |c| c.hp)).collect();
    if state.health.len() == health.len() {
        for (slot, (&before, &after)) in state.health.clone().iter().zip(&health).enumerate() {
            if after < before {
                state.noise = state.noise.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let second = state.noise & 0x10000 != 0;
                state.playing.retain(|s| s.slot != slot);
                state.playing.push(Splat {
                    slot,
                    started: now,
                    damage: before - after,
                    death: after == 0,
                    second,
                });
            }
        }
    }
    state.health = health;
    state.playing.retain(|s| (now - s.started) / SPLAT_FRAME < if s.death { 30.0 } else { 8.0 });
    let Ok(layer) = layers.single() else { return };
    commands.entity(layer).despawn_related::<Children>();
    let sheet = |h: &Handle<SpriteSheet>| sheets.get(h);
    for splat in &state.playing {
        let frame = ((now - splat.started) / SPLAT_FRAME) as usize;
        let handle = match (splat.death, splat.second) {
            (true, _) => &art.death_splat,
            (false, false) => &art.damage_splat,
            (false, true) => &art.damage_splat2,
        };
        let at = Vec2::new(
            if splat.slot % 2 == 0 { 22.0 } else { 533.0 },
            18.0 + 85.0 * (splat.slot / 2) as f32,
        );
        if let Some(part) = sheet(handle).and_then(|s| sprite(s, frame, at)) {
            commands.spawn((part, ChildOf(layer)));
        }
        if frame < 6
            && let Some(font) = sheet(&art.bold_font)
        {
            let line =
                text(font, &mut images, &splat.damage.to_string(), at + Vec2::new(0.0, 4.0), 82.0);
            commands.spawn((line, ChildOf(layer)));
        }
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
/// party's radius, growing to the 38-pixel rim at extreme range). Inside
/// the party's radius monsters sit on its edge and items (`spread`) are
/// spread across it.
fn blip_position(delta: Vec3, yaw: f32, spread: bool) -> Vec2 {
    const SCALE: f32 = 2.0;
    const RIM: f32 = 38.0;
    let inner = 1.0;
    let outer = 50.0 + inner;
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let flat = Vec2::new(delta.dot(right), delta.dot(forward));
    let distance = flat.length();
    let pixels = if distance <= inner {
        if spread { SCALE * distance / inner } else { SCALE }
    } else {
        (SCALE + (distance - inner) * (RIM - SCALE) / (outer - inner)).min(RIM)
    };
    let unit = flat.normalize_or_zero();
    Vec2::new(75.0 + unit.x * pixels, 404.0 - unit.y * pixels)
}

/// What the radar knows of lying items (retail's `RADAR_SEEN` flag, cleared
/// whenever a level is entered, and `LightRadarBlip`'s 10-second glow for
/// a find).
#[derive(Default)]
pub struct RadarItems {
    seen: HashSet<Entity>,
    hidden: HashSet<Entity>,
    /// Glowing until this time.
    lit: HashMap<Entity, f32>,
}

const ITEM_GLOW_SECONDS: f32 = 10.0;

/// Turn the compass rim with the party, and mark the monsters within the
/// far clip: hostile red, others yellow, brighter above and dimmer below;
/// and the items once seen, grey to white.
#[allow(clippy::too_many_arguments)]
pub fn update_radar(
    mut commands: Commands,
    party: Query<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>,
    camera: Query<(&Projection, &GlobalTransform), With<PartyCamera>>,
    mut rims: Query<&mut UiTransform, With<RadarRim>>,
    layers: Query<Entity, With<RadarBlips>>,
    monsters: Query<(&crate::game::world::population::Monster, &GlobalTransform)>,
    (items, time, spatial, bodies): (
        Query<(Entity, &crate::game::world::items::WorldItem, &GlobalTransform)>,
        Res<Time>,
        avian3d::prelude::SpatialQuery,
        Query<&avian3d::prelude::RigidBody>,
    ),
    data: Option<Res<GameData>>,
    species: Res<Assets<crate::content::species::MonsterCatalog>>,
    mut known: Local<RadarItems>,
) {
    let (Ok((position, body)), Ok(layer)) = (party.single(), layers.single()) else { return };
    for mut rim in &mut rims {
        // North stays put while the view turns.
        rim.rotation = Rot2::radians(body.yaw);
    }
    let (far, eye) = match camera.single() {
        Ok((Projection::Perspective(p), eye)) => (p.far, eye.translation()),
        _ => (0.0, position.0),
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
            absolute(blip_position(delta, body.yaw, false) - Vec2::ONE, Vec2::splat(2.0)),
            BackgroundColor(Color::srgb(r, g, b)),
            Pickable::IGNORE,
            ChildOf(layer),
        ));
    }
    // Items: once the eye has had them in sight, within the far clip.
    let now = time.elapsed_secs();
    let level = |entity| bodies.get(entity).is_ok_and(avian3d::prelude::RigidBody::is_static);
    let filter = avian3d::prelude::SpatialQueryFilter::default();
    known.seen.retain(|e| items.contains(*e));
    known.lit.retain(|e, until| items.contains(*e) && *until > now);
    for (entity, item, at) in &items {
        if item.hidden {
            known.hidden.insert(entity);
            continue;
        }
        if known.hidden.remove(&entity) {
            known.lit.insert(entity, now + ITEM_GLOW_SECONDS);
        }
        let delta = at.translation() - position.0;
        if delta.length() > far {
            continue;
        }
        if !known.seen.contains(&entity) {
            let to = at.translation();
            let in_sight = Dir3::new(to - eye).is_ok_and(|direction| {
                spatial
                    .cast_ray_predicate(eye, direction, eye.distance(to), true, &filter, &level)
                    .is_none()
            });
            if !in_sight {
                continue;
            }
            known.seen.insert(entity);
        }
        let ring = if delta.y.abs() < 3.0 {
            1
        } else if delta.y > 0.0 {
            2
        } else {
            0
        };
        // Retail makes a find's blip glow between the class's dim and
        // bright colours; here it shows the bright one.
        let ring = if known.lit.contains_key(&entity) { 2 } else { ring };
        let [r, g, b] = BLIP_COLOURS[ITEM_BLIPS * 3 + ring];
        commands.spawn((
            absolute(blip_position(delta, body.yaw, true) - Vec2::ONE, Vec2::splat(2.0)),
            BackgroundColor(Color::srgb(r, g, b)),
            Pickable::IGNORE,
            ChildOf(layer),
        ));
    }
}

/// Items' blip class.
const ITEM_BLIPS: usize = 4;

/// A working bank button.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Bank {
    Camp,
    Search,
    Formation,
    Order(Order),
    /// The combat round button: left click starts a round, or switches
    /// stance while one plays.
    Round,
}

fn press_bank(
    click: On<PointerClick>,
    buttons: Query<&Bank>,
    mut camp: MessageWriter<crate::game::camp::CampRequest>,
    mut search: MessageWriter<crate::game::world::search::ToggleSearch>,
    mut layout: ResMut<ScreenLayout>,
    mut orders: ResMut<PartyOrders>,
    selected: Res<SelectedMember>,
    mut round: MessageWriter<StartRound>,
    mut settings: ResMut<CombatSettings>,
    battle: Option<Res<crate::game::combat::Battle>>,
) {
    match buttons.get(click.entity) {
        Ok(Bank::Order(order)) => orders.set(selected.0, *order),
        Ok(Bank::Round) => {
            if battle.is_some_and(|b| b.executing) {
                settings.continuous = !settings.continuous;
            } else {
                round.write(StartRound);
            }
        }
        Ok(Bank::Camp) => {
            camp.write(crate::game::camp::CampRequest);
        }
        Ok(Bank::Search) => {
            search.write(crate::game::world::search::ToggleSearch);
        }
        // The Party Formation button opens and closes the formation panel.
        Ok(Bank::Formation) => layout.toggle(Panel::Formation),
        Err(_) => {}
    }
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
pub fn paint_hover_art(
    mut buttons: Query<(&Hovered, &HoverArt, &mut ImageNode), Changed<Hovered>>,
) {
    for (hovered, art, mut image) in &mut buttons {
        image.image = if hovered.get() { art.hovered.clone() } else { art.normal.clone() };
    }
}

/// A text box scroll arrow: lines to move by.
#[derive(Component, Clone, Copy)]
struct TextScroll(i32);

fn scroll_text(click: On<PointerClick>, arrows: Query<&TextScroll>, mut log: ResMut<TextLog>) {
    match arrows.get(click.entity) {
        Ok(TextScroll(lines)) if *lines < 0 => log.scroll_up(lines.unsigned_abs() as usize),
        Ok(TextScroll(lines)) => log.scroll_down(*lines as usize),
        Err(_) => {}
    }
}

/// PgUp, PgDn, Home, End and Del (`mgskeyboard.ini` 500-504): scroll the
/// text box a page, to its top or its end, or empty it.
pub fn text_box_keys(keys: Res<ButtonInput<KeyCode>>, mut log: ResMut<TextLog>) {
    if keys.just_pressed(KeyCode::PageUp) {
        log.scroll_up(super::notices::VISIBLE_LINES);
    }
    if keys.just_pressed(KeyCode::PageDown) {
        log.scroll_down(super::notices::VISIBLE_LINES);
    }
    if keys.just_pressed(KeyCode::Home) {
        log.scroll_to(0);
    }
    if keys.just_pressed(KeyCode::End) {
        log.scroll_to_end();
    }
    if keys.just_pressed(KeyCode::Delete) {
        log.clear();
    }
}

/// A text box channel tab.
#[derive(Component, Clone, Copy)]
struct Channel(usize);

fn select_channel(click: On<PointerClick>, tabs: Query<&Channel>, mut log: ResMut<TextLog>) {
    if let Ok(tab) = tabs.get(click.entity) {
        log.select(tab.0);
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
    hp: u32,
    hp_fill: u32,
    stamina_fill: u32,
    name: String,
    armour: i32,
    /// The main hand's weapon, by name.
    weapon: Option<String>,
    /// The worst condition's icon (`data/icons/conditions`), if any.
    condition: Option<String>,
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
    formation: Formation,
    /// The text box's channel and the lines it shows.
    text: (usize, Vec<String>),
    /// Whether it scrolls up and down, and where its thumb sits.
    text_scroll: ((bool, bool), Option<f32>),
    fighting: bool,
    /// Search mode, which keeps the search button pressed.
    searching: bool,
    orders: Vec<Order>,
    continuous: bool,
    /// The round's progress while one plays.
    progress: Option<f32>,
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
    (log, battle, formation, orders, settings, search): (
        Res<TextLog>,
        Option<Res<crate::game::combat::Battle>>,
        Res<Formation>,
        Res<PartyOrders>,
        Res<CombatSettings>,
        Res<crate::game::world::search::Search>,
    ),
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
                hp: character.hp,
                hp_fill: fill(character.hp, character.max_hp()),
                stamina_fill: fill(character.stamina, character.max_stamina()),
                name: character.name.clone(),
                weapon: carried
                    .and_then(|c| c.worn(EquipSlot::Primary))
                    .and_then(|i| catalog?.get(i.item))
                    .map(|d| d.name.clone()),
                armour,
                condition: if character.is_dead() {
                    Some("dead".into())
                } else {
                    character.worst_condition().map(|c| format!("{c:?}").to_lowercase())
                },
                two_handed: main.as_ref().is_some_and(|m| m.1),
                hands: [main.map(|m| m.0), off.map(|o| o.0)],
            })
        })
        .collect();
    let shown = Shown {
        layout: layout.clone(),
        slots,
        selected: selected.0,
        text: (log.selected(), log.visible().to_vec()),
        text_scroll: (log.can_scroll(), log.thumb()),
        fighting: battle.is_some(),
        searching: search.on,
        orders: (0..8).map(|slot| orders.of(slot)).collect(),
        continuous: settings.continuous,
        progress: battle.as_ref().and_then(|b| b.progress()),
        formation: formation.clone(),
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
        Some(cage),
        Some(symbols),
        Some(jewels),
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
        get(&art.formation_cage),
        get(&art.formation_symbols),
        get(&art.formation_jewels),
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
        // Retail's default shows hit points as a number, with wide bars
        // (`g_settings.numeric_hit_points`, on in `ResetGameplaySettings`).
        put(&mut commands, sprite(info, 3, Vec2::new(bars_x, y)));
        if !view.dead {
            // The full bar, then its empty part blacked out from the top.
            for (file, dx, fill) in
                [("healthbarwide", 3.0, view.hp_fill), ("staminabarwide", 9.0, view.stamina_fill)]
            {
                let at = Vec2::new(bars_x + dx, y + 17.0);
                put(
                    &mut commands,
                    sheet(format!("data/main interface/{file}.sti")).and_then(|s| sprite(s, 0, at)),
                );
                commands.spawn((
                    absolute(at, Vec2::new(5.0, (45 - fill) as f32)),
                    BackgroundColor(Color::srgb_u8(1, 1, 1)),
                    Pickable::IGNORE,
                    ChildOf(frame),
                ));
            }
            commands.spawn((
                absolute(Vec2::new(bars_x + 2.0, y + 62.0), Vec2::new(19.0, 9.0)),
                BackgroundColor(Color::BLACK),
                Pickable::IGNORE,
                ChildOf(frame),
            ));
            let hp = view.hp.to_string();
            put(
                &mut commands,
                Some(text(small_font, &mut images, &hp, Vec2::new(bars_x + 2.0, y + 62.0), 19.0)),
            );
        }
        // The condition and enchantment sockets (retail
        // `RedrawPartyPortraitOverlay` step 7): the condition on the
        // portrait's outer side, plugs where there is nothing; with a
        // condition both sockets are highlighted, a retail quirk.
        if closed {
            let (condition_x, enchantment_x) = if left {
                (menu_x(slot) + 25.0, menu_x(slot) + 88.0)
            } else {
                (menu_x(slot) + 88.0, menu_x(slot) + 25.0)
            };
            let plug = |x: f32| if x < 320.0 { "leftplug" } else { "rightplug" };
            let condition = match &view.condition {
                Some(name) => sheet(format!("data/icons/conditions/{name}.sti")),
                None => sheet(format!("data/main interface/{}.sti", plug(condition_x))),
            };
            put(
                &mut commands,
                condition.and_then(|s| sprite(s, 0, Vec2::new(condition_x, y + 3.0))),
            );
            let enchantment = sheet(format!("data/main interface/{}.sti", plug(enchantment_x)));
            put(
                &mut commands,
                enchantment.and_then(|s| sprite(s, 0, Vec2::new(enchantment_x, y + 3.0))),
            );
            if view.condition.is_some() {
                for x in [condition_x, enchantment_x] {
                    let rim = sheet("data/main interface/condition_highlight.sti".into());
                    put(&mut commands, rim.and_then(|s| sprite(s, 0, Vec2::new(x - 1.0, y + 2.0))));
                }
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
    // Damage splats play over the portraits.
    if closed {
        commands.spawn((
            Splats,
            absolute(Vec2::ZERO, Vec2::ZERO),
            Pickable::IGNORE,
            ChildOf(frame),
        ));
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
        // Its scroll arrows and thumb (retail `RedrawTextBoxScrollChrome`):
        // grayed where it cannot scroll; the thumb when the text overflows.
        if let Some(scroll) = sheets.get(&art.text_scroll) {
            let ((up, down), thumb) = shown.text_scroll;
            for (enabled, normal, grayed, y, lines) in
                [(up, 0, 3, 363.0, -1), (down, 8, 11, 429.0, 1)]
            {
                let image = if enabled { normal } else { grayed };
                if let Some((node_image, node, _)) = sprite(scroll, image, Vec2::new(461.0, y)) {
                    let mut arrow = commands.spawn((node_image, node, ChildOf(frame)));
                    if enabled {
                        arrow.insert((TextScroll(lines), hover_art(scroll, image, image + 1)));
                        arrow.observe(scroll_text);
                    }
                }
            }
            if let Some(thumb) = thumb {
                let at = Vec2::new(461.0, 379.0 + (thumb * 40.0).floor());
                put(&mut commands, sprite(scroll, 4, at));
            }
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
        // Left-clicking the radar opens the automap (retail region 0x62, a
        // circle of radius 46 about (75, 402)).
        commands
            .spawn((
                Node {
                    border_radius: BorderRadius::all(Val::Px(46.0)),
                    ..absolute(Vec2::new(29.0, 356.0), Vec2::splat(92.0))
                },
                ChildOf(frame),
            ))
            .observe(
                |_: On<PointerClick>, mut open: MessageWriter<super::automap::OpenAutomap>| {
                    open.write(super::automap::OpenAutomap);
                },
            );
    }
    // The formation board: the cage, the view cone and each member's facing
    // arrow and marching-order chip in their cell (retail
    // `DrawFormationSlotMarkers`).
    if layout.shows(Panel::Formation) {
        let board = Vec2::new(519.0, 359.0);
        put(&mut commands, sprite(cage, 0, board));
        put(&mut commands, sprite(view_cone, 0, board));
        for slot in 0..shown.slots.len() {
            let Some((row, seat)) = shown.formation.place_of(slot) else { continue };
            let (x, y) = FORMATION_CELLS[usize::from(row) * 3 + seat];
            let at = board + Vec2::new(x, y);
            put(&mut commands, sprite(symbols, Formation::facing(row) * 3, at));
            put(&mut commands, sprite(jewels, slot * 3, at));
        }
    }
    // The button bank (retail `pc.cpp` positions and frames: grayed, off,
    // ..., on). Camp, search and the formation panel work; the journal,
    // items, spells and options are grayed until they exist.
    let fighting = shown.fighting;
    // Each button's frames: grayed, off, hovered, on, on and hovered.
    let mut bank: Vec<(&SpriteSheet, f32, f32, [usize; 5], bool, Option<Bank>)> = vec![
        (options, 0.0, 450.0, [3, 0, 1, 2, 2], false, None),
        (icons, 40.0, 452.0, [0x2b, 0x28, 0x29, 0x2a, 0x2c], false, None),
        (icons, 70.0, 452.0, [3, 0, 1, 2, 4], false, Some(Bank::Camp)),
        (icons, 100.0, 452.0, [8, 5, 6, 7, 9], shown.searching, Some(Bank::Search)),
        (icons, 130.0, 452.0, [0x17, 0x14, 0x15, 0x16, 0x18], false, None),
        (icons, 160.0, 452.0, [0x1c, 0x19, 0x1a, 0x1b, 0x1d], false, None),
        (
            icons,
            571.0,
            452.0,
            [0x26, 0x23, 0x24, 0x25, 0x27],
            layout.shows(Panel::Formation),
            Some(Bank::Formation),
        ),
    ];
    if fighting {
        // End Combat (ending a fight by choice is not modelled), Defend and
        // Attack set the selected member's order.
        let order = shown.orders.get(shown.selected).copied().unwrap_or_default();
        bank.push((combat_toggle, 200.0, 452.0, [7, 4, 5, 6, 6], false, None));
        bank.push((
            icons,
            232.0,
            452.0,
            [0x12, 0xf, 0x10, 0x11, 0x13],
            order == Order::Defend,
            Some(Bank::Order(Order::Defend)),
        ));
        bank.push((
            icons,
            262.0,
            452.0,
            [0xd, 0xa, 0xb, 0xc, 0xe],
            order == Order::Attack,
            Some(Bank::Order(Order::Attack)),
        ));
        // The round button (retail `UpdateCombatStanceButtons`): Start
        // Round while a phased fight waits, Go Continuous while its round
        // plays; Go Phased while a continuous one plays, Start Round between.
        let file = match (shown.continuous, shown.progress.is_some()) {
            (false, false) => &art.attack_confirm,
            (false, true) => &art.cont_toggle,
            (true, true) => &art.combat_stop,
            (true, false) => &art.cont_start,
        };
        if let Some((node_image, node, _)) =
            sheets.get(file).and_then(|s| sprite(s, 0, Vec2::new(610.0, 450.0)))
        {
            commands.spawn((node_image, node, Bank::Round, ChildOf(frame))).observe(press_bank);
        }
        // The round's progress along the roof.
        if let Some(progress) = shown.progress
            && let Some((node_image, mut node, pick)) =
                sheets.get(&art.progress).and_then(|s| sprite(s, 0, Vec2::new(177.0, 2.0)))
        {
            node.width = Val::Px(286.0 * progress);
            node.overflow = Overflow::clip();
            commands.spawn((node_image, node, pick, ChildOf(frame)));
        }
    } else {
        bank.push((combat_toggle, 199.0, 452.0, [3, 0, 1, 2, 2], false, None));
    }
    for (sheet, x, y, [grayed, off, hover, on, on_hover], pressed, action) in bank {
        // The journal, camp and the combat toggle are grayed in a fight.
        let usable =
            action.is_some() && !(fighting && matches!(action, Some(Bank::Camp | Bank::Search)));
        let (image, hover) = match (usable, pressed) {
            (false, _) => (grayed, grayed),
            (true, false) => (off, hover),
            (true, true) => (on, on_hover),
        };
        if let Some((node_image, node, _)) = sprite(sheet, image, Vec2::new(x, y)) {
            let mut button = commands.spawn((node_image, node, ChildOf(frame)));
            if let (true, Some(action)) = (usable, action) {
                button.insert(action).observe(press_bank);
                button.insert(hover_art(sheet, image, hover));
            }
        }
    }
    // Previous and next character (retail `main_scroll_arrows`).
    for (image, x, step) in [(0, 300.0, -1), (4, 323.0, 1)] {
        if let Some((node_image, node, _)) = sprite(scroll_arrows, image, Vec2::new(x, 456.0)) {
            commands
                .spawn((
                    node_image,
                    node,
                    Step(step),
                    hover_art(scroll_arrows, image, image + 1),
                    ChildOf(frame),
                ))
                .observe(step_selection);
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
        // The second line: the order in a fight ("Attack - weapon", with
        // "Unarmed" for an empty hand), else the worst condition.
        let second = if shown.fighting {
            match shown.orders[shown.selected] {
                Order::Defend => Some("Defend".to_owned()),
                Order::Attack => {
                    Some(format!("Attack - {}", view.weapon.as_deref().unwrap_or("Unarmed")))
                }
            }
        } else {
            view.condition.as_ref().map(|c| {
                let mut c = c.clone();
                c[..1].make_ascii_uppercase();
                c
            })
        };
        if let Some(second) = second {
            put(
                &mut commands,
                Some(text(small_font, &mut images, &second, Vec2::new(343.0, 465.0), 185.0)),
            );
        }
    }
    *drawn = complete.then_some(shown);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blips_sit_ahead_and_beyond_on_the_rim() {
        // Ahead of a party facing -Z, 10 m off: straight up from the centre.
        let ahead = blip_position(Vec3::new(0.0, 0.0, -10.0), 0.0, false);
        assert_eq!(ahead.x, 75.0);
        assert!(ahead.y < 404.0 && ahead.y > 404.0 - 38.0);
        // Far beyond extreme range: on the 38-pixel rim, to the right.
        assert_eq!(blip_position(Vec3::new(200.0, 0.0, 0.0), 0.0, false), Vec2::new(113.0, 404.0));
        // Within the party's radius a monster sits on its edge, an item
        // where it is.
        let near = Vec3::new(0.5, 0.0, 0.0);
        assert_eq!(blip_position(near, 0.0, false), Vec2::new(77.0, 404.0));
        assert_eq!(blip_position(near, 0.0, true), Vec2::new(76.0, 404.0));
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
