//! The character review screen's item page (`docs/research/review-items.md`):
//! a member's equipment on their race's statue, their backpack, and the
//! party's pool. Opened with I for the selected member; the header's portraits
//! switch member, Esc, I or the Exit button closes it. Retail fills empty
//! cells with a PCX texture; a flat colour stands in for it.
//!
//! The screen is one scene with a node for each cell, the header and the
//! gold; their retained nodes update when their gameplay inputs change.

use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

use super::screen_art::{
    BitmapText, Picture, absolute, centred, label, picture, screen, sprite, text,
};
use crate::content::items::{ItemCatalog, QuantityKind};
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Gender, Race};
use crate::game::data::GameData;
use crate::game::inventory::{
    BACKPACK_SLOTS, Carried, EquipSlot, ItemInstance, ItemLocation, PartyInventory, Quantity, Slot,
};
use crate::game::party::roster::PartyRoster;
use crate::game::screen::Screen;

const CELL: Vec2 = Vec2::new(46.0, 54.0);
const FILL: Color = Color::srgb(0.16, 0.14, 0.11);
/// Pool cells shown at once.
const POOL_CELLS: usize = 8;

/// Equipment cells on the paper doll, by equipment slot: position and size.
fn equipment_cell(slot: EquipSlot) -> Rect {
    let (x, y, width, height) = match slot {
        EquipSlot::Head => (188.0, 176.0, 45.0, 57.0),
        EquipSlot::Misc1 => (431.0, 198.0, 40.0, 40.0),
        EquipSlot::Misc2 => (431.0, 241.0, 40.0, 40.0),
        EquipSlot::Cloak => (376.0, 176.0, 38.0, 73.0),
        EquipSlot::Torso => (134.0, 241.0, 57.0, 58.0),
        EquipSlot::Hands => (387.0, 257.0, 35.0, 43.0),
        EquipSlot::Primary => (158.0, 308.0, 34.0, 101.0),
        EquipSlot::Secondary => (400.0, 308.0, 34.0, 72.0),
        EquipSlot::AlternatePrimary => (121.0, 308.0, 34.0, 101.0),
        EquipSlot::AlternateSecondary => (437.0, 308.0, 34.0, 72.0),
        EquipSlot::Legs => (203.0, 359.0, 34.0, 77.0),
        EquipSlot::Feet => (362.0, 378.0, 26.0, 58.0),
    };
    Rect::new(x, y, x + width, y + height)
}

/// The slot's highlight rim, its first frame in the highlights sheet.
fn equipment_rim(slot: EquipSlot) -> usize {
    match slot {
        EquipSlot::Head => 0,
        EquipSlot::Misc1 => 40,
        EquipSlot::Misc2 => 36,
        EquipSlot::Cloak => 44,
        EquipSlot::Torso => 4,
        EquipSlot::Hands => 32,
        EquipSlot::Primary => 12,
        EquipSlot::Secondary => 24,
        EquipSlot::AlternatePrimary => 8,
        EquipSlot::AlternateSecondary => 28,
        EquipSlot::Legs => 16,
        EquipSlot::Feet => 20,
    }
}

/// Where a backpack (left) or pool (right) cell is: two columns of four.
fn grid(left: f32, i: usize) -> Vec2 {
    Vec2::new(left + (i % 2) as f32 * 49.0, 192.0 + (i / 2) as f32 * 57.0)
}

/// The open screen, reviewing one member. Absent while exploring.
#[derive(Component, Debug)]
pub struct InventoryScreen {
    pub member: Entity,
    /// The first pool entry shown; always even.
    pub pool_scroll: usize,
}

#[derive(Resource)]
pub struct InventoryArt {
    corner: Handle<SpriteSheet>,
    page: Handle<SpriteSheet>,
    bottom_bar: Handle<SpriteSheet>,
    gold_icon: Handle<SpriteSheet>,
    highlights: Handle<SpriteSheet>,
    unusable: Handle<SpriteSheet>,
    unidentified: Handle<SpriteSheet>,
    exit: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    bold: Handle<SpriteSheet>,
}

