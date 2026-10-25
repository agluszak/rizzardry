//! The character review screen's item page (`docs/research/review-items.md`):
//! a member's equipment on their race's statue, their backpack, and the
//! party's pool. Opened with I for the selected member; the header's portraits
//! switch member, Esc, I or the Exit button closes it. Retail fills empty
//! cells with a PCX texture; a flat colour stands in for it.
//!
//! The screen is one scene with a node for each cell, the header and the
//! gold; each draws itself again when what it shows changes.

use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

use super::screen_art::{absolute, centred, rect, screen, sprite, text};
use crate::content::items::{ItemCatalog, ItemDefinition, QuantityKind};
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Gender, Race};
use crate::game::data::GameData;
use crate::game::inventory::{
    BACKPACK_SLOTS, Carried, EquipSlot, ItemInstance, ItemLocation, PartyInventory, Quantity,
    Refusal, Slot,
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
#[derive(Resource, Debug)]
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
    mut commands: Commands,
    mut screen: ResMut<Screen>,
    roster: Res<PartyRoster>,
    selected: Res<super::main_screen::SelectedMember>,
) {
    if let Some(member) = selected.member(&roster) {
        open(&mut commands, &mut screen, member);
    }
}

/// Bring up a member's inventory, unless another screen is up.
pub fn open(commands: &mut Commands, screen: &mut Screen, member: Entity) {
    if screen.open(Screen::Inventory) {
        commands.insert_resource(InventoryScreen { member, pool_scroll: 0 });
    }
}

/// Close the screen, first putting away whatever is in hand.
fn close(mut commands: Commands, mut screen: ResMut<Screen>) {
    commands.run_system_cached(put_away_held);
    commands.remove_resource::<InventoryScreen>();
    screen.close(Screen::Inventory);
}

/// The held item goes back to the reviewed member, or the pool.
fn put_away_held(
    screen: Option<Res<InventoryScreen>>,
    mut party: ResMut<PartyInventory>,
    mut members: Query<&mut Carried>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
) {
    let (Some(screen), Some(catalog)) = (screen, data.and_then(|d| catalogs.get(&d.items))) else {
        return;
    };
    if let Ok(mut carried) = members.get_mut(screen.member) {
        party.put_away_held(&mut carried, catalog);
    }
}

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

/// A cell's node, with what it last drew and the sheets that took.
#[derive(Component)]
struct ItemCell {
    cell: Cell,
    drawn: Option<CellView>,
    _kept: Vec<Handle<SpriteSheet>>,
}

