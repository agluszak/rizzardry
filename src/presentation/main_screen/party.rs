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
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Vitals};
use crate::game::inventory::EquipSlot;
use crate::game::npcs::conversation::Conversation;
use crate::game::party::DamageApplied;
use crate::game::party::roster::PartyRoster;
use crate::presentation::PresentationRng;
use crate::presentation::screen_art::visible_if;
use crate::presentation::screen_art::{
    BitmapText, Picture, absolute, condition_icon, label, picture, rect, skull,
};

/// Display slots are not roster order: RPCs occupy the top pair and the
/// six player characters occupy the remaining rows. Derive this projection
/// from membership and NPC identity without reordering gameplay/RNG iteration.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PortraitSlots<'w, 's> {
    roster: Single<'w, 's, &'static PartyRoster>,
    npcs: Query<'w, 's, (), With<crate::game::npcs::Npc>>,
}
impl PortraitSlots<'_, '_> {
    fn member(&self, slot: usize) -> Option<Entity> {
        let rpc = slot < 2;
        self.roster.iter().filter(|e| self.npcs.contains(*e) == rpc).nth(if rpc {
            slot
        } else {
            slot - 2
        })
    }
    fn slot(&self, member: Entity) -> Option<usize> {
        let rpc = self.npcs.contains(member);
        let index = self
            .roster
            .iter()
            .filter(|e| self.npcs.contains(*e) == rpc)
            .position(|e| e == member)?;
        (index < if rpc { 2 } else { 6 }).then_some(index + if rpc { 0 } else { 2 })
    }
}

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

#[derive(Component)]
pub struct PartyMemberView {
    member: Entity,
    portrait: bool,
}

#[derive(Component)]
pub struct EmptySlot(usize);

#[derive(Clone, Copy, Default)]
enum Part {
    #[default]
    Portrait,
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
    ConditionRim(bool),
    Selection,
}

#[derive(Component, Clone)]
pub struct PartyPart {
    member: Entity,
    kind: Part,
}

fn part_picture(member: Entity, kind: Part, at: Vec2) -> impl Scene {
    bsn! { @picture(default(), 0, at) template(move |_| Ok(PartyPart { member, kind })) }
}

fn part_label(
    member: Entity,
    kind: Part,
    font: Handle<SpriteSheet>,
    at: Vec2,
    width: f32,
) -> impl Scene {
    bsn! { @label(font, at, width) template(move |_| Ok(PartyPart { member, kind })) }
}

/// Holds the portraits' damage splats.
#[derive(Component, Clone, Default)]
pub struct Splats;

pub fn portraits() -> impl SceneList {
    (0..8_usize).map(|slot| bsn! { @layer() SlotPortrait({slot}) }).collect::<Vec<_>>()
}

pub fn slots() -> impl SceneList {
    let slots: Vec<_> =
        (0..8_usize).map(|slot| bsn! { @layer() PartySlot { slot: {slot} } }).collect();
    slots
}

