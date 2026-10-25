//! The character review screen's item page (`docs/research/review-items.md`):
//! a member's equipment on their race's statue, their backpack, and the
//! party's pool. Opened with I for the selected member; the header's portraits
//! switch member, Esc or the Exit button closes it. Retail fills empty cells
//! with a PCX texture; a flat colour stands in for it.

use std::collections::HashMap;

use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;

use super::screen_art::{absolute, sprite, text};
use crate::content::items::{ItemCatalog, QuantityKind};
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Gender, Race};
use crate::game::data::GameData;
use crate::game::inventory::{
    BACKPACK_SLOTS, Carried, EquipSlot, ItemInstance, ItemLocation, PartyInventory, Quantity,
    Refusal, Slot,
};
use crate::game::party::roster::PartyRoster;

const SCREEN: Vec2 = Vec2::new(640.0, 480.0);
const CELL: Vec2 = Vec2::new(46.0, 54.0);
const FILL: Color = Color::srgb(0.16, 0.14, 0.11);
/// Pool cells shown at once.
const POOL_CELLS: usize = 8;

/// Equipment cells on the paper doll, by equipment slot: position and size.
const EQUIPMENT_CELLS: [(f32, f32, f32, f32); 12] = [
    (188.0, 176.0, 45.0, 57.0),
    (431.0, 198.0, 40.0, 40.0),
    (431.0, 241.0, 40.0, 40.0),
    (376.0, 176.0, 38.0, 73.0),
    (134.0, 241.0, 57.0, 58.0),
    (387.0, 257.0, 35.0, 43.0),
    (158.0, 308.0, 34.0, 101.0),
    (400.0, 308.0, 34.0, 72.0),
    (121.0, 308.0, 34.0, 101.0),
    (437.0, 308.0, 34.0, 72.0),
    (203.0, 359.0, 34.0, 77.0),
    (362.0, 378.0, 26.0, 58.0),
];

/// The open screen, reviewing one member. Absent while exploring.
#[derive(Resource, Debug)]
pub struct InventoryScreen {
    pub member: Entity,
    /// The first pool entry shown; always even.
    pub pool_scroll: usize,
    /// What the cells last showed, to draw them again when it changes;
    /// none until every picture they need has loaded.
    drawn: Option<Shown>,
}

impl InventoryScreen {
    pub fn new(member: Entity) -> Self {
        InventoryScreen { member, pool_scroll: 0, drawn: None }
    }
}

/// What the cells show.
#[derive(Debug, PartialEq)]
struct Shown {
    member: Entity,
    pool_scroll: usize,
    carried: Carried,
    pool: Vec<ItemInstance>,
    gold: u32,
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

/// The whole screen.
#[derive(Component)]
pub struct InventoryRoot;

/// The part drawn again when the inventory changes.
#[derive(Component)]
pub struct InventoryCells;

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

pub fn toggle_inventory(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    screen: Option<Res<InventoryScreen>>,
    automap: Option<Res<super::automap::Automap>>,
    roster: Res<PartyRoster>,
    selected: Res<super::main_screen::SelectedMember>,
    roots: Query<Entity, With<InventoryRoot>>,
) {
    match screen {
        Some(_) if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyI) => {
            close(&mut commands, &roots);
        }
        None if keys.just_pressed(KeyCode::KeyI) && automap.is_none() => {
            // Retail opens the selected character.
            if let Some(&member) = roster.0.get(selected.0).or(roster.0.first()) {
                commands.insert_resource(InventoryScreen::new(member));
            }
        }
        _ => {}
    }
}

/// Close the screen (the Exit button).
fn exit(_: On<PointerClick>, mut commands: Commands, roots: Query<Entity, With<InventoryRoot>>) {
    close(&mut commands, &roots);
}