/// What a cell shows.
#[derive(Clone, Debug, PartialEq)]
struct CellView {
    item: Option<ShownItem>,
    /// The member cannot use what is in it (backpack and pool).
    unusable: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct ShownItem {
    picture: String,
    count: Option<String>,
    identified: bool,
}

fn shown_item(item: &ItemInstance, definition: &ItemDefinition) -> ShownItem {
    ShownItem {
        picture: definition.image.clone(),
        count: count_label(item, definition.quantity),
        identified: item.identified,
    }
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

/// The header (the party's small portraits, the reviewed member's
/// portrait, name, race and profession) and their statue.
#[derive(Component, Clone, Default)]
struct Header {
    drawn: Option<HeaderView>,
}

#[derive(Clone, Debug, PartialEq)]
struct HeaderView {
    /// Each member and their portrait, in party order.
    party: Vec<(Entity, String)>,
    portrait: String,
    name: String,
    race: Race,
    profession: String,
    statue: String,
}

#[derive(Component, Clone, Default)]
struct GoldLabel {
    drawn: Option<u32>,
}

/// Build the screen when it opens, once its fixed art has loaded; remove
/// it when it closes.
pub fn spawn_inventory_screen(
    mut commands: Commands,
    open: Option<Res<InventoryScreen>>,
    art: Res<InventoryArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    roots: Query<Entity, With<InventoryRoot>>,
) {
    if open.is_none() {
        for root in &roots {
            commands.entity(root).despawn();
        }
        return;
    }
    if !roots.is_empty() {
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
        .map(|cell| {
            let area = cell.area();
            let node = absolute(area.min, area.size() + 1.0);
            bsn! {
                node
                template(move |_| Ok(ItemCell { cell, drawn: None, _kept: vec![] }))
                on(click_cell)
            }
        })
        .collect();
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
        on(|_: On<Start<CloseInventory>>, mut commands: Commands| {
            commands.run_system_cached(close);
        })
        Children [
            @screen()
            Children [
                {fixed}
                -- {titles}
                -- @{exit}
                    Pickable { should_block_lower: true, is_hoverable: true }
                    on(|_: On<PointerClick>, mut commands: Commands| {
                        commands.run_system_cached(close);
                    })
                // The header and statue lie under the cells.
                -- Node { position_type: PositionType::Absolute } Header
                -- Node { position_type: PositionType::Absolute } GoldLabel
                -- {cells}
            ]
        ]
    });
}

/// Draw the header and statue again when the reviewed member or the party
/// changes.
fn draw_header(
    mut commands: Commands,
    screen: Option<Res<InventoryScreen>>,
    art: Res<InventoryArt>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    roster: Res<PartyRoster>,
    members: Query<&Character>,
    mut headers: Query<(Entity, &mut Header)>,
    mut kept: Local<Vec<Handle<SpriteSheet>>>,
) {
    let (Some(screen), Ok((node, mut header))) = (screen, headers.single_mut()) else { return };
    let Ok(member) = members.get(screen.member) else { return };
    let view = HeaderView {
        party: roster
            .0
            .iter()
            .filter_map(|&m| Some((m, members.get(m).ok()?.portrait.clone())))
            .collect(),
        portrait: member.portrait.clone(),
        name: member.name.clone(),
        race: member.race,
        profession: format!("{:?}", member.profession),
        statue: statue(member.race, member.gender),
    };
    if header.drawn.as_ref() == Some(&view) {
        return;
    }
    let (Some(corner), Some(font), Some(bold)) =
        (sheets.get(&art.corner), sheets.get(&art.font), sheets.get(&art.bold))
    else {
        return;
    };
    let mut complete = true;
    let mut held = vec![];
    let mut sheet = |path: String| {
        let handle = assets.load(format!("retail://{path}"));
        let loaded = sheets.get(&handle);
        complete &= loaded.is_some();
        held.push(handle);
        loaded
    };
    let mut parts: Vec<Box<dyn Scene>> = vec![];
    // The party's cells switch member; the rest are empty.
    for (slot, (member, portrait)) in view.party.iter().enumerate() {
        let at = Vec2::new(6.0 + (slot % 2) as f32 * 48.0, 5.0 + (slot / 2) as f32 * 39.0);
        let picture = sheet(format!("data/portraits/small/s{portrait}.sti"));
        let member = *member;
        parts.push(Box::new(bsn! {
            @{picture.and_then(|s| sprite(s, 0, at))}
            Pickable { should_block_lower: true, is_hoverable: true }
            on(move |_: On<PointerClick>, screen: Option<ResMut<InventoryScreen>>| {
                if let Some(mut screen) = screen {
                    screen.member = member;
                }
            })
        }));
    }
    for slot in view.party.len()..8 {
        let at = Vec2::new(5.0 + (slot % 2) as f32 * 48.0, 4.0 + (slot / 2) as f32 * 39.0);
        parts.push(Box::new(sprite(corner, slot + 1, at)));
    }
    let portrait = sheet(format!("data/portraits/medium/m{}.sti", view.portrait));
    parts.push(Box::new(portrait.and_then(|s| sprite(s, 0, Vec2::new(164.0, 12.0)))));
    parts.push(Box::new(sprite(corner, 9, Vec2::new(164.0, 12.0))));
    parts.push(Box::new(text(bold, &mut images, &view.name, Vec2::new(116.0, 96.0), 186.0)));
    let race = format!("{:?}", view.race);
    parts.push(Box::new(text(font, &mut images, &race, Vec2::new(116.0, 111.0), 186.0)));
    parts.push(Box::new(text(font, &mut images, &view.profession, Vec2::new(116.0, 124.0), 186.0)));
    let statue = sheet(format!("data/review/{}.sti", view.statue));
    parts.push(Box::new(statue.and_then(|s| sprite(s, 0, Vec2::new(194.0, 165.0)))));
    commands
        .entity(node)
        .despawn_related::<Children>()
        .queue_spawn_related_scenes::<Children>(parts);
    *kept = held;
    header.drawn = complete.then_some(view);
}

fn draw_gold(
    mut commands: Commands,
    art: Res<InventoryArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    party: Res<PartyInventory>,
    mut labels: Query<(Entity, &mut GoldLabel)>,
) {
    let (Ok((node, mut label)), Some(font)) = (labels.single_mut(), sheets.get(&art.font)) else {
        return;
    };
    if label.drawn == Some(party.gold) {
        return;
    }
    let gold = text(font, &mut images, &format_gold(party.gold), Vec2::new(553.0, 425.0), 48.0);
    commands
        .entity(node)
        .despawn_related::<Children>()
        .queue_spawn_related_scenes::<Children>(vec![gold]);
    label.drawn = Some(party.gold);
}

/// Draw each cell again when what it shows changes: the item, its count
/// and identification, and whether the reviewed member can use it.
fn draw_cells(
    mut commands: Commands,
    screen: Option<Res<InventoryScreen>>,
    art: Res<InventoryArt>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    members: Query<(&Character, &Carried)>,
    party: Res<PartyInventory>,
    mut cells: Query<(Entity, &mut ItemCell)>,
) {
    let Some(screen) = screen else { return };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    let Ok((character, carried)) = members.get(screen.member) else { return };
    let get = |handle: &Handle<SpriteSheet>| sheets.get(handle);
    let (Some(font), Some(highlights), Some(unusable), Some(unidentified)) =
        (get(&art.font), get(&art.highlights), get(&art.unusable), get(&art.unidentified))
    else {
        return;
    };
    for (node, mut cell) in &mut cells {
        let item = match cell.cell {
            Cell::Member(Slot::Equipment(slot)) => carried.worn(slot),
            Cell::Member(Slot::Backpack(i)) => carried.backpack[i].as_ref(),
            Cell::Pool(i) => party.pool.get(screen.pool_scroll + i),
        };
        let definition = item.and_then(|item| Some((item, catalog.get(item.item)?)));
        let view = CellView {
            item: definition.map(|(item, definition)| shown_item(item, definition)),
            unusable: definition.is_some_and(|(_, definition)| !character.can_use(definition)),
        };
        if cell.drawn.as_ref() == Some(&view) {
            continue;
        }
        let mut complete = true;
        let mut kept = vec![];
        let mut parts: Vec<Box<dyn Scene>> = vec![];
        let area = cell.cell.area();
        let size = area.size();
        // Equipment shows the item's large picture over the statue's own
        // well; the backpack and pool show it on a cell.
        let (picture_frame, picture_at, count_at, count_width) = match cell.cell {
            Cell::Member(Slot::Equipment(_)) => {
                (1, Vec2::ZERO, Vec2::new(2.0, size.y - 13.0), size.x - 4.0)
            }
            _ => {
                if view.unusable {
                    parts.push(Box::new(sprite(unusable, 0, Vec2::ZERO)));
                } else {
                    parts.push(Box::new(rect(Vec2::ZERO, CELL, FILL)));
                }
                (0, Vec2::ONE, Vec2::new(1.0, 40.0), 44.0)
            }
        };
        if let Some(item) = &view.item {
            let handle = assets.load(format!("retail://{}", item.picture));
            let picture = sheets.get(&handle);
            complete &= picture.is_some();
            kept.push(handle);
            parts.push(Box::new(picture.and_then(|s| sprite(s, picture_frame, picture_at))));
            if let Some(count) = &item.count {
                // Right-aligned.
                let width = font.text_width(count) as f32;
                let at = Vec2::new(count_at.x + count_width - width, count_at.y);
                parts.push(Box::new(text(font, &mut images, count, at, 0.0)));
            }
            if !item.identified {
                parts.push(Box::new(sprite(unidentified, 0, Vec2::ZERO)));
            }
            if let Cell::Member(Slot::Equipment(slot)) = cell.cell {
                parts.push(Box::new(sprite(
                    highlights,
                    equipment_rim(slot) + 3,
                    -Vec2::splat(2.0),
                )));
            }
        }
        commands
            .entity(node)
            .despawn_related::<Children>()
            .queue_spawn_related_scenes::<Children>(parts);
        cell._kept = kept;
        cell.drawn = complete.then_some(view);
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
    screen: Option<Res<InventoryScreen>>,
    roster: Res<PartyRoster>,
    mut members: Query<(&Character, &mut Carried)>,
    mut party: ResMut<PartyInventory>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
) {
    let (Ok(cell), Some(screen)) = (cells.get(click.entity), screen) else { return };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    let held = party.held.as_ref().map(|h| h.item.clone());
    let result = match cell.cell {
        Cell::Pool(i) => {
            let row = screen.pool_scroll + i;
            if held.is_some() {
                let row = row.min(party.pool.len());
                party.deposit_held(row, catalog)
            } else {
                party.take_from_pool(row)
            }
        }
        Cell::Member(slot) => {
            if !roster.0.contains(&screen.member) {
                return;
            }
            let Ok((character, mut carried)) = members.get_mut(screen.member) else { return };
            let target = match slot {
                Slot::Equipment(equip) => carried.worn(equip).cloned(),
                Slot::Backpack(i) => carried.backpack[i].clone(),
            };
            match (&held, &target) {
                (Some(held), Some(target))
                    if held.item == target.item && matches!(held.quantity, Quantity::Stack(_)) =>
                {
                    let location = ItemLocation::Member(screen.member, slot);
                    match party.merge_held(Some(&mut carried), location, catalog) {
                        0 => Err(Refusal::DoesNotFit),
                        _ => Ok(()),
                    }
                }
                _ => party.swap_with_hand(character, &mut carried, screen.member, slot, catalog),
            }
        }
    };
    if let Err(refusal) = result {
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

    #[test]
    fn gold_is_grouped_in_thousands() {
        assert_eq!(format_gold(0), "0");
        assert_eq!(format_gold(950), "950");
        assert_eq!(format_gold(12345), "12,345");
        assert_eq!(format_gold(1234567), "1,234,567");
    }
}