/// A bar's fill, 0 to 45 pixels, at least 1 while any is left (retail
/// `SyncPartyPortraitVitalsBars`).
/// Create controls once for this layout. Gameplay updates their properties.
pub fn build_party_parts(
    mut commands: Commands,
    layout: Res<ScreenLayout>,
    art: Res<MainScreenArt>,
    assets: Res<AssetServer>,
    slots: PortraitSlots,
    members: Query<(), (With<Character>, With<Vitals>)>,
    views: Query<(Entity, &PartyMemberView, &ChildOf)>,
    roots: Query<
        (Entity, Option<&PartySlot>, Option<&SlotPortrait>, Has<PartyWidgetsBuilt>),
        Or<(With<PartySlot>, With<SlotPortrait>)>,
    >,
) {
    let closed = layout.mode == LayoutMode::Closed;
    for (entity, view, _) in &views {
        if !slots.roster.contains(&view.member) || !members.contains(view.member) {
            commands.entity(entity).despawn();
        }
    }
    for (root, overlay, portrait, built) in &roots {
        let slot = overlay.map_or_else(|| portrait.unwrap().0, |slot| slot.slot);
        if portrait.is_some() && !built {
            commands.entity(root).insert(PartyWidgetsBuilt);
            let back = assets.load(crate::content::paths::sheet("main interface/pback"));
            let dragon = assets.load(crate::content::paths::sheet("main interface/dragons"));
            commands
                .spawn_scene(bsn! { @layer() template(move |_| Ok(EmptySlot(slot))) Children [
                    @picture(back,0,Vec2::new(menu_x(slot)+20.,row_y(slot)))
                    -- @picture(dragon,0,Vec2::new(menu_x(slot)+23.,row_y(slot)))
                ] })
                .insert(ChildOf(root));
        }
        let Some(member) = slots.member(slot).filter(|e| members.contains(*e)) else {
            continue;
        };
        if let Some((entity, _, parent)) = views
            .iter()
            .find(|(_, view, _)| view.member == member && view.portrait == portrait.is_some())
        {
            if parent.parent() != root {
                commands.entity(entity).insert(ChildOf(root));
            }
            continue;
        }
        let view = commands
            .spawn((
                absolute(Vec2::ZERO, Vec2::ZERO),
                Pickable::IGNORE,
                PartyMemberView { member, portrait: portrait.is_some() },
                ChildOf(root),
            ))
            .id();
        let mut parts: Vec<Box<dyn Scene>> = vec![];
        let pictures = if portrait.is_some() {
            vec![Part::Portrait]
        } else {
            vec![
                Part::Bars,
                Part::Health,
                Part::Stamina,
                Part::Hands,
                Part::MainHand,
                Part::OffHand,
                Part::Condition,
                Part::Enchantment,
                Part::ConditionRim(false),
                Part::ConditionRim(true),
            ]
        };
        for kind in pictures {
            parts.push(Box::new(part_picture(member, kind, kind.at(slot, closed))));
        }
        if portrait.is_none() {
            for (kind, size, colour) in [
                (Part::HealthEmpty, Vec2::new(5., 45.), Color::srgb_u8(1, 1, 1)),
                (Part::StaminaEmpty, Vec2::new(5., 45.), Color::srgb_u8(1, 1, 1)),
                (Part::Alive, Vec2::new(19., 9.), Color::BLACK),
            ] {
                let at = kind.at(slot, closed);
                parts.push(Box::new(bsn! { @rect(at,size,colour) template(move |_| Ok(PartyPart { member, kind })) }));
            }
            for (kind, font, width) in [
                (Part::HitPoints, art.small_font.clone(), 19.),
                (Part::Armour, art.small_font.clone(), 13.),
                (Part::Name, art.font.clone(), 84.),
            ] {
                parts.push(Box::new(part_label(member, kind, font, kind.at(slot, closed), width)));
            }
            parts.push(Box::new(part_picture(
                member,
                Part::Selection,
                Part::Selection.at(slot, closed),
            )));
            let at = Vec2::new(
                if closed {
                    menu_x(slot) + 23.
                } else if slot.is_multiple_of(2) {
                    0.
                } else {
                    617.
                },
                row_y(slot),
            );
            let node = absolute(at, if closed { Vec2::new(82., 72.) } else { Vec2::new(22., 85.) });
            parts.push(Box::new(
                bsn! { node SlotButton { member: {member}, review_armed: false } on(press) on(press_right) on(release_right) on(leave) },
            ));
        }
        for part in parts {
            commands.spawn_scene(part).insert(ChildOf(view));
        }
    }
}

impl Part {
    fn at(self, slot: usize, closed: bool) -> Vec2 {
        let y = row_y(slot);
        let left = slot.is_multiple_of(2);
        let bars = match (closed, left) {
            (true, true) => 105.,
            (true, false) => 512.,
            (false, true) => 0.,
            (false, false) => 617.,
        };
        let hands = if left { 2. } else { 619. };
        let condition = menu_x(slot) + if left { 25. } else { 88. };
        let enchantment = menu_x(slot) + if left { 88. } else { 25. };
        match self {
            Self::Portrait => Vec2::new(menu_x(slot) + 20., y),
            Self::Bars => Vec2::new(bars, y),
            Self::Health | Self::HealthEmpty => Vec2::new(bars + 3., y + 17.),
            Self::Stamina | Self::StaminaEmpty => Vec2::new(bars + 9., y + 17.),
            Self::Alive | Self::HitPoints => Vec2::new(bars + 2., y + 62.),
            Self::Hands => Vec2::new(hands, y),
            Self::MainHand => Vec2::new(hands + 3., y + 23.),
            Self::OffHand => Vec2::new(hands + 3., y + 47.),
            Self::Armour => Vec2::new(hands + 3., y + 3.),
            Self::Name => Vec2::new(menu_x(slot) + 21., y + 73.),
            Self::Condition => Vec2::new(condition, y + 3.),
            Self::Enchantment => Vec2::new(enchantment, y + 3.),
            Self::ConditionRim(other) => {
                Vec2::new(if other { enchantment - 1. } else { condition - 1. }, y + 2.)
            }
            Self::Selection if closed => Vec2::new(menu_x(slot) + 23., y),
            Self::Selection => Vec2::new(bars + 1., y + 2.),
        }
    }
}

