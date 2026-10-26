//! The party's slots down the sides of the screen (retail
//! `RedrawPartyPortraitOverlay`): portraits in Closed, then each member's
//! hands, bars, name, armour, condition and the selection rim; damage
//! splats over the portraits. Clicking a slot selects the member, gives
//! them the held item, or (double click) opens their review screen.

use std::time::Duration;

use bevy::prelude::*;
use rand::Rng;

use super::{LayoutMode, MainScreenArt, ScreenLayout, SelectedMember, layer, loaded};
use crate::content::characters::BodyPart;
use crate::content::items::ItemCatalog;
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Race};
use crate::game::combat::attack::{Armour, armour};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot, Given, ItemNews, PartyInventory};
use crate::game::party::roster::PartyRoster;
use crate::presentation::PresentationRng;
use crate::presentation::screen_art::{absolute, rect, sprite, text};

/// Where a slot's row starts: four rows of 85 pixels from y 18.
fn row_y(slot: usize) -> f32 {
    18.0 + 85.0 * (slot / 2) as f32
}

/// A slot's column in Closed: even slots on the left, odd on the right.
fn menu_x(slot: usize) -> f32 {
    if slot.is_multiple_of(2) { 0.0 } else { 512.0 }
}

/// A slot's portrait, under the column art (Closed only).
#[derive(Component, Clone, Default)]
pub struct SlotPortrait(usize);

/// A slot's overlay on the column art, with what it last drew.
#[derive(Component, Clone, Default)]
pub struct PartySlot {
    slot: usize,
    drawn: Option<Option<SlotView>>,
}

/// The sheets a slot shows: holding their handles keeps them loaded while
/// they are shown.
#[derive(Component)]
pub struct HeldSheets {
    _kept: Vec<Handle<SpriteSheet>>,
}

/// Holds the portraits' damage splats.
#[derive(Component, Clone, Default)]
pub struct Splats;

pub fn portraits() -> impl SceneList {
    (0..8_usize).map(|slot| bsn! { @layer() SlotPortrait({slot}) }).collect::<Vec<_>>()
}

pub fn slots(closed: bool) -> impl SceneList {
    let slots: Vec<_> =
        (0..8_usize).map(|slot| bsn! { @layer() PartySlot { slot: {slot} } }).collect();
    (slots, closed.then(|| bsn! { @layer() Splats }))
}

/// What one party slot shows of its member.
#[derive(Clone, Debug, PartialEq)]
struct SlotView {
    member: Entity,
    portrait: String,
    race: Race,
    dead: bool,
    hp: u32,
    hp_fill: u32,
    stamina_fill: u32,
    name: String,
    armour: i32,
    /// The worst condition's icon (`data/icons/conditions`), if any.
    condition: Option<String>,
    /// The main and off hand's item pictures, or the race's empty hand.
    hands: [Option<String>; 2],
    two_handed: bool,
    selected: bool,
}

impl SlotView {
    fn of(
        member: Entity,
        character: &Character,
        carried: Option<&Carried>,
        catalog: Option<&ItemCatalog>,
        selected: bool,
    ) -> Self {
        let item = |equip| {
            let def = carried?.worn(equip).and_then(|i| catalog?.get(i.item))?;
            Some((def.image.clone(), def.two_handed))
        };
        let (main, off) = (item(EquipSlot::Primary), item(EquipSlot::Secondary));
        let armour = match (carried, catalog) {
            (Some(carried), Some(catalog)) => average_armour(&armour(character, carried, catalog)),
            _ => 0,
        };
        SlotView {
            member,
            portrait: character.portrait.clone(),
            race: character.race,
            dead: character.is_dead(),
            hp: character.hp,
            hp_fill: fill(character.hp, character.max_hp()),
            stamina_fill: fill(character.stamina, character.max_stamina()),
            name: character.name.clone(),
            armour,
            condition: condition_icon(character),
            two_handed: main.as_ref().is_some_and(|m| m.1),
            hands: [main.map(|m| m.0), off.map(|o| o.0)],
            selected,
        }
    }
}