/// Close the screen, first putting away whatever is in hand.
fn close(commands: &mut Commands, roots: &Query<Entity, With<InventoryRoot>>) {
    commands.run_system_cached(put_away_held);
    commands.remove_resource::<InventoryScreen>();
    for root in roots {
        commands.entity(root).despawn();
    }
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

/// A place an item can be clicked in or out of.
#[derive(Component, Clone, Copy, Debug)]
enum Cell {
    Member(Slot),
    /// A pool cell by its position on screen.
    Pool(usize),
}

/// Click a cell (retail's item-page click, `rcs.cpp:1459-1742`): with an
/// empty hand, pick the item up; holding one, a pool cell takes it in, a
/// stack of the same item takes what it can, anything else swaps.
/// Shift-clicking a single unit, pouring ammunition and stowing a weapon's
/// partner are not done yet.
fn click_cell(
    click: On<PointerClick>,
    cells: Query<&Cell>,
    screen: Option<Res<InventoryScreen>>,
    roster: Res<PartyRoster>,
    mut members: Query<(&Character, &mut Carried)>,
    mut party: ResMut<PartyInventory>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
) {
    let (Ok(&cell), Some(screen)) = (cells.get(click.entity), screen) else { return };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    let held = party.held.as_ref().map(|h| h.item.clone());
    let result = match cell {
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
            let Some(index) = roster.0.iter().position(|&m| m == screen.member) else { return };
            let Ok((character, mut carried)) = members.get_mut(screen.member) else { return };
            let target = match slot {
                Slot::Equipment(equip) => carried.worn(equip).cloned(),
                Slot::Backpack(i) => carried.backpack[i].clone(),
            };
            match (&held, &target) {
                (Some(held), Some(target))
                    if held.item == target.item && matches!(held.quantity, Quantity::Stack(_)) =>
                {
                    let location = ItemLocation::Member(index, slot);
                    match party.merge_held(Some(&mut carried), location, catalog) {
                        0 => Err(Refusal::DoesNotFit),
                        _ => Ok(()),
                    }
                }
                _ => {
                    let can_use = held
                        .as_ref()
                        .and_then(|h| catalog.get(h.item))
                        .is_none_or(|d| character.can_use(d));
                    party.swap_with_hand(&mut carried, index, slot, can_use, catalog)
                }
            }
        }
    };
    if let Err(refusal) = result {
        info!("inventory: {cell:?} refused: {refusal:?}");
    }
}

/// The held item, under the pointer.
#[derive(Component)]
pub struct HeldPicture;

/// Show what the hand holds at the pointer, here and on the main screen
/// (retail's item cursor).
pub fn follow_held(
    mut commands: Commands,
    party: Res<PartyInventory>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    scale: Res<UiScale>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    mut pictures: Query<(Entity, &mut ImageNode, &mut Node), With<HeldPicture>>,
    mut loading: Local<Option<Handle<SpriteSheet>>>,
) {
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    let definition = party.held.as_ref().zip(catalog).and_then(|(h, c)| c.get(h.item.item));
    let cursor = pointers
        .iter()
        .find(|(id, _)| **id == PointerId::Mouse)
        .and_then(|(_, location)| location.location.as_ref())
        .map(|location| location.position);
    let (Some(definition), Some(cursor)) = (definition, cursor) else {
        for (entity, ..) in &pictures {
            commands.entity(entity).despawn();
        }
        return;
    };
    let handle = loading.insert(assets.load(format!("retail://{}", definition.image)));
    let Some((image, frame)) =
        sheets.get(&*handle).and_then(|s| Some((s.images.first()?, s.frames.first()?)))
    else {
        return;
    };
    let size = Vec2::new(frame.width as f32, frame.height as f32);
    let at = cursor / scale.0 - size / 2.0;
    match pictures.single_mut() {
        Ok((_, mut node_image, mut node)) => {
            node_image.image = image.clone();
            *node = absolute(at, size);
        }
        Err(_) => {
            commands.spawn((
                HeldPicture,
                ImageNode::new(image.clone()),
                absolute(at, size),
                GlobalZIndex(20),
                Pickable::IGNORE,
            ));
        }
    }
}