pub fn load_inventory_art(mut commands: Commands, assets: Res<AssetServer>) {
    let load = |path: &str| assets.load(format!("retail://data/{path}"));
    commands.insert_resource(InventoryArt {
        corner: load("review/commoncorner.sti"),
        page: load("review/reviewitempage.sti"),
        bottom_bar: load("review/bottombuttonbar.sti"),
        gold_icon: load("review/reviewgoldicon.sti"),
        highlights: load("review/reviewitemhighlights.sti"),
        unusable: load("review/itemusablebackground.sti"),
        unidentified: load("review/itemunidentifiedoverlay.sti"),
        exit: load("char generation/cg_bottombuttons.sti"),
        font: load("fonts/wiz_text_font.sti"),
        bold: load("fonts/wiz_text_font_bold.sti"),
    });
}

/// The screen, with its keys: Escape or I closes it, before every other
/// screen's keys.
#[derive(Component, Clone, Default)]
pub struct InventoryRoot;

/// I opens the screen while the party is in the world.
#[derive(Component, Clone, Default)]
pub struct InventoryKey;

#[derive(InputAction)]
#[action_output(bool)]
struct OpenInventory;

#[derive(InputAction)]
#[action_output(bool)]
struct CloseInventory;

pub fn spawn_inventory_key(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        InventoryKey
        Actions<InventoryKey> [
            Action::<OpenInventory>::new()
            Bindings [ Binding(KeyCode::KeyI) ]
        ]
        on(open_with_key)
    });
}

/// Retail opens the selected character.
fn open_with_key(
    _: On<Start<OpenInventory>>,
    screen: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    roster: Res<PartyRoster>,
    selected: Res<super::main_screen::SelectedMember>,
) {
    if *screen.get() == Screen::World && selected.member(&roster).is_some() {
        next.set(Screen::Inventory);
    }
}

/// Screen state owns existence; the root owns its local selection and input.
fn enter_inventory(
    mut commands: Commands,
    roster: Res<PartyRoster>,
    selected: Res<super::main_screen::SelectedMember>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(member) = selected.member(&roster) else {
        next.set(Screen::World);
        return;
    };
    commands.spawn_scene(bsn! {
        @centred()
        InventoryRoot
        BackgroundColor(Color::BLACK)
        GlobalZIndex(10)
        ContextPriority::<InventoryRoot>::new(1)
        Actions<InventoryRoot> [
            Action::<CloseInventory>::new()
            ActionSettings { consume_input: true }
            Bindings [ Binding(KeyCode::Escape) Binding(KeyCode::KeyI) ]
        ]
        on(|_: On<Start<CloseInventory>>, mut next: ResMut<NextState<Screen>>| { next.set(Screen::World); })
    }).insert((InventoryScreen { member, pool_scroll: 0 }, DespawnOnExit(Screen::Inventory)));
}

/// This explicit exit consequence uses the same inventory operation as any
/// other caller, before Bevy removes the state-scoped root.
fn put_away_held(
    screen: Option<Single<&InventoryScreen>>,
    mut inventory: crate::game::inventory::operations::InventoryOperations,
) {
    if let Some(screen) = screen {
        let _ = inventory.put_away(screen.member);
    }
}

#[derive(Component)]
pub struct InventoryBuilt;

/// The race's statue on the item page (`statue_<race>_<m|f>.sti`).
fn statue(race: Race, gender: Gender) -> String {
    let race = match race {
        Race::Human => "human",
        Race::Elf => "elf",
        Race::Dwarf => "dwarf",
        Race::Gnome => "gnome",
        Race::Hobbit => "hobbit",
        Race::Faerie => "fairy",
        Race::Lizardman => "lizard",
        Race::Dracon => "dracon",
        Race::Felpurr => "felpurr",
        Race::Rawulf => "rawulf",
        Race::Mook => "mook",
        Race::Trynnie => return "statue_trynnie".into(),
        Race::TRang => return "statue_trang".into(),
        Race::Umpani => return "statue_umpani".into(),
        Race::Rapax => return "statue_rapax".into(),
        Race::Android => return "statue_android".into(),
    };
    let gender = if gender == Gender::Female { "f" } else { "m" };
    format!("statue_{race}_{gender}")
}