/// The worst condition's icon name, `dead` for the dead.
pub fn condition_icon(character: &Character) -> Option<String> {
    if character.is_dead() {
        Some("dead".into())
    } else {
        character.worst_condition().map(|c| format!("{c:?}").to_lowercase())
    }
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

/// The displayed armour class: the locations' values weighted by how
/// often blows land there, in percent, rounded (`characters.md` §4.8).
fn average_armour(armour: &Armour) -> i32 {
    let weight = |part| match part {
        BodyPart::Head => 15,
        BodyPart::Body => 40,
        BodyPart::Leg => 30,
        BodyPart::Hand => 10,
        BodyPart::Foot => 5,
    };
    let total: i32 = armour.by_location.iter().map(|(part, a)| a * weight(part)).sum();
    (total as f32 / 100.0).round() as i32
}

/// Draw each slot again when what it shows has changed, or when sheets it
/// needed had not loaded yet.
pub fn draw_party(
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
    mut slots: Query<(Entity, &mut PartySlot)>,
    portraits: Query<(Entity, &SlotPortrait)>,
) {
    if slots.is_empty() {
        return;
    }
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    let closed = layout.mode == LayoutMode::Closed;
    let (info, font, small_font) = (
        loaded(&sheets, &art.column_info),
        loaded(&sheets, &art.font),
        loaded(&sheets, &art.small_font),
    );
    for (overlay, mut party_slot) in &mut slots {
        let slot = party_slot.slot;
        let view = roster.0.get_index(slot).and_then(|&member| {
            members.get(member).ok().map(|(c, carried)| {
                SlotView::of(member, c, carried, catalog, selected.member(&roster) == Some(member))
            })
        });
        if party_slot.drawn.as_ref() == Some(&view) {
            continue;
        }
        let portrait = portraits.iter().find(|(_, p)| p.0 == slot).map(|(e, _)| e);
        let mut pictures: Vec<Box<dyn Scene>> = vec![];
        let mut parts: Vec<Box<dyn Scene>> = vec![];
        let mut held = vec![];
        let mut complete = true;
        let mut sheet = |path: String| {
            let handle = assets.load(format!("retail://{path}"));
            let sheet = sheets.get(&handle);
            complete &= sheet.is_some();
            held.push(handle);
            sheet
        };
        let (y, left) = (row_y(slot), slot % 2 == 0);
        if portrait.is_some() {
            let at = Vec2::new(menu_x(slot) + 20.0, y);
            match &view {
                None => {
                    let back = sheet("data/main interface/pback.sti".into());
                    pictures.push(Box::new(back.and_then(|s| sprite(s, 0, at))));
                    let dragons = sheet("data/main interface/dragons.sti".into());
                    let at = at + Vec2::X * 3.0;
                    pictures.push(Box::new(dragons.and_then(|s| sprite(s, 0, at))));
                }
                Some(view) if view.dead => {
                    let path = format!("data/portraits/medium/{}.sti", skull(view.race));
                    let skull = sheet(path).and_then(|s| sprite(s, 0, at));
                    // Mirrored on the right.
                    pictures.push(Box::new(bsn! { @{skull} ImageNode { flip_x: {!left} } }));
                }
                Some(view) => {
                    let path = format!("data/portraits/medium/m{}.sti", view.portrait);
                    pictures.push(Box::new(sheet(path).and_then(|s| sprite(s, 0, at))));
                }
            }
        }
        if let Some(view) = &view {
            let bars_x = match (closed, left) {
                (true, true) => 105.0,
                (true, false) => 512.0,
                (false, true) => 0.0,
                (false, false) => 617.0,
            };
            // Wide and Full show only the bars.
            if closed {
                let hands_x = if left { 2.0 } else { 619.0 };
                let frame = if view.two_handed { 2 } else { 0 };
                parts.push(Box::new(sprite(info, frame, Vec2::new(hands_x, y))));
                if !view.dead {
                    let [upper, lower] = empty_hands(view.race);
                    for (hand, (dy, empty)) in view.hands.iter().zip([(23.0, upper), (47.0, lower)])
                    {
                        if dy == 47.0 && view.two_handed {
                            continue;
                        }
                        let part = match hand {
                            Some(path) => sheet(path.clone())
                                .and_then(|s| sprite(s, 2, Vec2::new(hands_x + 3.0, y + dy))),
                            None => sheet(format!("data/main interface/{empty}.sti"))
                                .and_then(|s| sprite(s, 0, Vec2::new(hands_x + 4.0, y + dy))),
                        };
                        parts.push(Box::new(part));
                    }
                    let armour = view.armour.to_string();
                    let at = Vec2::new(hands_x + 3.0, y + 3.0);
                    parts.push(Box::new(text(small_font, &mut images, &armour, at, 13.0)));
                }
                let at = Vec2::new(menu_x(slot) + 21.0, y + 73.0);
                parts.push(Box::new(text(font, &mut images, &view.name, at, 84.0)));
            }
            // Retail's default shows hit points as a number, with wide bars
            // (`g_settings.numeric_hit_points`, on in `ResetGameplaySettings`).
            parts.push(Box::new(sprite(info, 3, Vec2::new(bars_x, y))));
            if !view.dead {
                // The full bar, then its empty part blacked out from the top.
                for (file, dx, fill) in [
                    ("healthbarwide", 3.0, view.hp_fill),
                    ("staminabarwide", 9.0, view.stamina_fill),
                ] {
                    let at = Vec2::new(bars_x + dx, y + 17.0);
                    let bar = sheet(format!("data/main interface/{file}.sti"));
                    parts.push(Box::new(bar.and_then(|s| sprite(s, 0, at))));
                    let empty = Vec2::new(5.0, (45 - fill) as f32);
                    parts.push(Box::new(rect(at, empty, Color::srgb_u8(1, 1, 1))));
                }
                let at = Vec2::new(bars_x + 2.0, y + 62.0);
                parts.push(Box::new(rect(at, Vec2::new(19.0, 9.0), Color::BLACK)));
                let hp = view.hp.to_string();
                parts.push(Box::new(text(small_font, &mut images, &hp, at, 19.0)));
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
                let at = Vec2::new(condition_x, y + 3.0);
                parts.push(Box::new(condition.and_then(|s| sprite(s, 0, at))));
                let enchantment = sheet(format!("data/main interface/{}.sti", plug(enchantment_x)));
                let at = Vec2::new(enchantment_x, y + 3.0);
                parts.push(Box::new(enchantment.and_then(|s| sprite(s, 0, at))));
                if view.condition.is_some() {
                    for x in [condition_x, enchantment_x] {
                        let rim = sheet("data/main interface/condition_highlight.sti".into());
                        let at = Vec2::new(x - 1.0, y + 2.0);
                        parts.push(Box::new(rim.and_then(|s| sprite(s, 0, at))));
                    }
                }
            }
            if view.selected {
                let rim = if closed {
                    sheet("data/main interface/charselect.sti".into())
                        .and_then(|s| sprite(s, 0, Vec2::new(menu_x(slot) + 23.0, y)))
                } else {
                    sheet("data/main interface/edgeselect.sti".into())
                        .and_then(|s| sprite(s, 0, Vec2::new(bars_x + 1.0, y + 2.0)))
                };
                parts.push(Box::new(rim));
            }
            // The portrait (or strip) is the slot's button.
            let (at, size) = if closed {
                (Vec2::new(menu_x(slot) + 23.0, y), Vec2::new(82.0, 72.0))
            } else {
                (Vec2::new(if left { 0.0 } else { 617.0 }, y), Vec2::new(22.0, 85.0))
            };
            let node = absolute(at, size);
            parts.push(Box::new(bsn! { node SlotButton({view.member}) on(press) }));
        }
        commands
            .entity(overlay)
            .despawn_related::<Children>()
            .queue_spawn_related_scenes::<Children>(parts);
        if let Some(portrait) = portrait {
            commands
                .entity(portrait)
                .despawn_related::<Children>()
                .queue_spawn_related_scenes::<Children>(pictures);
        }
        commands.entity(overlay).insert(HeldSheets { _kept: held });
        party_slot.drawn = complete.then_some(view);
    }
}

#[derive(Component, Clone, Copy, FromTemplate)]
struct SlotButton(Entity);

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The last click on a slot and when, for double clicks; the buttons are
/// drawn again as a click selects.
#[derive(Resource, Default)]
pub struct LastSlotClick(Option<(Entity, Duration)>);

/// With an item in hand, a click gives it to the member; otherwise it
/// selects them, and a double click opens their review screen.
fn press(
    click: On<PointerClick>,
    buttons: Query<&SlotButton>,
    mut selected: ResMut<SelectedMember>,
    roster: Res<PartyRoster>,
    time: Res<Time>,
    mut last_click: ResMut<LastSlotClick>,
    (mut party, mut members, data, catalogs, battle, mut news): (
        ResMut<PartyInventory>,
        Query<(&Character, &mut Carried)>,
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        Option<Res<crate::game::combat::Battle>>,
        MessageWriter<ItemNews>,
    ),
    screen: Res<State<crate::game::screen::Screen>>,
    mut next_screen: ResMut<NextState<crate::game::screen::Screen>>,
) {
    let Ok(&SlotButton(member)) = buttons.get(click.entity) else { return };
    if !roster.0.contains(&member) {
        return;
    }
    if party.held.is_some() {
        let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
        let Ok((character, mut carried)) = members.get_mut(member) else {
            return;
        };
        let item = party
            .held
            .as_ref()
            .and_then(|h| catalog.get(h.item.item).map(|d| h.item.display_name(d, false)))
            .unwrap_or_default();
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
        return;
    }
    selected.0 = Some(member);
    let now = time.elapsed();
    if last_click.0.is_some_and(|(s, at)| s == member && now - at < DOUBLE_CLICK) {
        if *screen.get() == crate::game::screen::Screen::World {
            next_screen.set(crate::game::screen::Screen::Inventory);
        }
    }
    last_click.0 = Some((member, now));
}

/// A splat playing over a member's portrait.
#[derive(Debug, Clone)]
struct Splat {
    slot: usize,
    /// Its frames: 8 for damage, 30 for death.
    timer: Timer,
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
    mut rng: ResMut<PresentationRng>,
    mut state: Local<SplatState>,
) {
    let health: Vec<u32> = roster.0.iter().map(|&m| members.get(m).map_or(0, |c| c.hp)).collect();
    if state.health.len() == health.len() {
        for (slot, (&before, &after)) in state.health.clone().iter().zip(&health).enumerate() {
            if after < before {
                let death = after == 0;
                let frames = if death { 30.0 } else { 8.0 };
                state.playing.retain(|s| s.slot != slot);
                state.playing.push(Splat {
                    slot,
                    timer: Timer::from_seconds(frames * SPLAT_FRAME, TimerMode::Once),
                    damage: before - after,
                    death,
                    second: rng.0.random_bool(0.5),
                });
            }
        }
    }
    state.health = health;
    for splat in &mut state.playing {
        splat.timer.tick(time.delta());
    }
    state.playing.retain(|s| !s.timer.is_finished());
    let Ok(layer) = layers.single() else { return };
    let mut parts: Vec<Box<dyn Scene>> = vec![];
    for splat in &state.playing {
        let frame = (splat.timer.elapsed_secs() / SPLAT_FRAME) as usize;
        let handle = match (splat.death, splat.second) {
            (true, _) => &art.death_splat,
            (false, false) => &art.damage_splat,
            (false, true) => &art.damage_splat2,
        };
        let x = if splat.slot.is_multiple_of(2) { 22.0 } else { 533.0 };
        let at = Vec2::new(x, row_y(splat.slot));
        parts.push(Box::new(sprite(loaded(&sheets, handle), frame, at)));
        if frame < 6 {
            let font = loaded(&sheets, &art.bold_font);
            let damage = splat.damage.to_string();
            parts.push(Box::new(text(font, &mut images, &damage, at + Vec2::new(0.0, 4.0), 82.0)));
        }
    }
    commands
        .entity(layer)
        .despawn_related::<Children>()
        .queue_spawn_related_scenes::<Children>(parts);
}
