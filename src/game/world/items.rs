//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use crate::game::party::roster::the_party;
use crate::game::world::level::LevelOwned;
use bevy::prelude::*;

use crate::game::data::Catalogs;

use crate::content::items::{ItemCatalog, ItemDefinition, ItemId, QuantityKind};
use crate::content::level::ItemPlacement;
use crate::game::inventory::{ItemIds, ItemNews, ItemStack, PartyInventory, Refusal};

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

/// Take a resolved world item. Validation and the owned-value move complete
/// synchronously, without a pointer, camera, rendering or a battle session.
pub fn take_item(world: &mut World, target: Entity) -> Result<(), Refusal> {
    if !crate::game::freedom::free_in(world) {
        return Err(Refusal::DoesNotFit);
    }
    // A second take of an item already taken finds nothing.
    if world.get::<WorldItem>(target).is_none() {
        return Err(Refusal::Nothing);
    }
    let eye = crate::game::party::body::PartyBody::eye_in(world);
    let at = world.query::<super::ActorPose>().get(world, target).unwrap().translation();
    // Retail measures to the model bounds' centre (ItemDistanceToCamera).
    // Deliberately use the gameplay placement so pickup never requires artwork.
    let distance = eye.distance(at);
    if !distance.is_finite() || !(0.0..=REACH).contains(&distance) {
        return Err(Refusal::DoesNotFit);
    }
    if world.get::<WorldItem>(target).unwrap().hidden {
        return Err(Refusal::DoesNotFit);
    }
    let party = the_party(world);
    if world.get::<PartyInventory>(party).unwrap().hand.held.is_some() {
        return Err(Refusal::HandFull);
    }
    let mut item = world.entity_mut(target).take::<WorldItem>().unwrap();
    if let GroundLoot::Pile { contents, .. } = &mut item.loot {
        let taken = Catalogs::scope(world, |world, catalogs| {
            world.get_mut::<PartyInventory>(party).unwrap().pool.collect(contents, &catalogs.items)
        });
        let moved = !taken.is_empty();
        for (name, _) in taken {
            world.write_message(ItemNews::Takes { member: None, item: name });
        }
        if !contents.is_empty() {
            world.entity_mut(target).insert(item);
            world.write_message(ItemNews::NoRoom);
            return if moved { Ok(()) } else { Err(Refusal::NoRoom) };
        }
    } else if let GroundLoot::Loose(stack) = item.loot {
        world.get_mut::<PartyInventory>(party).unwrap().hand.held = Some(stack);
    }
    world.despawn(target);
    Ok(())
}

/// Throw the held item from the party's eye along `aim`, validating the
/// current level and party activity: it stops short of what it hits and
/// rests above the ground below, or stays in hand if there is no ground.
pub fn drop_held(world: &mut World, level: Entity, aim: Dir3) -> Result<Entity, Refusal> {
    if !crate::game::freedom::free_in(world) {
        return Err(Refusal::DoesNotFit);
    }
    let eye = crate::game::party::body::PartyBody::eye_in(world);
    let ground = world
        .run_system_cached_with(landing, (eye, aim))
        .ok()
        .flatten()
        .ok_or(Refusal::DoesNotFit)?;
    let party = the_party(world);
    let held =
        world.get::<PartyInventory>(party).unwrap().hand.held.as_ref().ok_or(Refusal::Nothing)?;
    let definition = &world.resource::<crate::game::data::Catalogs>().items[held.item];
    if definition.crucial {
        return Err(Refusal::Crucial);
    }
    let held = world.get_mut::<PartyInventory>(party).unwrap().hand.held.take().unwrap();
    let item = lay(&mut world.commands(), level, ground, WorldItem::loose(held, false));
    world.flush();
    Ok(item)
}

/// The ground a throw from `eye` along `aim` comes down on, if any.
fn landing(
    In((eye, aim)): In<(Vec3, Dir3)>,
    obstacles: super::obstacles::WorldObstacles,
) -> Option<Vec3> {
    let reach = obstacles.first_hit(eye, aim, THROW).map_or(THROW, |hit| hit.distance);
    let above = eye + aim * (reach - CLEARANCE).max(0.0);
    let ground = obstacles.first_hit(above, Dir3::NEG_Y, 100.0)?;
    Some(above - Vec3::Y * ground.distance)
}

/// Retail throws a dropped item at most 2500 units, stops it 125 short of
/// what it hits, and rests it 125 above the ground below (`DropHeldItem`).
const THROW: f32 = 5.0;
const CLEARANCE: f32 = 0.25;
