//! The party's slots down the sides of the screen (retail
//! `RedrawPartyPortraitOverlay`): portraits in Closed, then each member's
//! hands, bars, name, armour, condition and the selection rim; damage
//! splats over the portraits. Clicking a slot selects the member, gives
//! them the held item, or (double click) opens their review screen.

use crate::presentation::character_view::{CharacterView, CharacterViewItem};
use std::time::Duration;

use bevy::picking::events::{PointerClick, PointerOut, PointerPress, PointerRelease};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use rand::Rng;

use super::{LayoutMode, MainScreenArt, ScreenLayout, SelectedMember, layer};
use crate::content::items::ItemCatalog;
use crate::content::ui::SpriteSheet;
#[cfg(test)]
use crate::game::characters::Race;
use crate::game::characters::{Character, Vitals};
use crate::game::combat::attack::armour;
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot, ItemNews};
use crate::game::party::DamageApplied;
use crate::game::party::roster::PartyRoster;
use crate::presentation::PresentationRng;
use crate::presentation::screen_art::{
    BitmapText, Picture, absolute, condition_icon, label, picture, rect, skull,
};

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

/// A party slot is a location in this layout, not actor identity.
#[derive(Component, Clone, Default)]
pub struct PartySlot {
    slot: usize,
}

#[derive(Component)]
pub struct PartyWidgetsBuilt;

#[derive(Component, Clone, Default)]
pub struct PartyButton(usize);

#[derive(Clone, Copy, Default)]
enum Part {
    #[default]
    Portrait,
    EmptyDragon,
    Bars,
    Health,
    Stamina,
    HealthEmpty,
    StaminaEmpty,
    Alive,
    Hands,
    MainHand,
    OffHand,
    Armour,
    Name,
    HitPoints,
    Condition,
    Enchantment,
    ConditionRim,
    Selection,
}

#[derive(Component, Clone, Default)]
pub struct PartyPart {
    slot: usize,
    kind: Part,
}

fn part_picture(slot: usize, kind: Part, at: Vec2) -> impl Scene {
    bsn! { @picture(default(), 0, at) template(move |_| Ok(PartyPart { slot, kind })) }
}