/// A place an item can be clicked in or out of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cell {
    Member(Slot),
    /// A pool cell by its position on screen.
    Pool(usize),
}

impl Cell {
    /// Where the cell is and its size.
    fn area(self) -> Rect {
        match self {
            Cell::Member(Slot::Equipment(slot)) => equipment_cell(slot),
            Cell::Member(Slot::Backpack(i)) => {
                Rect::from_corners(grid(11.0, i), grid(11.0, i) + CELL)
            }
            Cell::Pool(i) => Rect::from_corners(grid(512.0, i), grid(512.0, i) + CELL),
        }
    }
}

#[derive(Component)]
struct ItemCell {
    cell: Cell,
}

#[derive(Clone, Copy)]
enum CellPart {
    Picture,
    Count,
    Unusable,
    Unidentified,
    Rim,
}

#[derive(Component)]
struct CellDisplay {
    cell: Cell,
    part: CellPart,
}

/// A cell's count: a stack above one, charges or shots; `?` for charges
/// not yet identified.
fn count_label(item: &ItemInstance, kind: QuantityKind) -> Option<String> {
    let count = match item.quantity {
        Quantity::One => return None,
        Quantity::Stack(n) | Quantity::Uses(n) => n,
    };
    match kind {
        QuantityKind::None | QuantityKind::Unlabelled => None,
        QuantityKind::Stack if count <= 1 => None,
        QuantityKind::Charges | QuantityKind::Uses if !item.identified => Some("?".to_owned()),
        _ => Some(count.to_string()),
    }
}

#[derive(Clone, Copy)]
enum HeaderPart {
    Member(usize),
    Portrait,
    Statue,
    Name,
    Race,
    Profession,
}
#[derive(Component)]
struct HeaderDisplay(HeaderPart);
#[derive(Component, Clone, Copy, PartialEq, Eq, FromTemplate)]
struct InventoryMemberButton(Entity);
#[derive(Component, Clone, Default)]
struct GoldLabel;

fn header(art: &InventoryArt) -> impl SceneList {
    let portraits: Vec<_> = (0..8)
        .map(|slot| {
            let at = Vec2::new(5.0 + (slot % 2) as f32 * 48.0, 4.0 + (slot / 2) as f32 * 39.0);
            bsn! {
                @picture(default(),0,at)
                Pickable::default()
                template(move |_| Ok(HeaderDisplay(HeaderPart::Member(slot))))
                InventoryMemberButton({Entity::PLACEHOLDER})
                on(select_inventory_member)
            }
        })
        .collect();
    bsn_list! {
        {portraits}
        -- @picture(default(),0,Vec2::new(164.0,12.0)) template(|_|Ok(HeaderDisplay(HeaderPart::Portrait)))
        -- @picture(art.corner.clone(),9,Vec2::new(164.0,12.0))
        -- @label(art.bold.clone(),Vec2::new(116.0,96.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Name)))
        -- @label(art.font.clone(),Vec2::new(116.0,111.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Race)))
        -- @label(art.font.clone(),Vec2::new(116.0,124.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Profession)))
        -- @picture(default(),0,Vec2::new(194.0,165.0)) template(|_|Ok(HeaderDisplay(HeaderPart::Statue)))
    }
}

fn select_inventory_member(
    click: On<PointerClick>,
    buttons: Query<&InventoryMemberButton>,
    roster: Res<PartyRoster>,
    screen: Option<Single<&mut InventoryScreen>>,
) {
    if let Ok(button) = buttons.get(click.entity)
        && roster.0.contains(&button.0)
        && let Some(mut screen) = screen
    {
        screen.member = button.0;
    }
}

