//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use bevy::prelude::*;

use crate::content::items::{ItemCatalog, ItemDefinition, ItemId, QuantityKind};
use crate::content::level::ItemPlacement;
use crate::game::inventory::operations::transfer_contents;
use crate::game::inventory::{
    Held, ItemIds, ItemInstance, ItemNews, PartyInventory, Quantity, Refusal, make_item,
};
use crate::game::rng::GameRng;

/// An item lying in the level.
#[derive(Component, Debug, Clone)]
pub struct WorldItem {
    pub item: ItemInstance,
    /// Unseen, and out of reach, until found by searching.
    pub hidden: bool,
    /// A container's items (retail chains them after it), such as the Bag
    /// of Items a monster's several drops lie in.
    pub contents: Vec<ItemInstance>,
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
    mut items: Vec<ItemInstance>,
    catalog: &ItemCatalog,
    ids: &mut ItemIds,
    rng: &mut GameRng,
) {
    if !items.is_empty() {
        info!("loot: {} item(s) at {feet:.1}", items.len());
    }
    let lying = match items.len() {
        0 => return,
        1 => WorldItem { item: items.remove(0), hidden: false, contents: vec![] },
        _ => {
            let Some(bag) = catalog.get(BAG_OF_ITEMS) else {
                // Invalid/incomplete catalogs still preserve every owned drop.
                for item in items {
                    commands.spawn((
                        Name::new("loot"),
                        WorldItem { item, hidden: false, contents: vec![] },
                        Transform::from_translation(feet + Vec3::Y * CLEARANCE),
                        ChildOf(level),
                    ));
                }
                return;
            };
            let item = make_item(bag, ids, rng, false, false);
            WorldItem { item, hidden: false, contents: items }
        }
    };
    commands.spawn((
        Name::new("loot"),
        lying,
        Transform::from_translation(feet + Vec3::Y * CLEARANCE),
        ChildOf(level),
    ));
}

/// Retail takes an item from at most this far from the eye
/// (`PickNearestItemUnderCursor(..., 5000.0)`), in metres.
pub const REACH: f32 = 10.0;

/// The item an authored placement describes.
pub fn placed_item(
    placement: &ItemPlacement,
    definition: &ItemDefinition,
    ids: &mut ItemIds,
) -> ItemInstance {
    let quantity = match definition.quantity {
        QuantityKind::None => Quantity::One,
        QuantityKind::Stack => Quantity::Stack(placement.stack),
        _ => Quantity::Uses(placement.uses),
    };
    ItemInstance {
        id: ids.allocate(),
        item: definition.id,
        quantity,
        identified: placement.identified || definition.auto_identify,
        spell_known: false,
        curse_known: false,
        remarked: false,
    }
}

/// Take a resolved world item. Validation and the owned-value move complete
/// synchronously, without a pointer, camera, rendering or a battle session.
pub fn take_item(world: &mut World, target: Entity, distance: f32) -> Result<(), Refusal> {
    if !world.run_system_cached(crate::game::screen::party_free).unwrap_or(false) {
        return Err(Refusal::DoesNotFit);
    }
    if !distance.is_finite() || !(0.0..=REACH).contains(&distance) {
        return Err(Refusal::DoesNotFit);
    }
    let handle = world
        .get_resource::<crate::game::data::GameData>()
        .ok_or(Refusal::MissingContent)?
        .items
        .clone();
    let catalog = world
        .get_resource::<Assets<ItemCatalog>>()
        .and_then(|c| c.get(&handle))
        .ok_or(Refusal::MissingContent)?;
    let item = world.get::<WorldItem>(target).ok_or(Refusal::Nothing)?;
    let owner = world.get::<ChildOf>(target).ok_or(Refusal::Nothing)?.parent();
    if world.get::<super::level::Level>(owner).is_none() || item.hidden {
        return Err(Refusal::DoesNotFit);
    }
    let is_container = catalog.get(item.item.item).ok_or(Refusal::MissingContent)?.container;
    if world.resource::<PartyInventory>().held.is_some() {
        return Err(Refusal::HandFull);
    }
    let mut item = world.entity_mut(target).take::<WorldItem>().unwrap();
    if is_container {
        let taken = world.resource_scope(|world, mut party: Mut<PartyInventory>| {
            let catalog = world.resource::<Assets<ItemCatalog>>().get(&handle).unwrap();
            transfer_contents(&mut item.contents, &mut party, catalog)
        });
        for (name, _) in taken {
            world.write_message(ItemNews::Takes { member: None, item: name });
        }
        if !item.contents.is_empty() {
            world.entity_mut(target).insert(item);
            world.write_message(ItemNews::NoRoom);
            return Ok(());
        }
    } else {
        world.resource_mut::<PartyInventory>().held = Some(Held { item: item.item, origin: None });
    }
    world.despawn(target);
    Ok(())
}