fn part_label(
    slot: usize,
    kind: Part,
    font: Handle<SpriteSheet>,
    at: Vec2,
    width: f32,
) -> impl Scene {
    bsn! { @label(font, at, width) template(move |_| Ok(PartyPart { slot, kind })) }
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

/// A bar's fill, 0 to 45 pixels, at least 1 while any is left (retail
/// `SyncPartyPortraitVitalsBars`).
fn fill(now: u32, max: u32) -> u32 {
    if now == 0 || max == 0 { 0 } else { (now * 45 / max).max(1) }
}

/// Create controls once for this layout. Gameplay updates their properties.
pub fn build_party_parts(
    mut commands: Commands,
    layout: Res<ScreenLayout>,
    art: Res<MainScreenArt>,
    roots: Query<
        (Entity, Option<&PartySlot>, Option<&SlotPortrait>),
        (Or<(With<PartySlot>, With<SlotPortrait>)>, Without<PartyWidgetsBuilt>),
    >,
) {
    let closed = layout.mode == LayoutMode::Closed;
    for (root, overlay, portrait) in &roots {
        let slot = overlay.map_or_else(|| portrait.unwrap().0, |slot| slot.slot);
        let (y, left) = (row_y(slot), slot % 2 == 0);
        let mut parts: Vec<Box<dyn Scene>> = vec![];
        if portrait.is_some() {
            parts.push(Box::new(part_picture(
                slot,
                Part::Portrait,
                Vec2::new(menu_x(slot) + 20.0, y),
            )));
            parts.push(Box::new(part_picture(
                slot,
                Part::EmptyDragon,
                Vec2::new(menu_x(slot) + 23.0, y),
            )));
        } else {
            let bars = match (closed, left) {
                (true, true) => 105.0,
                (true, false) => 512.0,
                (false, true) => 0.0,
                (false, false) => 617.0,
            };
            parts.push(Box::new(part_picture(slot, Part::Bars, Vec2::new(bars, y))));
            for (kind, mask, dx) in
                [(Part::Health, Part::HealthEmpty, 3.0), (Part::Stamina, Part::StaminaEmpty, 9.0)]
            {
                let at = Vec2::new(bars + dx, y + 17.0);
                parts.push(Box::new(part_picture(slot, kind, at)));
                parts.push(Box::new(bsn! { @rect(at, Vec2::new(5.0, 45.0), Color::srgb_u8(1,1,1))
                template(move |_| Ok(PartyPart { slot, kind: mask })) }));
            }
            let at = Vec2::new(bars + 2.0, y + 62.0);
            parts.push(Box::new(bsn! { @rect(at, Vec2::new(19.0,9.0), Color::BLACK)
            template(move |_| Ok(PartyPart { slot, kind: Part::Alive })) }));
            parts.push(Box::new(part_label(
                slot,
                Part::HitPoints,
                art.small_font.clone(),
                at,
                19.0,
            )));
            let selected_at = if closed {
                Vec2::new(menu_x(slot) + 23.0, y)
            } else {
                Vec2::new(bars + 1.0, y + 2.0)
            };
            parts.push(Box::new(part_picture(slot, Part::Selection, selected_at)));
            if closed {
                let hands = if left { 2.0 } else { 619.0 };
                parts.push(Box::new(part_picture(slot, Part::Hands, Vec2::new(hands, y))));
                parts.push(Box::new(part_picture(
                    slot,
                    Part::MainHand,
                    Vec2::new(hands + 3.0, y + 23.0),
                )));
                parts.push(Box::new(part_picture(
                    slot,
                    Part::OffHand,
                    Vec2::new(hands + 3.0, y + 47.0),
                )));
                parts.push(Box::new(part_label(
                    slot,
                    Part::Armour,
                    art.small_font.clone(),
                    Vec2::new(hands + 3.0, y + 3.0),
                    13.0,
                )));
                parts.push(Box::new(part_label(
                    slot,
                    Part::Name,
                    art.font.clone(),
                    Vec2::new(menu_x(slot) + 21.0, y + 73.0),
                    84.0,
                )));
                let (condition, enchantment) = if left {
                    (menu_x(slot) + 25.0, menu_x(slot) + 88.0)
                } else {
                    (menu_x(slot) + 88.0, menu_x(slot) + 25.0)
                };
                parts.push(Box::new(part_picture(
                    slot,
                    Part::Condition,
                    Vec2::new(condition, y + 3.0),
                )));
                parts.push(Box::new(part_picture(
                    slot,
                    Part::Enchantment,
                    Vec2::new(enchantment, y + 3.0),
                )));
                for x in [condition, enchantment] {
                    parts.push(Box::new(part_picture(
                        slot,
                        Part::ConditionRim,
                        Vec2::new(x - 1.0, y + 2.0),
                    )));
                }
            }
            let (at, size) = if closed {
                (Vec2::new(menu_x(slot) + 23.0, y), Vec2::new(82.0, 72.0))
            } else {
                (Vec2::new(if left { 0.0 } else { 617.0 }, y), Vec2::new(22.0, 85.0))
            };
            let node = absolute(at, size);
            parts.push(Box::new(
                bsn! {
                    node PartyButton({slot}) SlotButton { member: Entity::PLACEHOLDER, review_armed: false }
                    on(press) on(press_right) on(release_right) on(leave)
                },
            ));
        }
        commands
            .entity(root)
            .insert(PartyWidgetsBuilt)
            .queue_spawn_related_scenes::<Children>(parts);
    }
}

pub fn draw_party(
    layout: Res<ScreenLayout>,
    selected: Res<SelectedMember>,
    art: Res<MainScreenArt>,
    assets: Res<AssetServer>,
    roster: Res<PartyRoster>,
    members: Query<CharacterView>,
    mut removed_carried: RemovedComponents<Carried>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    mut parts: Query<(
        Ref<PartyPart>,
        &mut Node,
        &mut Visibility,
        Option<&mut Picture>,
        Option<&mut BitmapText>,
        Option<&mut ImageNode>,
    )>,
) {
    let removed: Vec<_> = removed_carried.read().collect();
    let catalog = data.as_ref().and_then(|d| catalogs.get(&d.items));
    let closed = layout.mode == LayoutMode::Closed;
    for (part, mut node, mut visible, picture, label, image_node) in &mut parts {
        let member = roster.0.get_index(part.slot).copied();
        let actor = member.and_then(|entity| members.get(entity).ok());
        if !part.is_added()
            && !roster.is_changed()
            && !selected.is_changed()
            && !catalogs.is_changed()
            && !member.is_some_and(|m| removed.contains(&m))
            && actor.as_ref().is_none_or(|view| !view.is_changed())
        {
            continue;
        }
        let mut shown = actor.is_some();
        let mut source = None;
        let mut value = None;
        let mut hand_x = None;
        let image = |path: &str, frame| Picture {
            sheet: assets.load(format!("retail://{path}")),
            frame,
            ..default()
        };
        if let Some(CharacterViewItem { profile, vitals, carried }) = actor {
            let dead = vitals.is_dead();
            let worn = |slot| {
                carried.as_ref().and_then(|c| c.worn(slot)).and_then(|i| catalog?.get(i.item))
            };
            let two_handed = worn(EquipSlot::Primary).is_some_and(|d| d.two_handed);
            let condition = condition_icon(&vitals);
            match part.kind {
                Part::Portrait => {
                    source = Some(image(
                        &if dead {
                            format!("data/portraits/medium/{}.sti", skull(profile.race))
                        } else {
                            format!("data/portraits/medium/m{}.sti", profile.portrait)
                        },
                        0,
                    ))
                }
                Part::EmptyDragon => shown = false,
                Part::Bars => {
                    source = Some(Picture { sheet: art.column_info.clone(), frame: 3, ..default() })
                }
                Part::Health | Part::Stamina => {
                    shown = !dead;
                    source = Some(image(
                        if matches!(part.kind, Part::Health) {
                            "data/main interface/healthbarwide.sti"
                        } else {
                            "data/main interface/staminabarwide.sti"
                        },
                        0,
                    ));
                }
                Part::HealthEmpty | Part::StaminaEmpty => {
                    shown = !dead;
                    let amount = if matches!(part.kind, Part::HealthEmpty) {
                        fill(vitals.hp, profile.max_hp())
                    } else {
                        fill(vitals.stamina, profile.max_stamina())
                    };
                    let height = px(45 - amount);
                    if node.height != height {
                        node.height = height;
                    }
                }
                Part::Alive => shown = !dead,
                Part::Hands => {
                    source = Some(Picture {
                        sheet: art.column_info.clone(),
                        frame: if two_handed { 2 } else { 0 },
                        ..default()
                    })
                }
                Part::MainHand | Part::OffHand => {
                    let off = matches!(part.kind, Part::OffHand);
                    shown = !dead && !(off && two_handed);
                    let hand = if off { EquipSlot::Secondary } else { EquipSlot::Primary };
                    let item = worn(hand);
                    source = Some(match item {
                        Some(d) => image(&d.image, 2),
                        None => image(
                            &format!(
                                "data/main interface/{}.sti",
                                crate::presentation::screen_art::empty_hands(profile.race)
                                    [usize::from(off)]
                            ),
                            0,
                        ),
                    });
                    hand_x = Some(
                        (if part.slot % 2 == 0 { 2.0 } else { 619.0 })
                            + if item.is_some() { 3.0 } else { 4.0 },
                    );
                }
                Part::Armour => {
                    shown = !dead;
                    value = Some(
                        match (carried.as_ref(), catalog) {
                            (Some(c), Some(catalog)) => armour(&profile, c, catalog).average(),
                            _ => 0,
                        }
                        .to_string(),
                    );
                }
                Part::Name => value = Some(profile.name.clone()),
                Part::HitPoints => {
                    shown = !dead;
                    value = Some(vitals.hp.to_string());
                }
                Part::Condition | Part::Enchantment => {
                    let plug = if part.slot % 2 == 0 { "leftplug" } else { "rightplug" };
                    source = Some(image(
                        &if matches!(part.kind, Part::Condition) {
                            condition
                                .as_ref()
                                .map(|c| format!("data/icons/conditions/{c}.sti"))
                                .unwrap_or_else(|| format!("data/main interface/{plug}.sti"))
                        } else {
                            format!("data/main interface/{plug}.sti")
                        },
                        0,
                    ));
                }
                Part::ConditionRim => {
                    shown = condition.is_some();
                    source = Some(image("data/main interface/condition_highlight.sti", 0));
                }
                Part::Selection => {
                    shown = member == selected.member(&roster);
                    source = Some(image(
                        if closed {
                            "data/main interface/charselect.sti"
                        } else {
                            "data/main interface/edgeselect.sti"
                        },
                        0,
                    ));
                }
            }
        } else {
            match part.kind {
                Part::Portrait => {
                    shown = true;
                    source = Some(image("data/main interface/pback.sti", 0));
                }
                Part::EmptyDragon => {
                    shown = true;
                    source = Some(image("data/main interface/dragons.sti", 0));
                }
                _ => {}
            }
        }
        visible.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        if let Some(mut picture) = picture
            && let Some(mut source) = source
        {
            source.at = picture.at;
            if let Some(x) = hand_x {
                source.at.x = x;
            }
            picture.set_if_neq(source);
        }
        if let Some(mut label) = label
            && let Some(value) = value
            && label.value != value
        {
            label.value = value;
        }
        if let Some(mut image) = image_node {
            let flip = matches!(part.kind, Part::Portrait)
                && part.slot % 2 == 1
                && member
                    .and_then(|e| members.get(e).ok())
                    .is_some_and(|view| view.vitals.is_dead());
            if image.flip_x != flip {
                image.flip_x = flip;
            }
        }
    }
}

/// A control keeps its identity across display changes, but never changes the
/// actor it targets while a pointer press could be in flight.
pub fn bind_party_buttons(
    mut commands: Commands,
    roster: Res<PartyRoster>,
    members: Query<(), (With<Character>, With<Vitals>)>,
    mut buttons: Query<(Entity, &PartyButton, &SlotButton, &Node, &ChildOf, &mut Visibility)>,
) {
    for (entity, slot, button, node, parent, mut visibility) in &mut buttons {
        let member = roster
            .0
            .get_index(slot.0)
            .copied()
            .filter(|e| members.contains(*e))
            .unwrap_or(Entity::PLACEHOLDER);
        let shown =
            if member == Entity::PLACEHOLDER { Visibility::Hidden } else { Visibility::Inherited };
        if button.member != member {
            commands.entity(entity).despawn();
            commands
                .spawn((
                    PartyButton(slot.0),
                    SlotButton { member, review_armed: false },
                    node.clone(),
                    ChildOf(parent.parent()),
                    shown,
                ))
                .observe(press)
                .observe(press_right)
                .observe(release_right)
                .observe(leave);
        } else {
            visibility.set_if_neq(shown);
        }
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq, FromTemplate)]
pub struct SlotButton {
    member: Entity,
    /// Giving an item or rebinding the control is not the first selection click
    /// of a review gesture. Bevy owns pointer/button identity, counts and timing.
    review_armed: bool,
}

/// With an item in hand, a left click gives it to the member; otherwise it
/// selects them, and a double click opens their review screen.
fn press(
    click: On<PointerClick>,
    mut buttons: Query<&mut SlotButton>,
    mut selected: ResMut<SelectedMember>,
    roster: Res<PartyRoster>,
    mut inventory: crate::game::inventory::operations::InventoryOperations,
    mut news: MessageWriter<ItemNews>,
    mut review: crate::presentation::inventory::OpenReview,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok(mut button) = buttons.get_mut(click.entity) else { return };
    let member = button.member;
    if !roster.0.contains(&member) {
        return;
    }
    match inventory.give_to(member) {
        Ok(Some(outcome)) => {
            button.review_armed = false;
            news.write(outcome);
            return;
        }
        Err(reason) => {
            button.review_armed = false;
            info!("giving item refused: {reason:?}");
            return;
        }
        Ok(None) => {}
    }
    let already_selected = selected.0 == Some(member);
    selected.0 = Some(member);
    if already_selected && button.review_armed && click.count > 1 {
        review.open(member);
    }
    button.review_armed = true;
}

/// How long retail's right button must stay down on a portrait to open the
/// review (`SetCountdownClock(1000)` in the portrait region handler).
const REVIEW_HOLD: Duration = Duration::from_secs(1);

/// A right press on a portrait that has not been released or left yet.
#[derive(Component, Debug)]
pub struct RightHold {
    since: Duration,
}

/// Retail's right button on a portrait (`MainGameScreen` portrait region):
/// held for a second it opens the member's review; released sooner it gives
/// the held item, or opens the review with nothing held or Ctrl down.
fn press_right(
    press: On<PointerPress>,
    buttons: Query<(), With<SlotButton>>,
    time: Res<Time<Real>>,
    mut commands: Commands,
) {
    if press.button == PointerButton::Secondary && buttons.contains(press.entity) {
        commands.entity(press.entity).insert(RightHold { since: time.elapsed() });
    }
}

fn release_right(
    release: On<PointerRelease>,
    buttons: Query<(&SlotButton, &RightHold)>,
    time: Res<Time<Real>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut inventory: crate::game::inventory::operations::InventoryOperations,
    mut news: MessageWriter<ItemNews>,
    mut review: crate::presentation::inventory::OpenReview,
    mut selected: ResMut<SelectedMember>,
    mut commands: Commands,
) {
    if release.button != PointerButton::Secondary {
        return;
    }
    let Ok((button, hold)) = buttons.get(release.entity) else { return };
    commands.entity(release.entity).remove::<RightHold>();
    let member = button.member;
    let control =
        keys.is_some_and(|keys| keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]));
    if !control && time.elapsed() - hold.since < REVIEW_HOLD {
        // Retail's right-release give always offers the party pool first
        // (`force_to_party`); this uses the left click's pool rule.
        match inventory.give_to(member) {
            Ok(Some(outcome)) => {
                news.write(outcome);
                return;
            }
            Err(reason) => {
                info!("giving item refused: {reason:?}");
                return;
            }
            Ok(None) => {}
        }
    }
    if review.open(member) {
        selected.set_if_neq(SelectedMember(Some(member)));
    }
}