fn cell_controls(cell: Cell, art: &InventoryArt) -> impl Scene {
    let area = cell.area();
    let size = area.size();
    let (frame, at, count_at, width) = match cell {
        Cell::Member(Slot::Equipment(_)) => {
            (1, Vec2::ZERO, Vec2::new(2.0, size.y - 13.0), size.x - 4.0)
        }
        _ => (0, Vec2::ONE, Vec2::new(1.0, 40.0), 44.0),
    };
    let grid = !matches!(cell, Cell::Member(Slot::Equipment(_)));
    let background = if grid { FILL } else { Color::NONE };
    let node = absolute(area.min, size + 1.0);
    let rim = match cell {
        Cell::Member(Slot::Equipment(slot)) => equipment_rim(slot) + 3,
        _ => 0,
    };
    bsn! {
        node BackgroundColor({background}) template(move |_|Ok(ItemCell{cell}))
        Pickable::default()
        on(click_cell)
        Children [
            @picture(art.unusable.clone(),0,Vec2::ZERO)
            template(move |_|Ok(CellDisplay{cell,part:CellPart::Unusable}))
            -- @picture(default(),frame,at)
            template(move |_|Ok(CellDisplay{cell,part:CellPart::Picture}))
            -- @label(art.font.clone(),count_at,width)
            template(move |_|Ok(CellDisplay{cell,part:CellPart::Count}))
            -- @picture(art.unidentified.clone(),0,Vec2::ZERO)
            template(move |_|Ok(CellDisplay{cell,part:CellPart::Unidentified}))
            -- @picture(art.highlights.clone(),rim,-Vec2::splat(2.0))
            template(move |_|Ok(CellDisplay{cell,part:CellPart::Rim}))
        ]
    }
}

/// Build the screen when it opens, once its fixed art has loaded; remove
/// it when it closes.
pub fn spawn_inventory_screen(
    mut commands: Commands,
    art: Res<InventoryArt>,
    assets: Res<AssetServer>,
    mut next: ResMut<NextState<Screen>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    roots: Query<Entity, (With<InventoryRoot>, Without<InventoryBuilt>)>,
) {
    let Ok(root) = roots.single() else { return };
    if [
        &art.corner,
        &art.page,
        &art.bottom_bar,
        &art.gold_icon,
        &art.exit,
        &art.bold,
        &art.highlights,
        &art.unusable,
        &art.unidentified,
        &art.font,
    ]
    .into_iter()
    .any(|handle| matches!(assets.get_load_state(handle), Some(bevy::asset::LoadState::Failed(_))))
    {
        warn!("inventory artwork failed to load; returning to the world");
        next.set(Screen::World);
        return;
    }
    let get = |handle: &Handle<SpriteSheet>| sheets.get(handle);
    let (Some(corner), Some(page), Some(bar), Some(gold), Some(exit), Some(bold)) = (
        get(&art.corner),
        get(&art.page),
        get(&art.bottom_bar),
        get(&art.gold_icon),
        get(&art.exit),
        get(&art.bold),
    ) else {
        return;
    };
    let fixed = vec![
        sprite(corner, 0, Vec2::ZERO),
        sprite(page, 0, Vec2::new(310.0, 0.0)),
        sprite(page, 1, Vec2::new(0.0, 165.0)),
        sprite(page, 2, Vec2::new(113.0, 165.0)),
        sprite(page, 3, Vec2::new(478.0, 165.0)),
        sprite(gold, 0, Vec2::new(514.0, 422.0)),
        sprite(bar, 0, Vec2::new(0.0, 450.0)),
    ];
    let titles = vec![
        text(bold, &mut images, "Carried", Vec2::new(12.0, 174.0), 93.0),
        text(bold, &mut images, "Party Items", Vec2::new(486.0, 174.0), 141.0),
    ];
    let exit = sprite(exit, 16, Vec2::new(596.0, 450.0));
    let cells: Vec<_> = EnumSet::<EquipSlot>::all()
        .iter()
        .map(|slot| Cell::Member(Slot::Equipment(slot)))
        .chain((0..BACKPACK_SLOTS).map(|i| Cell::Member(Slot::Backpack(i))))
        .chain((0..POOL_CELLS).map(Cell::Pool))
        .map(|cell| cell_controls(cell, &art))
        .collect();
    commands.entity(root).insert(InventoryBuilt).queue_spawn_related_scenes::<Children>(vec![bsn! {
        Node { position_type: PositionType::Absolute }
        Children [
            @screen()
            Children [
                {fixed}
                -- {titles}
                -- @{exit}
                    Pickable { should_block_lower: true, is_hoverable: true }
                    on(|_: On<PointerClick>, mut next: ResMut<NextState<Screen>>| { next.set(Screen::World); })
                // The header and statue lie under the cells.
                -- {header(&art)}
                -- @label(art.font.clone(),Vec2::new(553.0,425.0),48.0) GoldLabel
                -- {cells}
            ]
        ]
    }]);
}