pub fn draw_party(
    layout: Res<ScreenLayout>,
    selected: Res<SelectedMember>,
    assets: Res<AssetServer>,
    slots: PortraitSlots,
    members: Query<CharacterView>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    party_effects: Option<Single<&crate::game::magic::enchantments::PartyEffects>>,
    content: If<Res<crate::game::data::Catalogs>>,
    mut parts: Query<(
        &PartyPart,
        &mut Node,
        &mut Visibility,
        Option<&mut Picture>,
        Option<&mut BitmapText>,
        Option<&mut ImageNode>,
    )>,
) {
    let catalog = &content.items;
    let closed = layout.mode == LayoutMode::Closed;
    for (part, mut node, mut visible, picture, label, image_node) in &mut parts {
        let Some(slot) = slots.slot(part.member) else { continue };
        let member = Some(part.member);
        let actor = members.get(part.member).ok();
        let at = part.kind.at(slot, closed);
        let mut shown = actor.is_some();
        let mut source = None;
        let mut value = None;
        let mut hand_x = None;
        let image = |path: &str, frame| (crate::content::paths::retail(path), frame);
        if let Some(view) = actor {
            let armour = view.armour(
                part.member,
                catalog,
                battle.as_deref().copied(),
                party_effects.as_deref().copied(),
            );
            let CharacterViewItem { profile, vitals, carried, .. } = view;
            let dead = vitals.is_dead();
            let worn = |slot| carried.worn(slot).map(|i| &catalog[i.item]);
            let two_handed = worn(EquipSlot::Primary).is_some_and(|d| d.two_handed);
            let condition = condition_icon(vitals);
            match part.kind {
                Part::Portrait => {
                    let name = if dead { &skull(profile.race)[1..] } else { &profile.portrait };
                    source = Some((
                        crate::content::paths::portrait(
                            crate::content::paths::PortraitSize::Medium,
                            name,
                        ),
                        0,
                    ));
                }

                Part::Bars => {
                    source = Some(image("data/main interface/main_column_info_normal.sti", 3))
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
                        crate::presentation::screen_art::vital_fill(
                            vitals.hp,
                            profile.stats(&vitals.enchantments).max_hp(),
                        )
                    } else {
                        crate::presentation::screen_art::vital_fill(
                            vitals.stamina,
                            profile.stats(&vitals.enchantments).max_stamina(),
                        )
                    };
                    let height = px(45 - amount);
                    if node.height != height {
                        node.height = height;
                    }
                }
                Part::Alive => shown = !dead,
                Part::Hands => {
                    source = Some(image(
                        "data/main interface/main_column_info_normal.sti",
                        if two_handed { 2 } else { 0 },
                    ))
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
                        (if slot.is_multiple_of(2) { 2.0 } else { 619.0 })
                            + if item.is_some() { 3.0 } else { 4.0 },
                    );
                }
                Part::Armour => {
                    shown = !dead;
                    value = Some(armour.average().to_string());
                }
                Part::Name => value = Some(profile.name.clone()),
                Part::HitPoints => {
                    shown = !dead;
                    value = Some(vitals.hp.to_string());
                }
                Part::Condition | Part::Enchantment => {
                    let plug = if slot.is_multiple_of(2) { "leftplug" } else { "rightplug" };
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
                Part::ConditionRim(_) => {
                    shown = condition.is_some();
                    source = Some(image("data/main interface/condition_highlight.sti", 0));
                }
                Part::Selection => {
                    shown = member == selected.member(&slots.roster);
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
        }
        shown &= closed
            || matches!(
                part.kind,
                Part::Bars
                    | Part::Health
                    | Part::Stamina
                    | Part::HealthEmpty
                    | Part::StaminaEmpty
                    | Part::Alive
                    | Part::HitPoints
                    | Part::Selection
            );
        visible.set_if_neq(visible_if(shown));
        if let Some(mut picture) = picture
            && let Some((path, frame)) = source
        {
            crate::presentation::screen_art::show_sheet(&mut picture, &assets, path);
            let at = Vec2::new(hand_x.unwrap_or(at.x), at.y);
            if picture.at != at {
                picture.at = at;
            }
            if picture.frame != frame {
                picture.frame = frame;
            }
        }
        if let Some(mut label) = label {
            if label.at != at {
                label.at = at;
            }
            if let Some(value) = value
                && label.value != value
            {
                label.value = value;
            }
        }
        if matches!(part.kind, Part::HealthEmpty | Part::StaminaEmpty | Part::Alive) {
            if node.left != px(at.x) {
                node.left = px(at.x);
            }
            if node.top != px(at.y) {
                node.top = px(at.y);
            }
        }
        if let Some(mut image) = image_node {
            let flip = matches!(part.kind, Part::Portrait)
                && slot % 2 == 1
                && member
                    .and_then(|e| members.get(e).ok())
                    .is_some_and(|view| view.vitals.is_dead());
            if image.flip_x != flip {
                image.flip_x = flip;
            }
        }
    }
}

/// Move retained controls; never change which actor a press means.
pub fn place_party_buttons(
    slots: PortraitSlots,
    layout: Res<ScreenLayout>,
    members: Query<(), (With<Character>, With<Vitals>)>,
    mut buttons: Query<(&SlotButton, &mut Node)>,
    mut empty: Query<(&EmptySlot, &mut Visibility)>,
) {
    for (slot, mut visible) in &mut empty {
        visible.set_if_neq(if slots.member(slot.0).is_none_or(|e| !members.contains(e)) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for (button, mut node) in &mut buttons {
        let Some(slot) = slots.slot(button.member) else { continue };
        let x = if layout.mode == LayoutMode::Closed {
            menu_x(slot) + 23.
        } else if slot.is_multiple_of(2) {
            0.
        } else {
            617.
        };
        let size = if layout.mode == LayoutMode::Closed {
            Vec2::new(82., 72.)
        } else {
            Vec2::new(22., 85.)
        };
        if node.width != px(size.x) {
            node.width = px(size.x);
        }
        if node.height != px(size.y) {
            node.height = px(size.y);
        }
        if node.left != px(x) {
            node.left = px(x);
        }
        if node.top != px(row_y(slot)) {
            node.top = px(row_y(slot));
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
    roster: Single<Ref<PartyRoster>>,
    mut operations: ParamSet<(
        crate::game::inventory::operations::InventoryOperations,
        super::magic_view::SourceTargeting,
    )>,
    mut review: crate::presentation::inventory::OpenReview,
    mut views: Query<(&mut super::magic_view::MagicView, Has<super::magic_view::CastsAt>)>,
    conversations: Query<
        (),
        (With<Conversation>, Without<crate::game::npcs::conversation::Closing>),
    >,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok(mut button) = buttons.get_mut(click.entity) else { return };
    let member = button.member;
    if !roster.contains(&member) {
        return;
    }
    if !conversations.is_empty() {
        review.selected.set_if_neq(SelectedMember(Some(member)));
        button.review_armed = false;
        return;
    }
    // A spell for one party member is aimed by clicking their portrait
    // (retail `AimAtCharacter`).
    if let Ok((mut view, false)) = views.single_mut()
        && view.wants_member(&operations.p1())
    {
        view.aim_at(member);
        return;
    }
    if operations.p0().give_to(member) {
        button.review_armed = false;
        return;
    }
    let already_selected = review.selected.0 == Some(member);
    review.selected.set_if_neq(SelectedMember(Some(member)));
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
    mut review: crate::presentation::inventory::OpenReview,
    mut commands: Commands,
    conversations: Query<
        (),
        (With<Conversation>, Without<crate::game::npcs::conversation::Closing>),
    >,
) {
    if release.button != PointerButton::Secondary || !conversations.is_empty() {
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
        if inventory.give_to(member) {
            return;
        }
    }
    review.open(member);
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
    mut commands: Commands,
) {
    for (entity, button, hold) in &holds {
        if !mouse.as_ref().is_some_and(|mouse| mouse.pressed(MouseButton::Right)) {
            commands.entity(entity).remove::<RightHold>();
        } else if time.elapsed() - hold.since >= REVIEW_HOLD {
            commands.entity(entity).remove::<RightHold>();
            review.open(button.member);
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
    slots: PortraitSlots,
    mut splats: Query<(Entity, &mut Splat, &mut Node, &Children)>,
    mut pictures: Query<&mut Picture>,
    mut numbers: Query<(&mut Visibility, &mut BitmapText), With<SplatNumber>>,
) {
    for (entity, mut splat, mut node, parts) in &mut splats {
        splat.timer.tick(time.delta());
        let Some(slot) = slots.slot(splat.actor).filter(|_| !splat.timer.is_finished()) else {
            commands.entity(entity).despawn();
            continue;
        };
        let at = Vec2::new(if slot.is_multiple_of(2) { 22.0 } else { 533.0 }, row_y(slot));
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
                visible.set_if_neq(visible_if(frame < 6));
                let value = splat.damage.to_string();
                if label.value != value {
                    label.value = value;
                }
            }
        }
    }
}
