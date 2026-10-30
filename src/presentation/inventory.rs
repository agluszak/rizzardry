//! Character review: equipment, statistics and skills. The item page shows
//! a member's equipment on their race's statue, their backpack, and the
//! party's pool. Opened with I for the selected member; the header's portraits
//! switch member, Esc, I or the Exit button closes it.
//!
//! The screen is one scene with a node for each cell, the header and the
//! gold; their retained nodes update when their gameplay inputs change.

mod info;
mod item_info;
mod pool_scroll;
mod progression;
mod review;
mod spells;

use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::picking::{
    hover::Hovered,
    pointer::{PointerId, PointerLocation},
};
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button as UiButton};
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

use super::screen_art::{
    BitmapText, Picture, absolute, centred, empty_hands, label, picture, rect, screen, sprite,
};
use crate::content::items::QuantityKind;
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Gender, Race, Vitals};
use crate::game::combat::attack::Armour;
use crate::game::inventory::{
    BACKPACK_SLOTS, Carried, EquipSlot, ItemAt, ItemStack, MemberSlot, PartyInventory, Slot,
};
use crate::game::party::roster::PartyRoster;
use crate::presentation::screen::Screen;
use crate::presentation::screen_art::{show_sheet, visible_if};

const CELL: Vec2 = Vec2::new(46.0, 54.0);
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
#[require(ItemAction)]
pub struct InventoryScreen {
    pub member: Entity,
}

/// What clicking an item does besides moving it (retail's item action
/// buttons, `SetCampItemActionMode`): armed by its button, spent by the
/// click that carries it out.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemAction {
    #[default]
    None,
    /// The party tries to identify the clicked item.
    Identify,
    /// The held item is merged with the clicked one.
    Merge,
}

/// An item action's button.
#[derive(Component, Clone, Copy)]
struct ItemActionButton(ItemAction);

/// An item action button arms its action, or disarms it when armed; with an
/// item held, Identify tries it at once (retail `OnCampIdentifyItem`,
/// `OnCampMoveItem`).
fn press_item_action(
    click: On<Activate>,
    buttons: Query<&ItemActionButton>,
    screen: Option<Single<(&InventoryScreen, &mut ItemAction)>>,
    mut inventory: crate::game::inventory::operations::InventoryOperations,
    mut rng: ResMut<crate::game::rng::GameRng>,
) {
    let (Ok(&ItemActionButton(pressed)), Some(screen)) = (buttons.get(click.entity), screen) else {
        return;
    };
    let (screen, mut action) = screen.into_inner();
    *action = if *action == pressed {
        ItemAction::None
    } else if pressed == ItemAction::Identify && inventory.holds() {
        if let Err(refusal) = inventory.identify(screen.member, ItemAt::Hand, &mut rng) {
            info!("identifying refused: {refusal:?}");
        }
        ItemAction::None
    } else {
        pressed
    };
}