/// Update the existing header controls from their actual dependencies.
fn draw_header(
    screen: Option<Single<Ref<InventoryScreen>>>,
    art: Res<InventoryArt>,
    assets: Res<AssetServer>,
    roster: Res<PartyRoster>,
    members: Query<Ref<Character>>,
    mut displays: Query<(
        Ref<HeaderDisplay>,
        Option<&mut InventoryMemberButton>,
        Option<&mut Picture>,
        Option<&mut BitmapText>,
    )>,
) {
    let Some(screen) = screen else { return };
    let Ok(profile) = members.get(screen.member) else { return };
    let changed = screen.is_changed()
        || roster.is_changed()
        || members.iter_many(&roster.0).flatten().any(|c| c.is_changed());
    for (display, button, picture, label) in &mut displays {
        if !display.is_added() && !changed {
            continue;
        }
        let mut source = None;
        let mut value = None;
        let mut anchor = None;
        let image = |path: String, frame| Picture {
            sheet: assets.load(format!("retail://{path}")),
            frame,
            ..default()
        };
        match display.0 {
            HeaderPart::Member(slot) => {
                let member = roster.0.get_index(slot).copied().filter(|m| members.contains(*m));
                if let Some(mut button) = button {
                    button.set_if_neq(InventoryMemberButton(member.unwrap_or(Entity::PLACEHOLDER)));
                }
                let occupied = member.is_some();
                let at = Vec2::new(
                    5.0 + (slot % 2) as f32 * 48.0 + f32::from(occupied),
                    4.0 + (slot / 2) as f32 * 39.0 + f32::from(occupied),
                );
                anchor = Some(at);
                source = Some(match member.and_then(|m| members.get(m).ok()) {
                    Some(profile) => {
                        image(format!("data/portraits/small/s{}.sti", profile.portrait), 0)
                    }
                    None => Picture { sheet: art.corner.clone(), frame: slot + 1, ..default() },
                });
            }
            HeaderPart::Portrait => {
                source = Some(image(format!("data/portraits/medium/m{}.sti", profile.portrait), 0))
            }
            HeaderPart::Statue => {
                source = Some(image(
                    format!("data/review/{}.sti", statue(profile.race, profile.gender)),
                    0,
                ))
            }
            HeaderPart::Name => value = Some(profile.name.clone()),
            HeaderPart::Race => value = Some(format!("{:?}", profile.race)),
            HeaderPart::Profession => value = Some(format!("{:?}", profile.profession)),
        }
        if let Some(mut picture) = picture
            && let Some(mut source) = source
        {
            source.at = anchor.unwrap_or(picture.at);
            picture.set_if_neq(source);
        }
        if let Some(mut label) = label
            && let Some(value) = value
            && label.value != value
        {
            label.value = value;
        }
    }
}

fn draw_gold(party: Res<PartyInventory>, mut labels: Query<&mut BitmapText, With<GoldLabel>>) {
    let value = format_gold(party.gold);
    for mut label in &mut labels {
        if label.value != value {
            label.value = value.clone();
        }
    }
}