fn leave(out: On<PointerOut>, mut commands: Commands, holds: Query<(), With<RightHold>>) {
    if holds.contains(out.entity) {
        commands.entity(out.entity).remove::<RightHold>();
    }
}

/// A right press held long enough opens the review; one released elsewhere
/// is forgotten.
pub fn open_held_reviews(
    time: Res<Time<Real>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    holds: Query<(Entity, &SlotButton, &RightHold)>,
    mut review: crate::presentation::inventory::OpenReview,
    mut selected: ResMut<SelectedMember>,
    mut commands: Commands,
) {
    for (entity, button, hold) in &holds {
        if !mouse.as_ref().is_some_and(|mouse| mouse.pressed(MouseButton::Right)) {
            commands.entity(entity).remove::<RightHold>();
        } else if time.elapsed() - hold.since >= REVIEW_HOLD {
            commands.entity(entity).remove::<RightHold>();
            if review.open(button.member) {
                selected.set_if_neq(SelectedMember(Some(button.member)));
            }
        }
    }
}

/// Presentation owns an accepted damage animation, keyed by actor rather
/// than the roster slot where that actor happened to be hit.
#[derive(Component)]
pub struct Splat {
    actor: Entity,
    timer: Timer,
    damage: u32,
    death: bool,
    second: bool,
}