/// Review another member (a header portrait).
#[derive(Component)]
struct ReviewMember(Entity);

fn review_member(
    click: On<PointerClick>,
    targets: Query<&ReviewMember>,
    screen: Option<ResMut<InventoryScreen>>,
) {
    if let (Ok(target), Some(mut screen)) = (targets.get(click.entity), screen) {
        screen.member = target.0;
    }
}

/// Build the screen's fixed art once its sheets have loaded.
pub fn spawn_inventory_screen(
    mut commands: Commands,
    screen: Option<Res<InventoryScreen>>,
    art: Res<InventoryArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    roots: Query<(), With<InventoryRoot>>,
) {
    if screen.is_none() || !roots.is_empty() {
        return;
    }
    let get = |handle: &Handle<SpriteSheet>| sheets.get(handle);
    let (Some(corner), Some(page), Some(bar), Some(gold), Some(exit_button), Some(bold)) = (
        get(&art.corner),
        get(&art.page),
        get(&art.bottom_bar),
        get(&art.gold_icon),
        get(&art.exit),
        get(&art.bold),
    ) else {
        return;
    };
    let root = commands
        .spawn((
            InventoryRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(10),
        ))
        .id();
    let frame = commands
        .spawn((
            Node { width: Val::Px(SCREEN.x), height: Val::Px(SCREEN.y), ..default() },
            ChildOf(root),
        ))
        .id();
    let layers = [
        sprite(corner, 0, Vec2::ZERO),
        sprite(page, 0, Vec2::new(310.0, 0.0)),
        sprite(page, 1, Vec2::new(0.0, 165.0)),
        sprite(page, 2, Vec2::new(113.0, 165.0)),
        sprite(page, 3, Vec2::new(478.0, 165.0)),
        sprite(gold, 0, Vec2::new(514.0, 422.0)),
        sprite(bar, 0, Vec2::new(0.0, 450.0)),
    ];
    for layer in layers.into_iter().flatten() {
        commands.spawn((layer, ChildOf(frame)));
    }
    commands
        .spawn((text(bold, &mut images, "Carried", Vec2::new(12.0, 174.0), 93.0), ChildOf(frame)));
    commands.spawn((
        text(bold, &mut images, "Party Items", Vec2::new(486.0, 174.0), 141.0),
        ChildOf(frame),
    ));
    if let Some((image, node, _)) = sprite(exit_button, 16, Vec2::new(596.0, 450.0)) {
        commands.spawn((image, node, ChildOf(frame))).observe(exit);
    }
    commands.spawn((
        InventoryCells,
        // Sizeless, so it catches no clicks itself; its cells overflow it.
        absolute(Vec2::ZERO, Vec2::ZERO),
        ChildOf(frame),
    ));
}

