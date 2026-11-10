//! Items lying in the world. Each is an item entity with a position under its
//! level, until the party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use bevy::prelude::*;

use crate::content::items::{ItemCatalog, ItemDefinition, ItemId, QuantityKind};
use crate::content::level::ItemPlacement;
use crate::game::items::{
    Bag, Give, Hidden, ItemStack, Items, Lying, Place, Put, Throw, make, roll_amount, unpack,
};
use crate::game::rng::GameRng;

/// An item lying in a level as a record keeps it, with what is in it if it is
/// a bag. A level that was left keeps what lay in it as these values.
#[derive(Clone, Debug)]
pub struct KeptItem {
    pub item: ItemStack,
    pub at: Vec3,
    pub hidden: bool,
    pub holds: Vec<ItemStack>,
}

impl KeptItem {
    /// An item lying as it is now, with what a bag holds.
    pub fn keep(items: &Items, catalog: &ItemCatalog, lying: Entity, at: Vec3) -> Option<Self> {
        Some(Self {
            item: items.stack(lying, catalog)?,
            at,
            hidden: items.hidden(lying),
            holds: items
                .stowed(lying)
                .iter()
                .filter_map(|&held| items.stack(held, catalog))
                .collect(),
        })
    }

    /// The item and what a bag holds, lying again where they were.
    pub fn restore(&self, commands: &mut Commands, level: Entity, catalog: &ItemCatalog) {
        let item = unpack(commands, &catalog[self.item.item], &self.item);
        lay(commands, level, (self.at, self.hidden), item);
        for held in &self.holds {
            let held = unpack(commands, &catalog[held.item], held);
            commands.trigger(Put { item: held, at: Place::Container(item) });
        }
    }
}

/// Lay an item at a point of a level, unseen if it is hidden.
pub fn lay(commands: &mut Commands, level: Entity, (at, hidden): (Vec3, bool), item: Entity) {
    commands.trigger(Put { item, at: Place::Ground { level, at } });
    if hidden {
        commands.entity(item).insert(Hidden);
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
    found: &[ItemId],
    (catalog, rng): (&ItemCatalog, &mut GameRng),
) {
    if !found.is_empty() {
        info!("loot: {} item(s) at {feet:.1}", found.len());
    }
    let at = (feet + Vec3::Y * CLEARANCE, false);
    let mut rolled = |commands: &mut Commands, id: ItemId| {
        let definition = &catalog[id];
        make(commands, definition, roll_amount(definition, rng, false), false)
    };
    match found {
        [] => {}
        &[id] => {
            let item = rolled(commands, id);
            lay(commands, level, at, item);
        }
        _ => {
            let bag = make(commands, &catalog[BAG_OF_ITEMS], 1, false);
            lay(commands, level, at, bag);
            for &id in found {
                let item = rolled(commands, id);
                commands.trigger(Put { item, at: Place::Container(bag) });
            }
        }
    }
}

/// Retail takes an item from at most this far from the eye
/// (`PickNearestItemUnderCursor(..., 5000.0)`), in metres.
pub const REACH: f32 = 10.0;

/// The item an authored placement describes, lying where it was placed: a
/// bag of the kind that holds things is placed empty.
pub fn place_item(
    commands: &mut Commands,
    level: Entity,
    placement: &ItemPlacement,
    definition: &ItemDefinition,
) {
    let amount = match definition.quantity {
        QuantityKind::None => 1,
        QuantityKind::Stack => placement.stack,
        _ => placement.uses,
    };
    let item = make(commands, definition, u16::from(amount), placement.identified);
    lay(commands, level, (placement.position, placement.hidden), item);
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
/// stands in, so pickup never needs artwork): a lone item into the hand,
/// a bag's contents into the pool as far as there is room.
pub fn take_items(
    mut requests: MessageReader<TakeItem>,
    freedom: crate::game::freedom::Freedom,
    party: Single<(Entity, &avian3d::prelude::Position)>,
    held: Items,
    lying: Query<(super::ActorPose, Has<Bag>), (With<Lying>, Without<Hidden>)>,
    mut commands: Commands,
) {
    let (party, position) = *party;
    let eye = crate::game::party::body::PartyBody::eye(position);
    let mut taken = Vec::new();
    // The hand fills as a lone item is taken, though it shows only once
    // the takings are carried out.
    let mut handful = held.held(party).is_some();
    for &TakeItem { item } in requests.read() {
        // A second take of an item already taken finds nothing.
        let Ok((at, bag)) = lying.get(item) else { continue };
        if !freedom.free()
            || taken.contains(&item)
            || eye.distance(at.translation()) > REACH
            || handful
        {
            continue;
        }
        taken.push(item);
        let inside = held.stowed(item);
        if !bag {
            commands.trigger(Put { item, at: Place::Hand { party } });
            handful = true;
        } else if inside.is_empty() {
            commands.entity(item).despawn();
        }
        // What the pool has no room for stays in the bag, which is gone once
        // it is empty.
        for &inside in inside {
            commands.trigger(Give { item: inside, to: None, party, pool_first: true, told: true });
        }
    }
}

/// Throw the held item: it stops short of what it hits and rests above the
/// ground below, or stays in hand if there is no ground.
pub fn drop_held(
    mut requests: MessageReader<DropHeld>,
    freedom: crate::game::freedom::Freedom,
    party: Single<(Entity, &avian3d::prelude::Position)>,
    held: Items,
    obstacles: super::obstacles::WorldObstacles,
    mut commands: Commands,
) {
    let (party, position) = *party;
    let eye = crate::game::party::body::PartyBody::eye(position);
    for &DropHeld { level, aim } in requests.read() {
        if !freedom.free() || held.held(party).is_none() {
            continue;
        }
        let Some(ground) = landing(eye, aim, &obstacles) else { continue };
        commands.trigger(Throw { party, level, at: ground + Vec3::Y * CLEARANCE });
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
/// How far above the ground an item rests.
pub const CLEARANCE: f32 = 0.25;
