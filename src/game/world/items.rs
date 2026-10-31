//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use crate::game::world::level::LevelOwned;
use bevy::prelude::*;

use crate::game::data::Catalogs;

use crate::content::items::{ItemCatalog, ItemDefinition, ItemId, QuantityKind};
use crate::content::level::ItemPlacement;
use crate::game::inventory::{ItemIds, ItemNews, ItemStack, PartyInventory};

/// An item lying in the level.
#[derive(Component, Debug, Clone)]
pub struct WorldItem {
    pub loot: GroundLoot,
    pub hidden: bool,
}
#[derive(Debug, Clone)]
pub enum GroundLoot {
    Loose(ItemStack),
    Pile { appearance: ItemId, identified: bool, contents: Vec<ItemStack> },
}
impl WorldItem {
    pub fn loose(item: ItemStack, hidden: bool) -> Self {
        Self { loot: GroundLoot::Loose(item), hidden }
    }
    pub fn appearance(&self) -> ItemId {
        match &self.loot {
            GroundLoot::Loose(s) => s.item,
            GroundLoot::Pile { appearance, .. } => *appearance,
        }
    }
    pub fn display_name(&self, definition: &ItemDefinition, count: bool) -> String {
        match &self.loot {
            GroundLoot::Loose(s) => s.display_name(definition, count),
            GroundLoot::Pile { identified, .. } => definition.known_name(*identified).to_owned(),
        }
    }
    pub fn stacks(&self) -> &[ItemStack] {
        match &self.loot {
            GroundLoot::Loose(s) => std::slice::from_ref(s),
            GroundLoot::Pile { contents, .. } => contents,
        }
    }
}

/// The Bag of Items that holds what a monster leaves when it is more than
/// one item (retail `DropMonsterLoot`'s default container).
const BAG_OF_ITEMS: ItemId = ItemId(0x23c);

/// Lay what a monster left where it fell (retail `DropMonsterLoot`): one
/// item by itself, several in a Bag of Items. Retail settles them on the
/// ground; they rest a quarter metre above the monster's feet, as dropped
/// items do.
pub fn drop_loot(
    commands: &mut Commands,
    level: Entity,
    feet: Vec3,
    mut items: Vec<ItemStack>,
    catalog: &ItemCatalog,
) {
    if !items.is_empty() {
        info!("loot: {} item(s) at {feet:.1}", items.len());
    }
    let mut lay = |lying| {
        lay(commands, level, feet, lying);
    };
    match items.len() {
        0 => {}
        1 => lay(WorldItem::loose(items.remove(0), false)),
        _ => lay(WorldItem {
            loot: GroundLoot::Pile {
                appearance: BAG_OF_ITEMS,
                identified: catalog[BAG_OF_ITEMS].auto_identify,
                contents: items,
            },
            hidden: false,
        }),
    }
}

/// Lay an item on the ground at `feet`, resting as dropped items do.
pub fn lay(commands: &mut Commands, level: Entity, feet: Vec3, item: WorldItem) -> Entity {
    commands
        .spawn((
            Name::new("dropped item"),
            item,
            Transform::from_translation(feet + Vec3::Y * CLEARANCE),
            LevelOwned(level),
        ))
        .id()
}

/// Retail takes an item from at most this far from the eye
/// (`PickNearestItemUnderCursor(..., 5000.0)`), in metres.
pub const REACH: f32 = 10.0;

/// The item an authored placement describes.
pub fn placed_item(
    placement: &ItemPlacement,
    definition: &ItemDefinition,
    ids: &mut ItemIds,
) -> ItemStack {
    let amount = match definition.quantity {
        QuantityKind::None => 1,
        QuantityKind::Stack => placement.stack,
        _ => placement.uses,
    };
    ItemStack::new(definition, ids, u16::from(amount), placement.identified)
}

/// The player takes an item lying in the world.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct TakeItem {
    pub item: Entity,
}

/// The player throws the held item from the party's eye along `aim`.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct DropHeld {
    pub level: Entity,
    pub aim: Dir3,
}

/// Take items the party can reach from its eye (retail measures to the
/// model bounds' centre, `ItemDistanceToCamera`; the gameplay placement
/// stands in, so pickup never needs artwork): a loose item into the hand,
/// a pile's contents into the pool as far as there is room.
pub fn take_items(
    mut requests: MessageReader<TakeItem>,
    freedom: crate::game::freedom::Freedom,
    party: Single<(&avian3d::prelude::Position, &mut PartyInventory)>,
    mut items: Query<(&mut WorldItem, super::ActorPose)>,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<ItemNews>,
    mut commands: Commands,
) {
    let (position, mut inventory) = party.into_inner();
    let eye = crate::game::party::body::PartyBody::eye(position);
    let mut taken = Vec::new();
    for &TakeItem { item: target } in requests.read() {
        // A second take of an item already taken finds nothing.
        let Ok((mut item, at)) = items.get_mut(target) else { continue };
        if !freedom.free()
            || taken.contains(&target)
            || item.hidden
            || eye.distance(at.translation()) > REACH
            || inventory.hand.held.is_some()
        {
            continue;
        }
        match &mut item.loot {
            GroundLoot::Pile { contents, .. } => {
                for (name, _) in inventory.pool.collect(contents, &content.items) {
                    news.write(ItemNews::Takes { member: None, item: name });
                }
                if !contents.is_empty() {
                    news.write(ItemNews::NoRoom);
                    continue;
                }
            }
            GroundLoot::Loose(stack) => inventory.hand.held = Some(stack.clone()),
        }
        taken.push(target);
        commands.entity(target).despawn();
    }
}

/// Throw the held item: it stops short of what it hits and rests above the
/// ground below, or stays in hand if there is no ground. A crucial item is
/// never thrown away.
pub fn drop_held(
    mut requests: MessageReader<DropHeld>,
    freedom: crate::game::freedom::Freedom,
    party: Single<(&avian3d::prelude::Position, &mut PartyInventory)>,
    obstacles: super::obstacles::WorldObstacles,
    content: If<Res<Catalogs>>,
    mut commands: Commands,
) {
    let (position, mut inventory) = party.into_inner();
    let eye = crate::game::party::body::PartyBody::eye(position);
    for &DropHeld { level, aim } in requests.read() {
        let Some(held) = &inventory.hand.held else { continue };
        if !freedom.free() || content.items[held.item].crucial {
            continue;
        }
        let Some(ground) = landing(eye, aim, &obstacles) else { continue };
        let held = inventory.hand.held.take().unwrap();
        lay(&mut commands, level, ground, WorldItem::loose(held, false));
    }
}

/// The ground a throw from `eye` along `aim` comes down on, if any.
fn landing(eye: Vec3, aim: Dir3, obstacles: &super::obstacles::WorldObstacles) -> Option<Vec3> {
    let reach = obstacles.first_hit(eye, aim, THROW).map_or(THROW, |hit| hit.distance);
    let above = eye + aim * (reach - CLEARANCE).max(0.0);
    let ground = obstacles.first_hit(above, Dir3::NEG_Y, 100.0)?;
    Some(above - Vec3::Y * ground.distance)
}

/// Retail throws a dropped item at most 2500 units, stops it 125 short of
/// what it hits, and rests it 125 above the ground below (`DropHeldItem`).
const THROW: f32 = 5.0;
const CLEARANCE: f32 = 0.25;