#[derive(Component, Clone, Default)]
pub struct SplatNumber;

const SPLAT_FRAME: f32 = 0.1;

pub fn record_splats(
    mut commands: Commands,
    mut damage: MessageReader<DamageApplied>,
    art: Res<MainScreenArt>,
    layers: Query<Entity, With<Splats>>,
    mut existing: Query<&mut Splat>,
    mut rng: ResMut<PresentationRng>,
) {
    let mut hits = std::collections::BTreeMap::<Entity, (u32, bool)>::new();
    for &DamageApplied { member, amount, died } in damage.read() {
        let (lost, death) = hits.entry(member).or_default();
        *lost += amount;
        *death |= died;
    }
    let Ok(layer) = layers.single() else { return };
    for (actor, (damage, death)) in hits {
        // RecordCharacterDamage accumulates hits while the splat is playing.
        if let Some(mut splat) = existing.iter_mut().find(|s| s.actor == actor) {
            splat.damage += damage;
            splat.death |= death;
            if splat.death {
                splat.timer.set_duration(Duration::from_secs_f32(30.0 * SPLAT_FRAME));
            }
            splat.timer.reset();
            continue;
        }
        let second = rng.0.random_bool(0.5);
        let splat = Splat {
            actor,
            damage,
            death,
            second,
            timer: Timer::from_seconds(
                if death { 30.0 } else { 8.0 } * SPLAT_FRAME,
                TimerMode::Once,
            ),
        };
        let value = damage.to_string();
        let font = art.bold_font.clone();
        commands.spawn_scene(bsn!{
            Node{position_type:PositionType::Absolute} Pickable::IGNORE
            Children [
                template(|_|Ok(Picture::default())) Pickable::IGNORE
                -- Node{position_type:PositionType::Absolute} SplatNumber Pickable::IGNORE
                template(move |_|Ok(BitmapText{palette:None,font:font.clone(),value:value.clone(),at:Vec2::new(0.0,4.0),width:82.0,justify:bevy::text::Justify::Center}))
            ]
        }).insert((splat,ChildOf(layer)));
    }
}