/// Drop the held owned value at a landing point resolved by an adapter. The
/// operation validates the current level, party activity and throw reach.
pub fn drop_held(world: &mut World, level: Entity, at: Vec3) -> Result<Entity, Refusal> {
    if !world.run_system_cached(crate::game::screen::party_free).unwrap_or(false)
        || world.get::<super::level::Level>(level).is_none()
        || !at.is_finite()
    {
        return Err(Refusal::DoesNotFit);
    }
    let eye = world
        .query_filtered::<&avian3d::prelude::Position, With<crate::game::party::body::PartyBody>>()
        .single(world)
        .map(crate::game::party::body::PartyBody::eye)
        .map_err(|_| Refusal::Nothing)?;
    if eye.xz().distance(at.xz()) > THROW + CLEARANCE {
        return Err(Refusal::DoesNotFit);
    }
    let held = world.resource_mut::<PartyInventory>().held.take().ok_or(Refusal::Nothing)?;
    Ok(world
        .spawn((
            Name::new("dropped item"),
            WorldItem { item: held.item, hidden: false, contents: vec![] },
            Transform::from_translation(at),
            ChildOf(level),
        ))
        .id())
}

/// Retail throws a dropped item at most 2500 units, stops it 125 short of
/// what it hits, and rests it 125 above the ground below (`DropHeldItem`).
pub(crate) const THROW: f32 = 5.0;
pub(crate) const CLEARANCE: f32 = 0.25;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::ItemDefinition;

    fn catalog() -> ItemCatalog {
        let mut items: Vec<ItemDefinition> = (0..=BAG_OF_ITEMS.0)
            .map(|id| ItemDefinition { id: ItemId(id), name: format!("item {id}"), ..default() })
            .collect();
        items[usize::from(BAG_OF_ITEMS.0)].container = true;
        ItemCatalog(items)
    }

    fn item(ids: &mut ItemIds, id: u16) -> ItemInstance {
        let definition = ItemDefinition { id: ItemId(id), ..default() };
        make_item(&definition, ids, &mut GameRng::seeded(1), false, true)
    }

    #[test]
    fn several_drops_lie_in_a_bag_one_lies_alone() {
        let (catalog, mut ids, mut rng) = (catalog(), ItemIds::default(), GameRng::seeded(2));
        let mut world = World::new();
        let level = world.spawn_empty().id();
        let (one, two) = (item(&mut ids, 1), vec![item(&mut ids, 2), item(&mut ids, 3)]);
        let mut commands = world.commands();
        drop_loot(&mut commands, level, Vec3::ZERO, vec![one], &catalog, &mut ids, &mut rng);
        drop_loot(&mut commands, level, Vec3::X, two, &catalog, &mut ids, &mut rng);
        drop_loot(&mut commands, level, Vec3::Y, vec![], &catalog, &mut ids, &mut rng);
        world.flush();
        let mut lying: Vec<(u16, usize)> = world
            .query::<&WorldItem>()
            .iter(&world)
            .map(|w| (w.item.item.0, w.contents.len()))
            .collect();
        lying.sort();
        assert_eq!(lying, [(1, 0), (BAG_OF_ITEMS.0, 2)]);
    }
}