fn draw_cells(
    screen: Option<Single<Ref<InventoryScreen>>>,
    assets: Res<AssetServer>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    members: Query<(Ref<Character>, Ref<Carried>)>,
    party: Res<PartyInventory>,
    mut displays: Query<(
        Ref<CellDisplay>,
        &mut Visibility,
        Option<&mut Picture>,
        Option<&mut BitmapText>,
    )>,
) {
    let Some(screen) = screen else { return };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    let Ok((profile, carried)) = members.get(screen.member) else { return };
    let changed = screen.is_changed()
        || profile.is_changed()
        || carried.is_changed()
        || party.is_changed()
        || catalogs.is_changed();
    for (display, mut visible, picture, label) in &mut displays {
        if !display.is_added() && !changed {
            continue;
        }
        let item = match display.cell {
            Cell::Member(Slot::Equipment(slot)) => carried.worn(slot),
            Cell::Member(Slot::Backpack(i)) => carried.backpack[i].as_ref(),
            Cell::Pool(i) => party.pool.get(screen.pool_scroll + i),
        };
        let definition = item.and_then(|i| Some((i, catalog.get(i.item)?)));
        let grid = !matches!(display.cell, Cell::Member(Slot::Equipment(_)));
        let count = definition.and_then(|(i, d)| count_label(i, d.quantity));
        let show = match display.part {
            CellPart::Picture => definition.is_some(),
            CellPart::Count => count.is_some(),
            CellPart::Unusable => grid && definition.is_some_and(|(_, d)| !profile.can_use(d)),
            CellPart::Unidentified => definition.is_some_and(|(i, _)| !i.identified),
            CellPart::Rim => !grid && definition.is_some(),
        };
        visible.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
        match display.part {
            CellPart::Picture => {
                if let Some((_, definition)) = definition
                    && let Some(mut picture) = picture
                {
                    let at = picture.at;
                    picture.set_if_neq(Picture {
                        sheet: assets.load(format!("retail://{}", definition.image)),
                        frame: if grid { 0 } else { 1 },
                        at,
                        bounds: None,
                    });
                }
            }
            CellPart::Count => {
                if let Some(mut label) = label {
                    if label.justify != bevy::text::Justify::Right {
                        label.justify = bevy::text::Justify::Right;
                    }
                    let value = count.unwrap_or_default();
                    if label.value != value {
                        label.value = value;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Click a cell (retail's item-page click, `rcs.cpp:1459-1742`): with an
/// empty hand, pick the item up; holding one, a pool cell takes it in, a
/// stack of the same item takes what it can, anything else swaps.
/// Shift-clicking a single unit, pouring ammunition and stowing a weapon's
/// partner are not done yet.
fn click_cell(
    click: On<PointerClick>,
    cells: Query<&ItemCell>,
    screen: Option<Single<&InventoryScreen>>,
    mut inventory: crate::game::inventory::operations::InventoryOperations,
) {
    let (Ok(cell), Some(screen)) = (cells.get(click.entity), screen) else { return };
    let destination = match cell.cell {
        Cell::Pool(row) => ItemLocation::Pool(screen.pool_scroll + row),
        Cell::Member(slot) => ItemLocation::Member(screen.member, slot),
    };
    if let Err(refusal) = inventory.activate(destination) {
        info!("inventory: {:?} refused: {refusal:?}", cell.cell);
    }
}

/// The held item at the pointer, here and on the main screen (retail's
/// item cursor): one node for the game's whole run, shown while the hand
/// holds something.
#[derive(Component, Clone, Default)]
pub struct HeldPicture;

pub fn spawn_held_picture(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        HeldPicture
        ImageNode
        Node { position_type: PositionType::Absolute }
        GlobalZIndex(20)
        Pickable::IGNORE
        Visibility::Hidden
    });
}

pub fn follow_held(
    party: Res<PartyInventory>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    scale: Res<UiScale>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    mut picture: Single<(&mut ImageNode, &mut Node, &mut Visibility), With<HeldPicture>>,
    mut loading: Local<Option<Handle<SpriteSheet>>>,
) {
    let (image, node, visibility) = &mut *picture;
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    let definition = party.held.as_ref().zip(catalog).and_then(|(h, c)| c.get(h.item.item));
    let cursor = pointers
        .iter()
        .find(|(id, _)| **id == PointerId::Mouse)
        .and_then(|(_, location)| location.location.as_ref())
        .map(|location| location.position);
    let (Some(definition), Some(cursor)) = (definition, cursor) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let handle = loading.insert(assets.load(format!("retail://{}", definition.image)));
    let Some((picture, frame)) =
        sheets.get(&*handle).and_then(|s| Some((s.images.first()?, s.frames.first()?)))
    else {
        return;
    };
    // Retail makes the picture the cursor, its corner at the hotspot with
    // the arrow drawn over it (`UpdateHeldItemCursor`).
    image.image = picture.clone();
    **node = absolute(cursor / scale.0, Vec2::new(frame.width as f32, frame.height as f32));
    visibility.set_if_neq(Visibility::Inherited);
}

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_input_context::<InventoryKey>()
            .add_input_context::<InventoryRoot>()
            .add_systems(Startup, (load_inventory_art, spawn_inventory_key, spawn_held_picture))
            .add_systems(OnEnter(Screen::Inventory), enter_inventory)
            .add_systems(OnExit(Screen::Inventory), put_away_held)
            .configure_sets(
                StateTransition,
                bevy::state::state::ExitSchedules::<Screen>::default()
                    .before(bevy::state::state_scoped::despawn_entities_on_exit_state::<Screen>),
            )
            .add_systems(
                Update,
                (spawn_inventory_screen, draw_header, draw_gold, draw_cells, follow_held).chain(),
            );
    }
}

/// Gold with thousands separated by commas.
fn format_gold(gold: u32) -> String {
    let digits = gold.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::ItemDefinition;

    #[test]
    fn closing_inventory_returns_the_held_item_even_before_art_is_ready() {
        use crate::game::characters::{Gender, Profession, Race};
        use crate::game::inventory::{Held, ItemIds, make_item};
        use crate::game::rng::GameRng;
        for still_a_member in [true, false] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                AssetPlugin::default(),
                bevy::scene::ScenePlugin,
                bevy::input::InputPlugin,
                bevy::state::app::StatesPlugin,
                EnhancedInputPlugin,
            ))
            .init_state::<Screen>()
            .init_asset::<SpriteSheet>()
            .init_asset::<Image>()
            .init_asset::<ItemCatalog>()
            .init_resource::<UiScale>()
            .init_resource::<PartyRoster>()
            .init_resource::<PartyInventory>()
            .init_resource::<super::super::main_screen::SelectedMember>()
            .add_plugins(InventoryPlugin);
            let definition = ItemDefinition { ..default() };
            let item = make_item(
                &definition,
                &mut ItemIds::default(),
                &mut GameRng::seeded(1),
                false,
                true,
            );
            let id = item.id;
            let items = app
                .world_mut()
                .resource_mut::<Assets<ItemCatalog>>()
                .add(ItemCatalog(vec![definition]));
            app.insert_resource(GameData { items, loot: default(), species: default() });
            let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
            let member = app
                .world_mut()
                .spawn((
                    crate::game::characters::Vitals::new(&profile),
                    profile,
                    Carried::default(),
                ))
                .id();
            app.world_mut().resource_mut::<PartyRoster>().0.insert(member);
            app.world_mut().resource_mut::<PartyInventory>().held =
                Some(Held { item, origin: None });
            app.world_mut().resource_mut::<super::super::main_screen::SelectedMember>().0 =
                Some(member);
            app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Inventory);
            app.finish();
            app.cleanup();
            app.update();
            if !still_a_member {
                app.world_mut().resource_mut::<PartyRoster>().0.shift_remove(&member);
            }
            app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::World);
            app.update();
            assert!(app.world().resource::<PartyInventory>().held.is_none());
            assert_eq!(
                app.world()
                    .get::<Carried>(member)
                    .unwrap()
                    .backpack
                    .iter()
                    .flatten()
                    .map(|i| i.id)
                    .collect::<Vec<_>>(),
                if still_a_member { vec![id] } else { vec![] }
            );
            assert_eq!(
                app.world()
                    .resource::<PartyInventory>()
                    .pool
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>(),
                if still_a_member { vec![] } else { vec![id] }
            );
        }
    }

    #[test]
    fn gold_is_grouped_in_thousands() {
        assert_eq!(format_gold(0), "0");
        assert_eq!(format_gold(950), "950");
        assert_eq!(format_gold(12345), "12,345");
        assert_eq!(format_gold(1234567), "1,234,567");
    }
}