pub fn play_splats(
    mut commands: Commands,
    time: Res<Time>,
    art: Res<MainScreenArt>,
    roster: Res<PartyRoster>,
    mut splats: Query<(Entity, &mut Splat, &mut Node, &Children)>,
    mut pictures: Query<&mut Picture>,
    mut numbers: Query<(&mut Visibility, &mut BitmapText), With<SplatNumber>>,
) {
    for (entity, mut splat, mut node, parts) in &mut splats {
        splat.timer.tick(time.delta());
        let Some(slot) = roster.0.get_index_of(&splat.actor).filter(|_| !splat.timer.is_finished())
        else {
            commands.entity(entity).despawn();
            continue;
        };
        let at = Vec2::new(if slot % 2 == 0 { 22.0 } else { 533.0 }, row_y(slot));
        if node.left != px(at.x) {
            node.left = px(at.x);
        }
        if node.top != px(at.y) {
            node.top = px(at.y);
        }
        let frame = (splat.timer.elapsed_secs() / SPLAT_FRAME) as usize;
        let sheet = match (splat.death, splat.second) {
            (true, _) => &art.death_splat,
            (false, false) => &art.damage_splat,
            (false, true) => &art.damage_splat2,
        };
        for part in parts.iter() {
            if let Ok(mut picture) = pictures.get_mut(part) {
                let at = picture.at;
                picture.set_if_neq(Picture { sheet: sheet.clone(), frame, at, bounds: None });
            }
            if let Ok((mut visible, mut label)) = numbers.get_mut(part) {
                visible.set_if_neq(if frame < 6 {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
                let value = splat.damage.to_string();
                if label.value != value {
                    label.value = value;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        characters::{Gender, Profession},
        inventory::PartyInventory,
        screen::Screen,
    };
    use bevy::picking::backend::HitData;

    /// Retail's right-button portrait gestures: a quick release gives the
    /// held item, or opens the review with nothing held; a held press opens
    /// it after a second.
    #[test]
    fn right_button_gives_or_opens_review_like_retail() {
        use crate::content::items::ItemDefinition;
        use crate::game::inventory::{ItemIds, make_item};
        use crate::game::rng::GameRng;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<ItemCatalog>()
            .init_resource::<PartyRoster>()
            .init_resource::<PartyInventory>()
            .init_resource::<SelectedMember>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<ItemNews>()
            .insert_resource(State::new(Screen::World))
            .init_resource::<NextState<Screen>>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                400,
            )))
            .add_systems(Update, open_held_reviews);
        let definition = ItemDefinition::default();
        let item =
            make_item(&definition, &mut ItemIds::default(), &mut GameRng::seeded(1), false, true);
        let items = app
            .world_mut()
            .resource_mut::<Assets<ItemCatalog>>()
            .add(ItemCatalog(vec![definition]));
        app.insert_resource(GameData { items, loot: default(), species: default() });
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let actor =
            app.world_mut().spawn((Vitals::new(&profile), profile, Carried::default())).id();
        app.world_mut().resource_mut::<PartyRoster>().0.insert(actor);
        let button = app
            .world_mut()
            .spawn(SlotButton { member: actor, review_armed: false })
            .observe(press_right)
            .observe(release_right)
            .id();
        app.update();
        let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
        let right = |app: &mut App, down: bool| {
            let pointer = crate::presentation::test_input::mouse();
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            if down {
                mouse.press(MouseButton::Right);
            } else {
                mouse.release(MouseButton::Right);
            }
            if down {
                app.world_mut().trigger(PointerPress {
                    entity: button,
                    pointer,
                    button: PointerButton::Secondary,
                    hit: hit.clone(),
                    count: 1,
                });
            } else {
                app.world_mut().trigger(PointerRelease {
                    entity: button,
                    pointer,
                    button: PointerButton::Secondary,
                    hit: hit.clone(),
                });
            }
            app.update();
        };
        let opened = |app: &mut App| {
            let next = app.world_mut().resource_mut::<NextState<Screen>>();
            let opened = matches!(*next, NextState::Pending(Screen::Inventory));
            app.world_mut().insert_resource(NextState::<Screen>::Unchanged);
            opened
        };
        // Holding an item: a quick release gives it.
        app.world_mut().resource_mut::<PartyInventory>().hand.held = Some(item);
        right(&mut app, true);
        right(&mut app, false);
        assert!(app.world().resource::<PartyInventory>().hand.held.is_none());
        assert!(!opened(&mut app));
        // Nothing held: a quick release opens the review for that member.
        right(&mut app, true);
        right(&mut app, false);
        assert!(opened(&mut app));
        assert_eq!(app.world().resource::<SelectedMember>().0, Some(actor));
        // Held down for a second opens it without a release.
        right(&mut app, true);
        for _ in 0..3 {
            app.update();
        }
        assert!(opened(&mut app));
        assert!(app.world().get::<RightHold>(button).is_none());
    }

    #[test]
    fn new_damage_refreshes_the_existing_splat_before_playback_expires_it() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<SpriteSheet>()
            .init_resource::<PartyRoster>()
            .init_resource::<PresentationRng>()
            .add_message::<DamageApplied>()
            .add_systems(Startup, super::super::load_main_screen_art)
            .add_systems(Update, (record_splats, play_splats).chain());
        let actor = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<PartyRoster>().0.insert(actor);
        let layer = app.world_mut().spawn(Splats).id();
        let mut timer = Timer::from_seconds(8.0 * SPLAT_FRAME, TimerMode::Once);
        timer.set_elapsed(Duration::from_secs_f32(0.79));
        let splat = app
            .world_mut()
            .spawn((
                Splat { actor, timer, damage: 10, death: false, second: false },
                Node::default(),
                ChildOf(layer),
            ))
            .id();
        app.world_mut().spawn((Picture::default(), ChildOf(splat)));
        app.update();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            100,
        )));
        app.world_mut().write_message(DamageApplied { member: actor, amount: 3, died: false });
        app.world_mut().write_message(DamageApplied { member: actor, amount: 4, died: true });
        app.update();
        let retained =
            app.world().get::<Splat>(splat).expect("refresh must retain the animation entity");
        assert_eq!(retained.damage, 17);
        assert!(retained.death);
        assert!(retained.timer.elapsed_secs() < 0.2);
        assert_eq!(retained.timer.duration(), Duration::from_secs(3));
        assert_eq!(app.world_mut().query::<&Splat>().iter(app.world()).count(), 1);
    }

    #[test]
    fn native_click_counts_review_only_the_current_selection_gesture() {
        let mut world = World::new();
        world.init_resource::<PartyRoster>();
        world.init_resource::<PartyInventory>();
        world.init_resource::<Assets<ItemCatalog>>();
        world.init_resource::<SelectedMember>();
        world.init_resource::<Messages<ItemNews>>();
        world.insert_resource(State::new(Screen::World));
        world.init_resource::<NextState<Screen>>();
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let actor = world.spawn((Vitals::new(&profile), profile, Carried::default())).id();
        world.resource_mut::<PartyRoster>().0.insert(actor);
        let root = world.spawn_empty().id();
        let button = world
            .spawn((
                SlotButton { member: actor, review_armed: false },
                PartyButton(0),
                Node::default(),
                Visibility::Inherited,
                ChildOf(root),
            ))
            .observe(press)
            .id();
        let click = |world: &mut World, count| {
            world.trigger(PointerClick {
                entity: button,
                pointer: crate::presentation::test_input::mouse(),
                button: PointerButton::Primary,
                hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
                duration: std::time::Duration::ZERO,
                count,
            })
        };
        click(&mut world, 1);
        assert_eq!(world.resource::<SelectedMember>().0, Some(actor));
        world.get_mut::<Vitals>(actor).unwrap().hp -= 1;
        use bevy::ecs::system::RunSystemOnce;
        world.run_system_once(bind_party_buttons).unwrap();
        assert!(world.get_entity(button).is_ok(), "health changes retain the click target");
        click(&mut world, 2);
        assert!(matches!(
            world.resource::<NextState<Screen>>(),
            NextState::Pending(Screen::Inventory)
        ));
        world.insert_resource(NextState::<Screen>::Unchanged);
        // A new binding/gesture must select before review even if Bevy's
        // pointer sequence still contains clicks on the previous binding.
        world.entity_mut(button).insert(SlotButton { member: actor, review_armed: false });
        click(&mut world, 3);
        assert!(matches!(world.resource::<NextState<Screen>>(), NextState::Unchanged));
        let replacement = Character::new("B", Race::Human, Gender::Male, Profession::Fighter);
        let other = world.spawn((Vitals::new(&replacement), replacement, Carried::default())).id();
        world.resource_mut::<PartyRoster>().0 = [other].into_iter().collect();
        world.run_system_once(bind_party_buttons).unwrap();
        assert!(
            world.get_entity(button).is_err(),
            "a rebind retires the pressed target instead of redirecting release"
        );
        assert_eq!(world.resource::<SelectedMember>().0, Some(actor));
    }
}