/// Draw the reviewed member's header, statue and items, and the pool.
pub fn draw_inventory_cells(
    mut commands: Commands,
    screen: Option<ResMut<InventoryScreen>>,
    art: Res<InventoryArt>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    roster: Res<PartyRoster>,
    members: Query<(&Character, &Carried)>,
    party: Res<PartyInventory>,
    layers: Query<Entity, With<InventoryCells>>,
    mut loading: Local<HashMap<String, Handle<SpriteSheet>>>,
) {
    let Some(mut screen) = screen else { return };
    let Ok(layer) = layers.single() else { return };
    let get = |handle: &Handle<SpriteSheet>| sheets.get(handle);
    let (
        Some(font),
        Some(bold),
        Some(corner),
        Some(highlights),
        Some(unusable),
        Some(unidentified),
    ) = (
        get(&art.font),
        get(&art.bold),
        get(&art.corner),
        get(&art.highlights),
        get(&art.unusable),
        get(&art.unidentified),
    )
    else {
        return;
    };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    let Ok((character, carried)) = members.get(screen.member) else { return };
    let shown = Shown {
        member: screen.member,
        pool_scroll: screen.pool_scroll,
        carried: carried.clone(),
        pool: party.pool.iter().skip(screen.pool_scroll).take(POOL_CELLS).cloned().collect(),
        gold: party.gold,
    };
    if screen.drawn.as_ref() == Some(&shown) {
        return;
    }
    let mut complete = true;
    // Sheets loaded here are kept, or dropping the handle would cancel them.
    let mut sheet = |path: String| {
        let handle =
            loading.entry(path).or_insert_with_key(|path| assets.load(format!("retail://{path}")));
        let loaded = sheets.get(&*handle);
        complete &= loaded.is_some();
        loaded
    };
    commands.entity(layer).despawn_related::<Children>();

    // Header: party cells, the big portrait, name, race and profession.
    for (slot, &member) in roster.0.iter().enumerate() {
        let Ok((other, _)) = members.get(member) else { continue };
        let at = Vec2::new(6.0 + (slot % 2) as f32 * 48.0, 5.0 + (slot / 2) as f32 * 39.0);
        let path = format!("data/portraits/small/s{}.sti", other.portrait);
        if let Some(portrait) = sheet(path).and_then(|s| sprite(s, 0, at)) {
            commands
                .spawn((portrait.0, portrait.1, ReviewMember(member), ChildOf(layer)))
                .observe(review_member);
        }
    }
    for slot in roster.0.len()..8 {
        let at = Vec2::new(5.0 + (slot % 2) as f32 * 48.0, 4.0 + (slot / 2) as f32 * 39.0);
        if let Some(empty) = sprite(corner, slot + 1, at) {
            put(&mut commands, layer, empty);
        }
    }
    if let Some(portrait) = sheet(format!("data/portraits/medium/m{}.sti", character.portrait))
        .and_then(|s| sprite(s, 0, Vec2::new(164.0, 12.0)))
    {
        put(&mut commands, layer, portrait);
    }
    if let Some(overlay) = sprite(corner, 9, Vec2::new(164.0, 12.0)) {
        put(&mut commands, layer, overlay);
    }
    put(
        &mut commands,
        layer,
        text(bold, &mut images, &character.name, Vec2::new(116.0, 96.0), 186.0),
    );
    put(
        &mut commands,
        layer,
        text(font, &mut images, &format!("{:?}", character.race), Vec2::new(116.0, 111.0), 186.0),
    );
    put(
        &mut commands,
        layer,
        text(
            font,
            &mut images,
            &format!("{:?}", character.profession),
            Vec2::new(116.0, 124.0),
            186.0,
        ),
    );
    let statue_path = format!("data/review/{}.sti", statue(character.race, character.gender));
    if let Some(statue) = sheet(statue_path).and_then(|s| sprite(s, 0, Vec2::new(194.0, 165.0))) {
        put(&mut commands, layer, statue);
    }
    put(
        &mut commands,
        layer,
        text(font, &mut images, &format_gold(party.gold), Vec2::new(553.0, 425.0), 48.0),
    );

    // Equipment on the statue.
    for (index, slot) in EquipSlot::ALL.iter().enumerate() {
        let (x, y, w, h) = EQUIPMENT_CELLS[index];
        let at = Vec2::new(x, y);
        // The doll's own wells show through empty cells.
        let Some(item) = carried.worn(*slot) else { continue };
        let Some(definition) = catalog.get(item.item) else { continue };
        if let Some(picture) = sheet(definition.image.clone()).and_then(|s| sprite(s, 1, at)) {
            put(&mut commands, layer, picture);
        }
        quantity(
            &mut commands,
            layer,
            font,
            &mut images,
            item,
            definition.quantity,
            Vec2::new(x + 2.0, y + h - 13.0),
            w - 4.0,
        );
        if !item.identified
            && let Some(overlay) = sprite(unidentified, 0, at)
        {
            put(&mut commands, layer, overlay);
        }
        let rim = [0, 40, 36, 44, 4, 32, 12, 24, 8, 28, 16, 20][index];
        if let Some(rim) = sprite(highlights, rim + 3, at - Vec2::splat(2.0)) {
            put(&mut commands, layer, rim);
        }
    }

    // Backpack and pool cells.
    let grid = |left: f32, i: usize| {
        Vec2::new(left + (i % 2) as f32 * 49.0, 192.0 + (i / 2) as f32 * 57.0)
    };
    let pool = party.pool.iter().skip(screen.pool_scroll).take(POOL_CELLS).map(Some);
    let cells = carried
        .backpack
        .iter()
        .take(BACKPACK_SLOTS)
        .map(Option::as_ref)
        .enumerate()
        .map(|(i, item)| (grid(11.0, i), item))
        .chain(
            pool.chain(std::iter::repeat(None))
                .take(POOL_CELLS)
                .enumerate()
                .map(|(i, item)| (grid(512.0, i), item)),
        );
    for (at, item) in cells {
        let definition = item.and_then(|i| catalog.get(i.item));
        match definition {
            Some(definition) if !character.can_use(definition) => {
                if let Some(background) = sprite(unusable, 0, at) {
                    put(&mut commands, layer, background);
                }
            }
            _ => {
                commands.spawn((
                    absolute(at, CELL),
                    BackgroundColor(FILL),
                    Pickable::IGNORE,
                    ChildOf(layer),
                ));
            }
        }
        let (Some(item), Some(definition)) = (item, definition) else { continue };
        if let Some(picture) =
            sheet(definition.image.clone()).and_then(|s| sprite(s, 0, at + Vec2::ONE))
        {
            put(&mut commands, layer, picture);
        }
        quantity(
            &mut commands,
            layer,
            font,
            &mut images,
            item,
            definition.quantity,
            at + Vec2::new(1.0, 40.0),
            44.0,
        );
        if !item.identified
            && let Some(overlay) = sprite(unidentified, 0, at)
        {
            put(&mut commands, layer, overlay);
        }
    }
    // Clickable cells over everything else.
    let cells = EquipSlot::ALL
        .iter()
        .zip(EQUIPMENT_CELLS)
        .map(|(&slot, (x, y, w, h))| {
            (Cell::Member(Slot::Equipment(slot)), Vec2::new(x, y), Vec2::new(w + 1.0, h + 1.0))
        })
        .chain(
            (0..BACKPACK_SLOTS)
                .map(|i| (Cell::Member(Slot::Backpack(i)), grid(11.0, i), CELL + 1.0)),
        )
        .chain((0..POOL_CELLS).map(|i| (Cell::Pool(i), grid(512.0, i), CELL + 1.0)));
    for (cell, at, size) in cells {
        commands.spawn((cell, absolute(at, size), ChildOf(layer))).observe(click_cell);
    }
    screen.drawn = complete.then_some(shown);
}

fn put(commands: &mut Commands, layer: Entity, part: (ImageNode, Node, Pickable)) {
    commands.spawn((part, ChildOf(layer)));
}

/// A cell's count, right-aligned: a stack above one, charges or shots.
fn quantity(
    commands: &mut Commands,
    layer: Entity,
    font: &SpriteSheet,
    images: &mut Assets<Image>,
    item: &ItemInstance,
    kind: QuantityKind,
    at: Vec2,
    width: f32,
) {
    let count = match item.quantity {
        Quantity::One => return,
        Quantity::Stack(n) | Quantity::Uses(n) => n,
    };
    let label = match kind {
        QuantityKind::None | QuantityKind::Unlabelled => return,
        QuantityKind::Stack if count <= 1 => return,
        QuantityKind::Charges | QuantityKind::Uses if !item.identified => "?".to_owned(),
        _ => count.to_string(),
    };
    let image = font.text_image(&label);
    let size = image.size_f32();
    put(
        commands,
        layer,
        (
            ImageNode::new(images.add(image)),
            absolute(Vec2::new(at.x + width - size.x, at.y), size),
            Pickable::IGNORE,
        ),
    );
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