/// The armed action's button shows pressed.
fn style_item_actions(
    action: Option<Single<&ItemAction>>,
    buttons: Query<(Entity, &ItemActionButton, Has<bevy::ui::Selected>)>,
    mut commands: Commands,
) {
    let armed = action.map_or(ItemAction::None, |action| **action);
    for (entity, button, selected) in &buttons {
        let active = button.0 == armed;
        if active != selected {
            if active {
                commands.entity(entity).insert(bevy::ui::Selected);
            } else {
                commands.entity(entity).remove::<bevy::ui::Selected>();
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct InventoryArt {
    fill: Handle<Image>,
    corner: Handle<SpriteSheet>,
    portrait_highlight: Handle<SpriteSheet>,
    level_up: Handle<SpriteSheet>,
    page: Handle<SpriteSheet>,
    bottom_bar: Handle<SpriteSheet>,
    page_buttons: Handle<SpriteSheet>,
    item_buttons: Handle<SpriteSheet>,
    stats_page: Handle<SpriteSheet>,
    magic_page: Handle<SpriteSheet>,
    skills_page: Handle<SpriteSheet>,
    skills_icons: Handle<SpriteSheet>,
    slider: Handle<SpriteSheet>,
    combat_panel: Handle<SpriteSheet>,
    combat_buttons: Handle<SpriteSheet>,
    effect_filters: Handle<SpriteSheet>,
    scroll: Handle<SpriteSheet>,
    column: Handle<SpriteSheet>,
    health: Handle<SpriteSheet>,
    stamina: Handle<SpriteSheet>,
    left_plug: Handle<SpriteSheet>,
    right_plug: Handle<SpriteSheet>,
    condition_rim: Handle<SpriteSheet>,
    gold_icon: Handle<SpriteSheet>,
    highlights: Handle<SpriteSheet>,
    unusable: Handle<SpriteSheet>,
    unidentified: Handle<SpriteSheet>,
    exit: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    bold: Handle<SpriteSheet>,
    small_font: Handle<SpriteSheet>,
    green: Handle<SpriteSheet>,
    yellow: Handle<SpriteSheet>,
    gray: Handle<SpriteSheet>,
    beige: Handle<SpriteSheet>,
    blue: Handle<SpriteSheet>,
    bronze: Handle<SpriteSheet>,
    red: Handle<SpriteSheet>,
    leveling_bar: Handle<SpriteSheet>,
    realms: [Handle<SpriteSheet>; 6],
    info_dialog: Handle<SpriteSheet>,
    info_buttons: Handle<SpriteSheet>,
}

pub fn load_inventory_art(mut commands: Commands, assets: Res<AssetServer>) {
    let load = |path: &str| assets.load(format!("retail://data/{path}"));
    commands.insert_resource(InventoryArt {
        fill: assets.load("retail://data/main interface/basic_fill_texture.pcx"),
        corner: load("review/commoncorner.sti"),
        portrait_highlight: load("review/smallportraithighlight.sti"),
        level_up: load("main interface/leveling_button.sti"),
        page: load("review/reviewitempage.sti"),
        bottom_bar: load("review/bottombuttonbar.sti"),
        page_buttons: load("review/reviewpagebuttons.sti"),
        item_buttons: load("review/reviewitembuttons.sti"),
        stats_page: load("review/reviewstatspage.sti"),
        magic_page: load("review/reviewmagicpage.sti"),
        skills_page: load("review/reviewskillspage.sti"),
        skills_icons: load("review/reviewskillsicons.sti"),
        slider: load("review/reviewsliderbar.sti"),
        combat_panel: load("review/commod_back.sti"),
        combat_buttons: load("review/commod_buttons.sti"),
        effect_filters: load("review/reviewfxfilters.sti"),
        scroll: load("main interface/main_scroll.sti"),
        column: load("main interface/main_column_info_normal.sti"),
        health: load("main interface/healthbar.sti"),
        stamina: load("main interface/staminabar.sti"),
        left_plug: load("main interface/leftplug.sti"),
        right_plug: load("main interface/rightplug.sti"),
        condition_rim: load("main interface/condition_highlight.sti"),
        gold_icon: load("review/reviewgoldicon.sti"),
        highlights: load("review/reviewitemhighlights.sti"),
        unusable: load("review/itemusablebackground.sti"),
        unidentified: load("review/itemunidentifiedoverlay.sti"),
        exit: load("char generation/cg_bottombuttons.sti"),
        font: load("fonts/wiz_text_font.sti"),
        bold: load("fonts/wiz_text_font_bold.sti"),
        small_font: load("fonts/smfnt.sti"),
        green: load("fonts/palettegreen.sti"),
        yellow: load("fonts/paletteyellow.sti"),
        gray: load("fonts/palettegray.sti"),
        beige: load("fonts/palettebeige.sti"),
        blue: load("fonts/paletteblue.sti"),
        bronze: load("fonts/palettebronze.sti"),
        red: load("fonts/palettered.sti"),
        leveling_bar: load("review/leveling_bar.sti"),
        realms: ["fire", "water", "air", "earth", "mental", "divine"]
            .map(|realm| load(&format!("spell casting/{realm}_realm.sti"))),
        info_dialog: load("dialogs/popup_monsterinfo.sti"),
        info_buttons: load("dialogs/popup_confirmationbuttons.sti"),
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
        ContextActivity::<InventoryKey>::INACTIVE
        on(open_with_key)
    });
}

/// The key is held by the same rule as the other world shortcuts, so a
/// question or a focused control owns the keyboard instead.
fn hold_inventory_key(
    mut commands: Commands,
    world_keys: super::main_screen::keys::WorldKeys,
    keys: Query<(Entity, &ContextActivity<InventoryKey>)>,
) {
    let active = world_keys.active();
    for (entity, activity) in &keys {
        if **activity != active {
            commands.entity(entity).insert(ContextActivity::<InventoryKey>::new(active));
        }
    }
}

/// The one acceptance rule for opening a member's review, however it was
/// asked for: the party is in the world, entering no level, under no
/// question and no pending screen change, and the member is in the party.
/// Acceptance selects that member together with the screen transition.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct OpenReview<'w, 's> {
    pub(super) selected: ResMut<'w, super::main_screen::SelectedMember>,
    screen: Res<'w, State<Screen>>,
    next: ResMut<'w, NextState<Screen>>,
    freedom: crate::game::freedom::Freedom<'w, 's>,
    roster: Single<'w, 's, &'static PartyRoster>,
}

impl OpenReview<'_, '_> {
    /// Open `member`'s review; whether it opens.
    pub(crate) fn open(&mut self, member: Entity) -> bool {
        if *self.screen.get() != Screen::World
            || !matches!(*self.next, NextState::Unchanged)
            || !self.freedom.undisturbed()
            || !self.roster.contains(&member)
        {
            return false;
        }
        self.selected.set_if_neq(super::main_screen::SelectedMember(Some(member)));
        self.next.set(Screen::Inventory);
        true
    }
}

/// Retail opens the selected character.
fn open_with_key(
    _: On<Start<OpenInventory>>,
    mut review: OpenReview,
    roster: Single<&PartyRoster>,
) {
    if let Some(member) = review.selected.member(&roster) {
        review.open(member);
    }
}

/// Screen state owns existence; the root owns its local selection and input.
fn enter_inventory(
    mut commands: Commands,
    roster: Single<Ref<PartyRoster>>,
    selected: Res<super::main_screen::SelectedMember>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(member) = selected.member(&roster) else {
        next.set(Screen::World);
        return;
    };
    commands
        .spawn_scene(bsn! {
            @centred()
            InventoryRoot
            TabGroup::modal() bevy::ui_widgets::ModalDialog
            BackgroundColor(Color::BLACK)
            GlobalZIndex(10)
            ContextPriority::<InventoryRoot>::new(1)
            Actions<InventoryRoot> [
                Action::<CloseInventory>::new()
                ActionSettings { consume_input: true }
                Bindings [ Binding(KeyCode::Escape) -- Binding(KeyCode::KeyI) ]
            ]
            on(|_: On<Start<CloseInventory>>, mut close: CloseReview| { close.finish(); })
        })
        .insert((InventoryScreen { member }, DespawnOnExit(Screen::Inventory)));
}

/// Capture the preferred member while accepting the user's close, before the
/// view can disappear. Gameplay settles the request independently of the view.
#[derive(bevy::ecs::system::SystemParam)]
struct CloseReview<'w, 's> {
    commands: Commands<'w, 's>,
    view: Single<'w, 's, &'static InventoryScreen>,
    next: ResMut<'w, NextState<Screen>>,
}
impl CloseReview<'_, '_> {
    fn finish(&mut self) {
        self.commands
            .trigger(crate::game::inventory::operations::PutAwayHeld { member: self.view.member });
        self.next.set(Screen::World);
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

/// A transient display address, never a retained gameplay item identity.
/// Pool activation resolves its row to the owned stack before transferring.
enum CellLocation {
    Member(MemberSlot),
    Pool(usize),
}

impl Cell {
    fn location(self, member: Entity, pool_start: usize) -> CellLocation {
        match self {
            Self::Member(slot) => CellLocation::Member(MemberSlot { member, slot }),
            Self::Pool(index) => CellLocation::Pool(pool_start + index),
        }
    }

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

#[derive(Component, Clone, Copy)]
enum CellPart {
    Picture,
    Count,
    Unusable,
    Unidentified,
    Rim,
}

/// A cell's count: a stack above one, charges or shots; `?` for charges
/// not yet identified.
fn count_label(item: &ItemStack, kind: QuantityKind) -> Option<String> {
    let count = item.remaining_uses().unwrap_or(item.count());
    match kind {
        QuantityKind::None | QuantityKind::Unlabelled => None,
        QuantityKind::Stack if count <= 1 => None,
        QuantityKind::Charges | QuantityKind::Uses if !item.identified => Some("?".to_owned()),
        _ => Some(count.to_string()),
    }
}

#[derive(Clone, Copy)]
enum HeaderPart {
    Portrait,
    Statue,
    Name,
    Race,
    Profession,
    Level,
}
#[derive(Component)]
struct HeaderDisplay(HeaderPart);
#[derive(Component, Clone, Copy, PartialEq, Eq, FromTemplate)]
struct InventoryMemberButton(Entity);
#[derive(Component, Clone, Default)]
struct GoldLabel;
#[derive(Component, Clone, Copy)]
struct RetailLabel(usize);
#[derive(Component, Clone, Default)]
struct PortraitHighlight;
#[derive(Component, Clone, Default)]
struct VitalArt;
#[derive(Component, Clone, Copy)]
enum VitalShade {
    Health,
    Stamina,
}
#[derive(Component, Clone, Copy)]
enum HeaderCondition {
    Icon,
    Rim,
}
#[derive(Component, Clone, Copy)]
enum HeaderHand {
    Background,
    Hand(bool),
    Count(bool),
    Armour,
}

fn header(art: &InventoryArt) -> impl SceneList {
    let empty: Vec<_> = (0..8)
        .map(|slot| {
            bsn! {
                @picture(art.corner.clone(),slot+1,member_position(slot))
                template(move |_| Ok(EmptyMemberSlot(slot)))
            }
        })
        .collect();
    bsn_list! {
        @rect(Vec2::ZERO, Vec2::ZERO, Color::NONE) MemberSelector Children [{empty}]
        -- @picture(default(),0,Vec2::new(164.0,12.0)) template(|_|Ok(HeaderDisplay(HeaderPart::Portrait)))
        -- @picture(art.corner.clone(),9,Vec2::new(164.0,12.0))
        -- @picture(art.left_plug.clone(),0,Vec2::new(165.0,14.0)) template(|_|Ok(HeaderCondition::Icon))
        -- @picture(art.condition_rim.clone(),0,Vec2::new(164.0,13.0)) template(|_|Ok(HeaderCondition::Rim)) Visibility::Hidden
        -- @picture(art.right_plug.clone(),0,Vec2::new(238.0,14.0))
        -- @label(art.bold.clone(),Vec2::new(116.0,96.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Name)))
        -- @label(art.font.clone(),Vec2::new(116.0,111.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Race)))
        -- @label(art.font.clone(),Vec2::new(116.0,124.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Profession)))
        -- @label(art.font.clone(),Vec2::new(116.0,137.0),186.0) template(|_|Ok(HeaderDisplay(HeaderPart::Level)))
        -- @picture(art.column.clone(),0,Vec2::new(129.0,11.0)) template(|_|Ok(HeaderHand::Background))
        -- @picture(default(),0,Vec2::new(133.0,34.0)) template(|_|Ok(HeaderHand::Hand(false)))
        -- @picture(default(),0,Vec2::new(133.0,58.0)) template(|_|Ok(HeaderHand::Hand(true)))
        -- @label(art.small_font.clone(),Vec2::new(129.0,49.0),16.0) template(|_|Ok(HeaderHand::Count(false)))
        -- @label(art.small_font.clone(),Vec2::new(132.0,73.0),16.0) template(|_|Ok(HeaderHand::Count(true)))
        -- @label(art.small_font.clone(),Vec2::new(132.0,14.0),13.0) template(|_|Ok(HeaderHand::Armour))
        -- @picture(art.column.clone(),1,Vec2::new(268.0,11.0))
        -- @picture(art.health.clone(),0,Vec2::new(274.0,34.0)) VitalArt
        -- @picture(art.stamina.clone(),0,Vec2::new(278.0,34.0)) VitalArt
        -- @rect(Vec2::new(274.0,34.0),Vec2::new(3.0,0.0),Color::BLACK) VitalArt template(|_|Ok(VitalShade::Health))
        -- @rect(Vec2::new(278.0,34.0),Vec2::new(3.0,0.0),Color::BLACK) VitalArt template(|_|Ok(VitalShade::Stamina))
    }
}

fn select_inventory_member(
    click: On<Activate>,
    buttons: Query<&InventoryMemberButton>,
    roster: Single<Ref<PartyRoster>>,
    screen: Option<Single<&mut InventoryScreen>>,
) {
    if let Ok(button) = buttons.get(click.entity)
        && roster.contains(&button.0)
        && let Some(mut screen) = screen
    {
        screen.member = button.0;
    }
}

fn draw_portrait_highlights(
    screen: Option<Single<&InventoryScreen>>,
    roster: Single<Ref<PartyRoster>>,
    buttons: Query<(&InventoryMemberButton, &Hovered, &Children)>,
    mut highlights: Query<(&mut Picture, &mut Visibility), With<PortraitHighlight>>,
) {
    let Some(screen) = screen else { return };
    for (button, hovered, children) in &buttons {
        for child in children.iter() {
            let Ok((mut picture, mut visibility)) = highlights.get_mut(child) else { continue };
            let occupied = roster.contains(&button.0);
            let frame = if hovered.0 {
                Some(0)
            } else if button.0 == screen.member {
                Some(1)
            } else {
                None
            };
            visibility.set_if_neq(visible_if(occupied && frame.is_some()));
            if let Some(frame) = frame
                && picture.frame != frame
            {
                picture.frame = frame;
            }
        }
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
    let fill = art.fill.clone();
    let node = absolute(area.min, size + 1.0);
    let rim = match cell {
        Cell::Member(Slot::Equipment(slot)) => equipment_rim(slot) + 3,
        _ => 0,
    };
    bsn! {
        node template(move |_|Ok(ItemCell{cell}))
        UiButton TabIndex(0) Pickable::default()
        crate::presentation::retail_controls::FocusRing
        on(click_cell)
        Children [
            template(move |_| Ok(ImageNode { image: fill.clone(), rect: Some(Rect::new(0.0, 0.0, 46.0, 54.0)), ..default() }))
            Node { position_type: PositionType::Absolute, width: px(46), height: px(54) }
            template(move |_| Ok(visible_if(grid)))
            Pickable::IGNORE
            -- @picture(art.unusable.clone(),0,Vec2::ZERO)
            template(|_| Ok(CellPart::Unusable))
            -- @picture(default(),frame,at)
            template(|_| Ok(CellPart::Picture))
            -- @label(art.font.clone(),count_at,width)
            template(|_| Ok(CellPart::Count))
            -- @picture(art.unidentified.clone(),0,Vec2::ZERO)
            template(|_| Ok(CellPart::Unidentified))
            -- @picture(art.highlights.clone(),rim,-Vec2::splat(2.0))
            template(|_| Ok(CellPart::Rim))
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
    focus: Option<ResMut<bevy::input_focus::InputFocus>>,
    sheets: Res<Assets<SpriteSheet>>,
    roots: Query<Entity, (With<InventoryRoot>, Without<InventoryBuilt>)>,
    strings: crate::presentation::screen_art::GameStrings,
) {
    let Ok(root) = roots.single() else { return };
    if [
        &art.corner,
        &art.portrait_highlight,
        &art.page,
        &art.bottom_bar,
        &art.page_buttons,
        &art.item_buttons,
        &art.stats_page,
        &art.magic_page,
        &art.skills_page,
        &art.skills_icons,
        &art.slider,
        &art.combat_panel,
        &art.combat_buttons,
        &art.effect_filters,
        &art.scroll,
        &art.column,
        &art.health,
        &art.stamina,
        &art.left_plug,
        &art.right_plug,
        &art.condition_rim,
        &art.gold_icon,
        &art.exit,
        &art.bold,
        &art.small_font,
        &art.green,
        &art.yellow,
        &art.gray,
        &art.beige,
        &art.blue,
        &art.bronze,
        &art.red,
        &art.leveling_bar,
        &art.info_dialog,
        &art.info_buttons,
        &art.highlights,
        &art.unusable,
        &art.unidentified,
        &art.font,
    ]
    .into_iter()
    .chain(art.realms.iter())
    .any(|handle| matches!(assets.get_load_state(handle), Some(bevy::asset::LoadState::Failed(_))))
        || strings.failed(&assets)
        || matches!(assets.get_load_state(&art.fill), Some(bevy::asset::LoadState::Failed(_)))
    {
        // Presentation failure hides the view, but does not accept a put-away.
        // The hand still owns its item and world controls can handle it.
        warn!("inventory artwork failed to load; returning to the world");
        next.set(Screen::World);
        return;
    }
    let get = |handle: &Handle<SpriteSheet>| sheets.get(handle);
    let (Some(corner), Some(page), Some(bar), Some(gold), Some(exit)) = (
        get(&art.corner),
        get(&art.page),
        get(&art.bottom_bar),
        get(&art.gold_icon),
        get(&art.exit),
    ) else {
        return;
    };
    let fixed = vec![
        sprite(page, 0, Vec2::new(310.0, 0.0)),
        sprite(page, 1, Vec2::new(0.0, 165.0)),
        sprite(page, 2, Vec2::new(113.0, 165.0)),
        sprite(page, 3, Vec2::new(478.0, 165.0)),
        sprite(gold, 0, Vec2::new(514.0, 422.0)),
    ];
    let exit = sprite(exit, 16, Vec2::new(596.0, 450.0));
    let corner = sprite(corner, 0, Vec2::ZERO);
    let bottom_bar = sprite(bar, 0, Vec2::new(0.0, 450.0));
    let cells: Vec<_> = EnumSet::<EquipSlot>::all()
        .iter()
        .map(|slot| Cell::Member(Slot::Equipment(slot)))
        .chain((0..BACKPACK_SLOTS).map(|i| Cell::Member(Slot::Backpack(i))))
        .map(|cell| cell_controls(cell, &art))
        .collect();
    commands.entity(root).insert(InventoryBuilt);
    let content = commands
        .spawn_scene(bsn! {
            @screen() ChildOf(root)
            Children [
                @{corner}
                -- {header(&art)}
            ]
        })
        .id();
    let items = commands.spawn_scene(bsn! {
        @screen() Pickable::IGNORE ChildOf(content)
        template(|_| Ok(review::Page::Items))
        Children [
            {fixed}
            -- @label(art.bold.clone(),Vec2::new(12.0,174.0),93.0) template(|_|Ok(RetailLabel(0x92b)))
            -- @label(art.bold.clone(),Vec2::new(486.0,174.0),141.0) template(|_|Ok(RetailLabel(0x92c)))
            -- @picture(default(),0,Vec2::new(194.0,165.0)) template(|_|Ok(HeaderDisplay(HeaderPart::Statue)))
            -- @label(art.font.clone(),Vec2::new(553.0,425.0),48.0) GoldLabel
            -- {cells}
        ]
    }).id();
    commands.spawn_scene(bsn! { @{bottom_bar} ChildOf(content) });
    commands.spawn_scene(bsn! {
        @{exit} ChildOf(content)
            UiButton TabIndex(0) Pickable::default()
            crate::presentation::retail_controls::FocusRing
            on(|_: On<Activate>, mut close: CloseReview| { close.finish(); })
    });
    let initial_tab = review::build(&mut commands, content, &art);
    if let Some(mut focus) = focus {
        focus.set(initial_tab, bevy::input_focus::FocusCause::Navigated);
    }
    item_info::build(&mut commands, items, content, &art);
    spells::build(&mut commands, content, &art);
    let pool = commands
        .spawn((absolute(Vec2::ZERO, Vec2::new(640.0, 450.0)), Pickable::IGNORE, ChildOf(items)))
        .observe(crate::presentation::retail_controls::scroll_wheel)
        .id();
    let track = pool_scroll::build(&mut commands, pool, &art);
    commands.entity(pool).insert(crate::presentation::retail_controls::ScrollWheel(track));
    let pool_cells: Vec<_> = (0..POOL_CELLS).map(|i| cell_controls(Cell::Pool(i), &art)).collect();
    commands.entity(pool).queue_spawn_related_scenes::<Children>(bsn_list! { {pool_cells} });
}

// Retail profession titles, indexed by profession and level band.
fn profession_title(profile: &Character) -> usize {
    const TITLES: [[usize; 9]; 15] = [
        [0x3b6, 0x3b7, 0x3b8, 0x3b9, 0x3ba, 0x3bb, 0x3bc, 0x3bd, 0x3be],
        [0x3b6, 0x3b7, 0x3bf, 0x3c0, 0x3c1, 0x3c2, 0x3c3, 0x3c4, 0x3c5],
        [0x3b6, 0x3b7, 0x3c6, 0x3b9, 0x3c7, 0x3c2, 0x3c8, 0x3c9, 0x3ca],
        [0x3b6, 0x3b7, 0x3cb, 0x3cc, 0x3cd, 0x3ce, 0x3cf, 0x3d0, 0x3d1],
        [0x3b6, 0x3b7, 0x3d2, 0x3d3, 0x3d4, 0x3d5, 0x3d6, 0x3bd, 0x3d7],
        [0x3b6, 0x3b7, 0x3d8, 0x3d9, 0x3da, 0x3db, 0x3dc, 0x3dd, 0x3de],
        [0x3b6, 0x3b7, 0x3f9, 0x3df, 0x3e0, 0x3e1, 0x3dc, 0x3e2, 0x3e3],
        [0x3b6, 0x3b7, 0x3e4, 0x3e5, 0x3e6, 0x3e7, 0x3e8, 0x3e9, 0x3ea],
        [0x3b6, 0x3b7, 0x3eb, 0x3ec, 0x3ed, 0x3ee, 0x3ef, 0x3f0, 0x3f1],
        [0x3b6, 0x3b7, 0x3f2, 0x3f3, 0x3f4, 0x3f5, 0x3f6, 0x3f7, 0x3f8],
        [0x3b6, 0x3b7, 0x3f9, 0x3fa, 0x3fb, 0x3fc, 0x3fd, 0x3fe, 0x3ff],
        [0x3b6, 0x3b7, 0x400, 0x401, 0x402, 0x403, 0x404, 0x405, 0x406],
        [0x3b6, 0x3b7, 0x407, 0x408, 0x409, 0x40a, 0x40b, 0x40c, 0x40d],
        [0x3b6, 0x3b7, 0x40e, 0x40f, 0x410, 0x411, 0x412, 0x413, 0x414],
        [0x3b6, 0x3b7, 0x415, 0x416, 0x417, 0x418, 0x419, 0x41a, 0x41b],
    ];
    let level = profile.profession_levels[profile.profession];
    let band = match level {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=6 => 3,
        7..=10 => 4,
        11..=15 => 5,
        16..=21 => 6,
        22..=27 => 7,
        _ => 8,
    };
    TITLES[profile.profession as usize][band]
}

fn draw_vitals(
    screen: Option<Single<&InventoryScreen>>,
    members: Query<(&Character, &Vitals)>,
    mut art: Query<(&VitalArt, Option<&VitalShade>, Option<&mut Node>, &mut Visibility)>,
) {
    let Some(screen) = screen else { return };
    let Ok((profile, vitals)) = members.get(screen.member) else { return };
    for (_, shade, node, mut visibility) in &mut art {
        visibility.set_if_neq(visible_if(!vitals.is_dead()));
        if let (Some(shade), Some(mut node)) = (shade, node) {
            let (current, maximum) = match shade {
                VitalShade::Health => (vitals.hp, profile.stats(&vitals.enchantments).max_hp()),
                VitalShade::Stamina => {
                    (vitals.stamina, profile.stats(&vitals.enchantments).max_stamina())
                }
            };
            let filled = super::screen_art::vital_fill(current, maximum);
            let height = px((45 - filled) as f32);
            if node.height != height {
                node.height = height;
            }
        }
    }
}

fn draw_header_condition(
    screen: Option<Single<&InventoryScreen>>,
    members: Query<&Vitals>,
    assets: Res<AssetServer>,
    art: Res<InventoryArt>,
    mut displays: Query<(&HeaderCondition, &mut Picture, &mut Visibility)>,
) {
    let Some(screen) = screen else { return };
    let Ok(vitals) = members.get(screen.member) else { return };
    let condition = super::screen_art::condition_icon(vitals);
    for (display, mut picture, mut visibility) in &mut displays {
        match *display {
            HeaderCondition::Icon => {
                if let Some(name) = &condition {
                    show_sheet(
                        &mut picture,
                        &assets,
                        format!("retail://data/icons/conditions/{name}.sti"),
                    );
                } else if picture.sheet != art.left_plug {
                    picture.sheet = art.left_plug.clone();
                }
            }
            HeaderCondition::Rim => {
                visibility.set_if_neq(visible_if(condition.is_some()));
            }
        };
    }
}

fn draw_header_hands(
    screen: Option<Single<&InventoryScreen>>,
    assets: Res<AssetServer>,
    content: If<Res<crate::game::data::Catalogs>>,
    members: Query<crate::presentation::character_view::CharacterView>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    party_effects: Option<Single<&crate::game::magic::enchantments::PartyEffects>>,
    art: Res<InventoryArt>,
    mut parts: Query<(&HeaderHand, &mut Visibility, Option<&mut Picture>, Option<&mut BitmapText>)>,
) {
    let Some(screen) = screen else { return };
    let catalog = &content.items;
    let Ok(view) = members.get(screen.member) else { return };
    let armour = view.armour(
        screen.member,
        catalog,
        battle.as_deref().copied(),
        party_effects.as_deref().copied(),
    );
    let profile = view.profile;
    let carried = view.carried;
    let empty = Carried::default();
    let carried = carried.unwrap_or(&empty);
    let two_handed = carried
        .worn(EquipSlot::Primary)
        .and_then(|item| catalog.get(item.item))
        .is_some_and(|item| item.two_handed);
    for (part, mut visibility, picture, text) in &mut parts {
        let mut source = None;
        let mut path = None;
        let mut value = None;
        let mut shown = true;
        match *part {
            HeaderHand::Background => {
                source = Some(Picture {
                    sheet: art.column.clone(),
                    frame: if two_handed { 2 } else { 0 },
                    at: Vec2::new(129.0, 11.0),
                    ..default()
                })
            }
            HeaderHand::Hand(off) => {
                shown = !(off && two_handed);
                let slot = if off { EquipSlot::Secondary } else { EquipSlot::Primary };
                let item = carried.worn(slot).and_then(|item| catalog.get(item.item));
                let (hand_path, frame, x) = if let Some(item) = item {
                    (item.image.clone(), 2, 132.0)
                } else {
                    (
                        format!(
                            "data/main interface/{}.sti",
                            empty_hands(profile.race)[usize::from(off)]
                        ),
                        0,
                        133.0,
                    )
                };
                path = Some(crate::content::paths::retail(&hand_path));
                source = Some(Picture {
                    sheet: default(),
                    frame,
                    at: Vec2::new(x, if off { 58.0 } else { 34.0 }),
                    ..default()
                });
            }
            HeaderHand::Count(off) => {
                shown = !(off && two_handed);
                let slot = if off { EquipSlot::Secondary } else { EquipSlot::Primary };
                value = Some(
                    match carried
                        .worn(slot)
                        .filter(|item| {
                            catalog
                                .get(item.item)
                                .is_some_and(|d| d.quantity == QuantityKind::Stack)
                        })
                        .map(ItemStack::count)
                    {
                        Some(count) => count.to_string(),
                        _ => String::new(),
                    },
                );
            }
            HeaderHand::Armour => {
                value = Some(armour.as_ref().map_or(0, Armour::average).to_string())
            }
        }
        visibility.set_if_neq(visible_if(shown));
        if let (Some(mut picture), Some(mut source)) = (picture, source) {
            if let Some(path) = path {
                show_sheet(&mut picture, &assets, path);
                source.sheet = picture.sheet.clone();
            }
            picture.set_if_neq(source);
        }
        if let (Some(mut text), Some(value)) = (text, value)
            && text.value != value
        {
            text.value = value;
        }
    }
}

#[derive(Component, Clone, Default)]
struct MemberSelector;
#[derive(Component)]
struct EmptyMemberSlot(usize);
fn member_position(slot: usize) -> Vec2 {
    Vec2::new(5.0 + (slot % 2) as f32 * 48.0, 4.0 + (slot / 2) as f32 * 39.0)
}

/// Occupied controls retain actor identity; slots only determine placement.
fn sync_member_portraits(
    mut commands: Commands,
    roots: Query<Entity, With<MemberSelector>>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, Option<&Vitals>)>,
    mut buttons: Query<(Entity, &InventoryMemberButton, &mut Picture)>,
    mut empty: Query<(&EmptyMemberSlot, &mut Visibility)>,
    art: Res<InventoryArt>,
    assets: Res<AssetServer>,
) {
    let Ok(root) = roots.single() else { return };
    for (slot, mut visible) in &mut empty {
        let occupied = roster.get_index(slot.0).is_some_and(|e| members.contains(*e));
        visible.set_if_neq(visible_if(!occupied));
    }
    for (entity, button, _) in &buttons {
        if !roster.contains(&button.0) || !members.contains(button.0) {
            commands.entity(entity).despawn();
        }
    }
    for (slot, member) in roster.iter().enumerate() {
        let Ok((profile, vitals)) = members.get(member) else { continue };
        let name = if vitals.is_some_and(Vitals::is_dead) {
            &super::screen_art::skull(profile.race)[1..]
        } else {
            &profile.portrait
        };
        let path =
            crate::content::paths::portrait(crate::content::paths::PortraitSize::Small, name);
        let at = member_position(slot) + Vec2::ONE;
        if let Some((_, _, mut picture)) = buttons.iter_mut().find(|(_, b, _)| b.0 == member) {
            show_sheet(&mut picture, &assets, path);
            if picture.at != at {
                picture.at = at;
            }
        } else {
            let source = Picture { sheet: assets.load(path), at, ..default() };
            commands
                .spawn_scene(bsn! {
                    template(move |_| Ok(source.clone()))
                    UiButton TabIndex(0) Hovered::default() Pickable::default()
                    crate::presentation::retail_controls::FocusRing
                    InventoryMemberButton({member})
                        on(select_inventory_member)
                    Children [
                        @picture(art.portrait_highlight.clone(),1,Vec2::ZERO)
                        PortraitHighlight Visibility::Hidden
                    ]
                })
                .insert(ChildOf(root));
        }
    }
}

/// Update the existing header controls from their actual dependencies.
fn draw_header(
    screen: Option<Single<&InventoryScreen>>,
    assets: Res<AssetServer>,
    members: Query<(&Character, Option<&Vitals>)>,
    strings: crate::presentation::screen_art::GameStrings,
    mut displays: Query<(&HeaderDisplay, Option<&mut Picture>, Option<&mut BitmapText>)>,
) {
    let Some(screen) = screen else { return };
    let Ok((profile, vitals)) = members.get(screen.member) else { return };
    for (display, picture, label) in &mut displays {
        let mut source = None;
        let mut value = None;
        match display.0 {
            HeaderPart::Portrait => {
                let name = if vitals.as_ref().is_some_and(|v| v.is_dead()) {
                    &super::screen_art::skull(profile.race)[1..]
                } else {
                    &profile.portrait
                };
                source = Some(crate::content::paths::portrait(
                    crate::content::paths::PortraitSize::Medium,
                    name,
                ));
            }
            HeaderPart::Statue => {
                source = Some(crate::content::paths::sheet(&format!(
                    "review/{}",
                    statue(profile.race, profile.gender)
                )));
            }
            HeaderPart::Name => value = Some(profile.name.clone()),
            HeaderPart::Race => {
                let gender = if profile.gender == Gender::Male { 0x2d1 } else { 0x2d2 };
                let gender = strings.get(gender);
                let race = strings.get(crate::content::ui::RACE_NAMES + profile.race as usize);
                value = Some(format!("{gender} {race}").trim().to_owned());
            }
            HeaderPart::Profession => {
                value = Some(
                    strings
                        .get(crate::content::ui::PROFESSION_NAMES + profile.profession as usize)
                        .to_owned(),
                );
            }
            HeaderPart::Level => {
                let title = strings.get(profession_title(profile));
                let level = strings.get(0x91a);
                value = Some(format!("{level} {} ({title})", profile.level()));
            }
        }
        if let Some(mut picture) = picture
            && let Some(path) = source
        {
            show_sheet(&mut picture, &assets, path);
        }
        if let Some(mut label) = label
            && let Some(value) = value
            && label.value != value
        {
            label.value = value;
        }
    }
}

fn draw_retail_labels(
    strings: crate::presentation::screen_art::GameStrings,
    mut labels: Query<(Ref<RetailLabel>, &mut BitmapText)>,
) {
    for (label, mut text) in &mut labels {
        let value = strings.get(label.0);
        if text.value != value {
            text.value = value.to_owned();
        }
    }
}

fn draw_gold(
    party: Single<Ref<PartyInventory>>,
    mut labels: Query<(Ref<GoldLabel>, &mut BitmapText)>,
) {
    if !party.is_changed() && !labels.iter().any(|(label, _)| label.is_added()) {
        return;
    }
    let value = format_gold(party.gold);
    for (_, mut label) in &mut labels {
        if label.value != value {
            label.value = value.clone();
        }
    }
}

fn draw_cells(
    screen: Option<Single<&InventoryScreen>>,
    assets: Res<AssetServer>,
    content: If<Res<crate::game::data::Catalogs>>,
    members: Query<(&Character, Option<&Carried>)>,
    party: Single<&PartyInventory>,
    pool_scroll: Option<Single<&bevy::ui_widgets::SliderValue, With<pool_scroll::PoolScroll>>>,
    cells: Query<(&ItemCell, &Children)>,
    mut displays: Query<(
        &CellPart,
        &mut Visibility,
        Option<&mut Picture>,
        Option<&mut BitmapText>,
    )>,
) {
    let Some(screen) = screen else { return };
    let catalog = &content.items;
    let Ok((profile, carried)) = members.get(screen.member) else { return };
    let empty = Carried::default();
    let carried = carried.unwrap_or(&empty);
    let pool_start = pool_scroll.as_ref().map_or(0, |value| value.0 as usize * 2);
    for (cell, children) in &cells {
        let item = match cell.cell.location(screen.member, pool_start) {
            CellLocation::Member(slot) => carried.slot(slot.slot),
            CellLocation::Pool(index) => party.pool.stacks.get(index),
        };
        let definition = item.and_then(|i| Some((i, catalog.get(i.item)?)));
        let grid = !matches!(cell.cell, Cell::Member(Slot::Equipment(_)));
        let count = definition.and_then(|(i, d)| count_label(i, d.quantity));
        for child in children.iter() {
            let Ok((part, mut visible, picture, label)) = displays.get_mut(child) else { continue };
            let show = match *part {
                CellPart::Picture => definition.is_some(),
                CellPart::Count => count.is_some(),
                CellPart::Unusable => grid && definition.is_some_and(|(_, d)| !profile.can_use(d)),
                CellPart::Unidentified => definition.is_some_and(|(i, _)| !i.identified),
                CellPart::Rim => !grid && definition.is_some(),
            };
            visible.set_if_neq(visible_if(show));
            match *part {
                CellPart::Picture => {
                    if let Some((_, definition)) = definition
                        && let Some(mut picture) = picture
                    {
                        show_sheet(
                            &mut picture,
                            &assets,
                            crate::content::paths::retail(&definition.image),
                        );
                        let at = picture.at;
                        picture.set_if_neq(Picture {
                            sheet: picture.sheet.clone(),
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
                        let value = count.as_deref().unwrap_or_default();
                        if label.value != value {
                            label.value = value.to_owned();
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Click a cell (retail's item-page click, `rcs.cpp:1459-1742`): with an
/// empty hand, pick the item up; holding one, a pool cell takes it in, a
/// stack of the same item takes what it can, anything else swaps.
/// Shift-clicking a single unit and manual reloading are not wired to UI yet.
fn click_cell(
    click: On<Activate>,
    cells: Query<&ItemCell>,
    screen: Option<Single<(&InventoryScreen, &mut ItemAction)>>,
    mut inventory: crate::game::inventory::operations::InventoryOperations,
    selected: Option<Single<&bevy::ui_widgets::SelectedTab, With<review::PageTabs>>>,
    tabs: Query<&review::PageButton>,
    pool_scroll: Option<Single<&bevy::ui_widgets::SliderValue, With<pool_scroll::PoolScroll>>>,
    conversations: Query<
        (),
        (
            With<crate::game::npcs::conversation::Conversation>,
            Without<crate::game::npcs::conversation::Closing>,
        ),
    >,
    (mut ids, mut rng): (
        ResMut<crate::game::inventory::ItemIds>,
        ResMut<crate::game::rng::GameRng>,
    ),
) {
    if !conversations.is_empty() {
        return;
    }
    let (Ok(cell), Some(screen), Some(selected)) = (cells.get(click.entity), screen, selected)
    else {
        return;
    };
    if selected.0.and_then(|entity| tabs.get(entity).ok()).map(|tab| tab.0)
        != Some(review::Page::Items)
    {
        return;
    }
    let (screen, mut action) = screen.into_inner();
    let destination = cell
        .cell
        .location(screen.member, pool_scroll.as_ref().map_or(0, |value| value.0 as usize * 2));
    // An armed action takes the click; an empty cell disarms it.
    let target = match destination {
        CellLocation::Member(slot) => Some(ItemAt::Slot(slot.slot)),
        CellLocation::Pool(row) => inventory.pool_row(row).map(ItemAt::Pool),
    };
    use crate::game::inventory::{Refusal, merge::Merged};
    match *action {
        ItemAction::Identify => {
            if let Some(at) = target
                && let Err(refusal) = inventory.identify(screen.member, at, &mut rng)
                && refusal != Refusal::Nothing
            {
                info!("identifying refused: {refusal:?}");
            }
            *action = ItemAction::None;
            return;
        }
        // Retail keeps merging armed until something is made or stacked.
        ItemAction::Merge if inventory.holds() => {
            match target.map(|at| inventory.merge(screen.member, at, (&mut ids, &mut rng))) {
                None | Some(Ok(Merged::Made(_) | Merged::Stacked) | Err(Refusal::Nothing)) => {
                    *action = ItemAction::None;
                }
                Some(Ok(_)) => {}
                Some(Err(refusal)) => info!("merging refused: {refusal:?}"),
            }
            return;
        }
        _ => {}
    }
    let result = match destination {
        CellLocation::Member(slot) => inventory.interact_slot(slot),
        CellLocation::Pool(index) => inventory.click_pool(index),
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
    party: Single<Ref<PartyInventory>>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    scale: Res<UiScale>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    content: If<Res<crate::game::data::Catalogs>>,
    mut picture: Single<(&mut ImageNode, &mut Node, &mut Visibility), With<HeldPicture>>,
    mut loading: Local<Option<(String, Handle<SpriteSheet>)>>,
) {
    let (image, node, visibility) = &mut *picture;
    let catalog = &content.items;
    let definition = party.hand.held.as_ref().and_then(|h| catalog.get(h.item));
    let cursor = pointers
        .iter()
        .find(|(id, _)| **id == PointerId::Mouse)
        .and_then(|(_, location)| location.location.as_ref())
        .map(|location| location.position);
    let (Some(definition), Some(cursor)) = (definition, cursor) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    if loading.as_ref().is_none_or(|(path, _)| *path != definition.image) {
        let handle = assets.load(crate::content::paths::retail(&definition.image));
        *loading = Some((definition.image.clone(), handle));
    }
    let Some((_, handle)) = &*loading else { return };
    let Some((picture, frame)) =
        sheets.get(handle).and_then(|s| Some((s.images.first()?, s.frames.first()?)))
    else {
        return;
    };
    // Retail makes the picture the cursor, its corner at the hotspot with
    // the arrow drawn over it (`UpdateHeldItemCursor`).
    if image.image != *picture {
        image.image = picture.clone();
    }
    node.set_if_neq(absolute(cursor / scale.0, Vec2::new(frame.width as f32, frame.height as f32)));
    visibility.set_if_neq(Visibility::Inherited);
}

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(progression::plugin)
            .add_input_context::<InventoryKey>()
            .add_input_context::<InventoryRoot>()
            .add_input_context::<info::InfoDialog>()
            .add_systems(Startup, (load_inventory_art, spawn_inventory_key, spawn_held_picture))
            .add_systems(
                PreUpdate,
                hold_inventory_key
                    .after(bevy::input_focus::InputFocusSystems::Dispatch)
                    .before(EnhancedInputSystems::Update),
            )
            .add_systems(OnEnter(Screen::Inventory), enter_inventory)
            .add_systems(
                Update,
                (
                    spawn_inventory_screen,
                    (
                        sync_member_portraits,
                        draw_header,
                        draw_header_condition,
                        draw_header_hands,
                        draw_portrait_highlights.after(sync_member_portraits),
                        draw_vitals,
                        draw_retail_labels,
                        draw_gold,
                        draw_cells,
                    )
                        .after(spawn_inventory_screen)
                        .after(crate::game::GameplaySet::Stats)
                        .run_if(any_with_component::<InventoryScreen>),
                    follow_held,
                ),
            )
            .add_systems(
                Update,
                (
                    review::show_pages,
                    review::refresh,
                    review::layout_skills,
                    review::style_info_button,
                    review::style_effect_buttons,
                    style_item_actions,
                    info::update_scroll,
                    info::initialize_focus,
                    item_info::refresh,
                    spells::refresh,
                    pool_scroll::sync.before(draw_cells),
                )
                    .after(spawn_inventory_screen)
                    .after(crate::game::GameplaySet::Stats),
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
    use crate::content::{items::ItemCatalog, ui::TextTable};

    /// The inventory screen as the game installs it, over keyboard input.
    fn inventory_app() -> App {
        let mut app = crate::presentation::test_input::input_app();
        app.init_state::<Screen>()
            .init_asset::<SpriteSheet>()
            .init_asset::<TextTable>()
            .init_asset::<Image>()
            .init_resource::<UiScale>()
            .add_message::<crate::game::inventory::ItemNews>()
            .add_observer(crate::game::inventory::operations::put_away_held)
            .init_resource::<crate::game::inventory::ItemIds>()
            .init_resource::<crate::game::rng::GameRng>()
            .init_resource::<super::super::main_screen::SelectedMember>()
            .add_plugins(InventoryPlugin)
            .add_observer(crate::presentation::retail_controls::primary_buttons_only)
            .add_systems(PostUpdate, crate::presentation::retail_controls::sync_button_navigation);
        app
    }

    #[test]
    fn scrolling_a_cell_projects_and_activates_the_same_location() {
        use crate::game::{
            characters::{Gender, Profession, Race},
            inventory::ItemIds,
        };
        let mut app = crate::presentation::test_input::input_app();
        app.init_asset::<SpriteSheet>()
            .add_message::<crate::game::inventory::ItemNews>()
            .init_resource::<ItemIds>()
            .init_resource::<crate::game::rng::GameRng>()
            .add_systems(Update, draw_cells);
        let definition =
            ItemDefinition { quantity: QuantityKind::Stack, stack_limit: 99, ..default() };
        let items = ItemCatalog(vec![definition.clone()]);
        app.insert_resource(crate::game::data::Catalogs { items, ..default() });
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let member =
            app.world_mut().spawn((Vitals::new(&profile), profile, Carried::default())).id();
        let party = crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.world_mut().spawn(InventoryScreen { member });
        let mut ids = ItemIds::default();
        for count in [2, 3, 4] {
            let item = ItemStack::new(&definition, &mut ids, count, true);
            app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(item);
        }
        let target = app.world().get::<PartyInventory>(party).unwrap().pool.stacks[2].id;
        let slider = app
            .world_mut()
            .spawn((pool_scroll::PoolScroll, bevy::ui_widgets::SliderValue(0.)))
            .id();
        let tab = app.world_mut().spawn(review::PageButton(review::Page::Items)).id();
        app.world_mut().spawn((review::PageTabs, bevy::ui_widgets::SelectedTab(Some(tab))));
        let cell = app
            .world_mut()
            .spawn_scene(cell_controls(Cell::Pool(0), &InventoryArt::default()))
            .unwrap()
            .id();
        app.finish();
        app.cleanup();
        app.update();
        let children = app.world().get::<Children>(cell).unwrap().to_vec();
        let count =
            children.iter().copied().find(|e| app.world().get::<BitmapText>(*e).is_some()).unwrap();
        assert_eq!(app.world().get::<BitmapText>(count).unwrap().value, "2");
        app.world_mut().entity_mut(slider).insert(bevy::ui_widgets::SliderValue(1.));
        app.update();
        assert_eq!(app.world().get::<BitmapText>(count).unwrap().value, "4");
        assert_eq!(app.world().get::<Children>(cell).unwrap().as_ref(), children.as_slice());
        app.world_mut().trigger(Activate { entity: cell });
        assert_eq!(
            app.world().get::<PartyInventory>(party).unwrap().hand.held.as_ref().unwrap().id,
            target
        );
    }

    #[test]
    fn member_portraits_move_without_retargeting_an_in_flight_activation() {
        use crate::game::characters::{Gender, Profession, Race};
        let mut app = crate::presentation::test_input::input_app();
        app.init_asset::<SpriteSheet>()
            .init_resource::<InventoryArt>()
            .add_systems(Update, sync_member_portraits);
        let a = app
            .world_mut()
            .spawn(Character::new("A", Race::Human, Gender::Male, Profession::Fighter))
            .id();
        let b = app
            .world_mut()
            .spawn(Character::new("B", Race::Human, Gender::Male, Profession::Fighter))
            .id();
        crate::game::party::roster::test_party(app.world_mut(), &[a, b]);
        app.world_mut().spawn(MemberSelector);
        let screen = app.world_mut().spawn(InventoryScreen { member: b }).id();
        app.finish();
        app.cleanup();
        app.update();
        let button = app
            .world_mut()
            .query::<(Entity, &InventoryMemberButton)>()
            .iter(app.world())
            .find(|(_, button)| button.0 == a)
            .unwrap()
            .0;
        crate::game::party::roster::test_reorder(app.world_mut(), &[b, a]);
        app.update();
        assert_eq!(app.world().get::<Picture>(button).unwrap().at, Vec2::new(54., 5.));
        app.world_mut().trigger(Activate { entity: button });
        assert_eq!(app.world().get::<InventoryScreen>(screen).unwrap().member, a);
        app.world_mut().entity_mut(a).remove::<crate::game::party::roster::PartyMember>();
        app.update();
        assert!(app.world().get_entity(button).is_err());
    }

    #[test]
    fn closing_inventory_returns_the_held_item_even_before_art_is_ready() {
        use crate::game::characters::{Gender, Profession, Race};
        use crate::game::inventory::{ItemIds, make_item};
        use crate::game::rng::GameRng;
        use crate::presentation::test_input::key;
        for (still_a_member, explicit_close, full, loaded) in [
            (true, true, false, true),
            (false, true, false, true),
            (true, true, true, true),
            (true, true, false, false),
            (true, false, false, true),
        ] {
            let mut app = inventory_app();
            let window =
                app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow)).id();
            let definition = ItemDefinition { ..default() };
            let item = make_item(
                &definition,
                &mut ItemIds::default(),
                &mut GameRng::seeded(1),
                false,
                true,
            );
            let id = item.id;
            let items = ItemCatalog(vec![definition.clone()]);
            app.insert_resource(crate::game::data::Catalogs {
                items: if loaded { items } else { default() },
                ..default()
            });
            let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
            let member = app
                .world_mut()
                .spawn((
                    crate::game::characters::Vitals::new(&profile),
                    profile,
                    Carried::default(),
                ))
                .id();
            // Someone else stays when the selected member leaves.
            let other = app
                .world_mut()
                .spawn(Character::new("B", Race::Human, Gender::Male, Profession::Fighter))
                .id();
            let party = crate::game::party::roster::test_party(app.world_mut(), &[member, other]);
            if full {
                let mut ids = ItemIds::default();
                // Use distinct identities from the held item.
                ids.allocate();
                let mut carried = app.world_mut().get_mut::<Carried>(member).unwrap();
                for slot in &mut carried.backpack {
                    *slot = Some(ItemStack::new(&definition, &mut ids, 1, true));
                }
                app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks = (0
                    ..crate::game::inventory::POOL_LIMIT)
                    .map(|_| ItemStack::new(&definition, &mut ids, 1, true))
                    .collect();
            }
            app.world_mut().get_mut::<PartyInventory>(party).unwrap().hand.held = Some(item);
            app.world_mut().resource_mut::<super::super::main_screen::SelectedMember>().0 =
                Some(member);
            app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Inventory);
            app.finish();
            app.cleanup();
            app.update();
            if !still_a_member {
                app.world_mut()
                    .entity_mut(member)
                    .remove::<crate::game::party::roster::PartyMember>();
            }
            if explicit_close {
                key(&mut app, window, KeyCode::Escape, true);
            } else {
                // The artwork-failure path only hides the screen. It must not
                // implicitly accept an inventory operation through OnExit.
                app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::World);
            }
            app.update();
            assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::World);
            if !explicit_close || full || !loaded {
                assert_eq!(
                    app.world()
                        .get::<PartyInventory>(party)
                        .unwrap()
                        .hand
                        .held
                        .as_ref()
                        .unwrap()
                        .id,
                    id
                );
                assert_eq!(
                    app.world().get::<Carried>(member).unwrap().backpack.iter().flatten().count(),
                    if full { BACKPACK_SLOTS } else { 0 }
                );
                assert_eq!(
                    app.world().get::<PartyInventory>(party).unwrap().pool.stacks.len(),
                    if full { crate::game::inventory::POOL_LIMIT } else { 0 }
                );
                continue;
            }
            assert!(app.world().get::<PartyInventory>(party).unwrap().hand.held.is_none());
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
                    .get::<PartyInventory>(party)
                    .unwrap()
                    .pool
                    .stacks
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>(),
                if still_a_member { vec![] } else { vec![id] }
            );
        }
    }

    #[test]
    fn review_acceptance_selects_its_member_and_refusal_preserves_selection() {
        use super::super::main_screen::SelectedMember;
        use bevy::ecs::system::RunSystemOnce;
        let mut app = crate::presentation::test_input::input_app();
        app.init_state::<Screen>()
            .init_resource::<SelectedMember>()
            .add_input_context::<InventoryRoot>()
            .add_systems(OnEnter(Screen::Inventory), enter_inventory);
        let first = app.world_mut().spawn_empty().id();
        let second = app.world_mut().spawn_empty().id();
        let outsider = app.world_mut().spawn_empty().id();
        crate::game::party::roster::test_party(app.world_mut(), &[first, second]);
        app.world_mut().resource_mut::<SelectedMember>().0 = Some(first);
        app.finish();
        app.cleanup();
        app.update();
        let open = |app: &mut App, member| {
            app.world_mut()
                .run_system_once(move |mut review: OpenReview| review.open(member))
                .unwrap()
        };
        assert!(!open(&mut app, outsider));
        assert_eq!(app.world().resource::<SelectedMember>().0, Some(first));
        assert!(matches!(*app.world().resource::<NextState<Screen>>(), NextState::Unchanged));
        assert!(open(&mut app, second));
        assert_eq!(app.world().resource::<SelectedMember>().0, Some(second));
        // A second request cannot redirect the accepted opening.
        assert!(!open(&mut app, first));
        app.update();
        let mut screens = app.world_mut().query::<&mut InventoryScreen>();
        assert_eq!(screens.single(app.world()).unwrap().member, second);
        // Browsing the open inventory stays local; it does not change HUD selection.
        screens.single_mut(app.world_mut()).unwrap().member = first;
        assert_eq!(app.world().resource::<SelectedMember>().0, Some(second));
    }

    /// A travel question owns the keyboard: I cannot open the review under it.
    #[test]
    fn inventory_key_waits_for_an_open_travel_question() {
        use crate::game::characters::{Gender, Profession, Race};
        use crate::game::world::level::TravelQuestion;
        use crate::presentation::test_input::key;
        let mut app = inventory_app();
        let window = app
            .world_mut()
            .spawn((bevy::window::Window::default(), bevy::window::PrimaryWindow))
            .id();
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let member =
            app.world_mut().spawn((Vitals::new(&profile), profile, Carried::default())).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.finish();
        app.cleanup();
        app.update();
        // Without artwork the screen returns to the world, so look at it as
        // the key goes down.
        let opens = |app: &mut App| {
            key(app, window, KeyCode::KeyI, true);
            let opened = *app.world().resource::<State<Screen>>().get() == Screen::Inventory;
            key(app, window, KeyCode::KeyI, false);
            app.update();
            opened
        };
        let question = app.world_mut().spawn(TravelQuestion { entrance: "gate".into() }).id();
        app.update();
        assert!(!opens(&mut app));
        app.world_mut().despawn(question);
        app.update();
        assert!(opens(&mut app));
    }

    #[test]
    fn review_and_information_dialog_keep_keyboard_navigation() {
        use crate::game::characters::{Gender, Profession, Race};
        use crate::presentation::test_input::{key, mouse, tap};
        use bevy::input_focus::{
            InputDispatchPlugin, InputFocus, tab_navigation::TabNavigationPlugin,
        };
        use bevy::picking::{
            backend::HitData,
            events::{PointerClick, PointerPress},
            pointer::PointerButton,
        };
        use bevy::ui_widgets::{
            ButtonPlugin, ModalDialogPlugin, SelectedTab, SliderPlugin, TabPlugin,
        };
        use bevy::window::{PrimaryWindow, Window};
        let mut app = inventory_app();
        app.add_plugins((
            InputDispatchPlugin,
            TabNavigationPlugin,
            bevy::input_focus::pointer_focus::PointerFocusPlugin,
            TabPlugin,
            SliderPlugin,
            ButtonPlugin,
            ModalDialogPlugin,
            bevy::camera::visibility::VisibilityPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        let profile = Character::new("Test", Race::Human, Gender::Male, Profession::Fighter);
        let member =
            app.world_mut().spawn((Vitals::new(&profile), profile, Carried::default())).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.world_mut().resource_mut::<super::super::main_screen::SelectedMember>().0 =
            Some(member);
        app.finish();
        app.cleanup();
        app.update();
        let frame =
            crate::content::ui::Frame { offset: [0, 0], width: 1, height: 1, rgba: vec![255; 4] };
        let sheet = app.world_mut().resource_mut::<Assets<SpriteSheet>>().add(SpriteSheet {
            frames: vec![frame; 100],
            images: vec![Handle::default(); 100],
            palette: None,
        });
        let strings = app
            .world_mut()
            .resource_mut::<Assets<crate::content::ui::TextTable>>()
            .add(crate::content::ui::TextTable(vec![Some(String::new()); 0xa00]));
        app.insert_resource(crate::presentation::screen_art::StringTable(strings));
        app.world_mut().insert_resource(InventoryArt {
            corner: sheet.clone(),
            page: sheet.clone(),
            bottom_bar: sheet.clone(),
            gold_icon: sheet.clone(),
            exit: sheet.clone(),
            font: sheet,
            ..default()
        });
        tap(&mut app, window, KeyCode::KeyI);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Inventory);
        let pool_cell = app
            .world_mut()
            .query::<(Entity, &ItemCell)>()
            .iter(app.world())
            .find_map(|(entity, cell)| matches!(cell.cell, Cell::Pool(0)).then_some(entity))
            .unwrap();
        let item_detail = app
            .world_mut()
            .query::<(Entity, &info::InfoTarget)>()
            .iter(app.world())
            .find_map(|(entity, target)| {
                matches!(target.subject, info::Subject::Secondary(_)).then_some(entity)
            })
            .unwrap();
        for entity in [pool_cell, item_detail] {
            assert!(app.world().get::<InheritedVisibility>(entity).unwrap().get());
        }
        let initial =
            app.world().resource::<InputFocus>().get().expect("opening review focuses a control");
        tap(&mut app, window, KeyCode::Tab);
        let next =
            app.world().resource::<InputFocus>().get().expect("Tab keeps focus inside review");
        assert_ne!(initial, next);
        let skill_tab = app
            .world_mut()
            .query::<(Entity, &review::PageButton)>()
            .iter(app.world())
            .find_map(|(entity, tab)| (tab.0 == review::Page::Skills).then_some(entity))
            .unwrap();
        key(&mut app, window, KeyCode::ShiftLeft, true);
        tap(&mut app, window, KeyCode::Tab);
        key(&mut app, window, KeyCode::ShiftLeft, false);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(initial));
        tap(&mut app, window, KeyCode::ArrowRight);
        tap(&mut app, window, KeyCode::ArrowRight);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(skill_tab));
        tap(&mut app, window, KeyCode::Enter);
        let list = app
            .world_mut()
            .query_filtered::<&SelectedTab, With<review::PageTabs>>()
            .single(app.world())
            .unwrap();
        assert_eq!(list.0, Some(skill_tab));
        for entity in [pool_cell, item_detail] {
            assert!(!app.world().get::<InheritedVisibility>(entity).unwrap().get());
        }
        let target = app
            .world_mut()
            .query::<(Entity, &info::InfoTarget)>()
            .iter(app.world())
            .find_map(|(entity, target)| {
                matches!(
                    target.subject,
                    info::Subject::Skill(crate::content::characters::Skill::Sword)
                )
                .then_some(entity)
            })
            .unwrap();
        app.world_mut().trigger(PointerPress {
            entity: target,
            pointer: mouse(),
            button: PointerButton::Secondary,
            hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
            count: 1,
        });
        app.world_mut().trigger(PointerClick {
            entity: target,
            pointer: mouse(),
            button: PointerButton::Secondary,
            hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
            duration: std::time::Duration::ZERO,
            count: 1,
        });
        app.update();
        let dialog = app
            .world_mut()
            .query_filtered::<Entity, With<info::InfoDialog>>()
            .single(app.world())
            .unwrap();
        let close = app.world().resource::<InputFocus>().get().expect("popup takes focus");
        tap(&mut app, window, KeyCode::Tab);
        let after_tab = app.world().resource::<InputFocus>().get().unwrap();
        assert_ne!(close, after_tab);
        key(&mut app, window, KeyCode::ShiftLeft, true);
        tap(&mut app, window, KeyCode::Tab);
        key(&mut app, window, KeyCode::ShiftLeft, false);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(close));
        tap(&mut app, window, KeyCode::Escape);
        assert!(app.world().get_entity(dialog).is_err());
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(skill_tab));
        tap(&mut app, window, KeyCode::Tab);
        assert!(app.world().resource::<InputFocus>().get().is_some());
    }
}
