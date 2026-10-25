//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use avian3d::prelude::{RigidBody, SpatialQuery, SpatialQueryFilter};
use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::items::{ItemCatalog, ItemDefinition, ItemId, QuantityKind};
use crate::content::level::ItemPlacement;
use crate::game::data::GameData;
use crate::game::inventory::{
    Held, ItemIds, ItemInstance, ItemNews, PartyInventory, Quantity, make_item,
};
use crate::game::party::body::{Interact, PartyCamera};
use crate::game::rng::GameRng;
use crate::game::world::level::Level;
use crate::game::world::props::{Prop, drawn_under_ray, owner, ray_under_pointer};

/// An item lying in the level.
#[derive(Component, Debug, Clone)]
#[require(Visibility)]
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
            let Some(bag) = catalog.get(BAG_OF_ITEMS) else { return };
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
        curse_released: false,
        remarked: false,
    }
}

/// A click in the view, for items (retail `MainGameScreen` view click):
/// with an item in hand and no prop under the pointer, drop it there;
/// with an empty hand, take the item under the pointer.
#[allow(clippy::too_many_arguments)]
pub fn click_items_in_view(
    mut commands: Commands,
    interact: Single<&ActionEvents, With<Action<Interact>>>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    camera: Query<(&Camera, &GlobalTransform), With<PartyCamera>>,
    mut meshes: MeshRayCast,
    (sky, parents, props, levels, bodies): (
        Query<(), With<RenderLayers>>,
        Query<&ChildOf>,
        Query<(), With<Prop>>,
        Query<Entity, With<Level>>,
        Query<&RigidBody>,
    ),
    spatial: SpatialQuery,
    mut items: Query<&mut WorldItem>,
    (data, catalogs, mut inventory, mut news): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        ResMut<PartyInventory>,
        MessageWriter<ItemNews>,
    ),
) {
    if !interact.contains(ActionEvents::START) {
        return;
    }
    let Ok((camera, transform)) = camera.single() else { return };
    let Some(ray) = ray_under_pointer(&pointers, camera, transform) else { return };
    let hit = drawn_under_ray(ray, &mut meshes, &sky);
    if inventory.held.is_some() {
        if hit.is_some_and(|(hit, _)| owner(hit, |e| props.contains(e), &parents).is_some()) {
            return;
        }
        let Ok(level) = levels.single() else { return };
        let solid = |entity| bodies.get(entity).is_ok_and(RigidBody::is_static);
        let Some(at) = drop_point(&spatial, ray, &solid) else { return };
        let Some(held) = inventory.held.take() else { return };
        commands.spawn((
            Name::new("dropped item"),
            WorldItem { item: held.item, hidden: false, contents: vec![] },
            Transform::from_translation(at),
            ChildOf(level),
        ));
        return;
    }
    let Some((hit, distance)) = hit else { return };
    let Some(entity) = owner(hit, |e| items.contains(e), &parents) else { return };
    let Ok(mut item) = items.get_mut(entity) else { return };
    let Some(catalog) = data.and_then(|d| catalogs.get(&d.items)) else { return };
    let Some(definition) = catalog.get(item.item.item) else { return };
    if item.hidden {
        return;
    }
    if distance > REACH {
        debug!("{} is out of reach: {distance:.1} m", definition.name);
        return;
    }
    // Retail opens a container in its item picker, which is not ported:
    // here, as with chests, the party takes what is inside into its pool,
    // and what does not fit stays in the container.
    if definition.container {
        let emptied = empty_into_pool(&mut item, &mut inventory, catalog, &mut |n| {
            news.write(n);
        });
        if emptied {
            commands.entity(entity).despawn();
        }
        return;
    }
    info!("took {}", definition.name);
    inventory.held = Some(Held { item: item.item.clone(), origin: None });
    commands.entity(entity).despawn();
}

/// Move what a container holds into the party's pool, telling of each item
/// taken; what does not fit stays inside ("No room!"). Whether it is now
/// empty.
fn empty_into_pool(
    container: &mut WorldItem,
    inventory: &mut PartyInventory,
    catalog: &ItemCatalog,
    tell: &mut dyn FnMut(ItemNews),
) -> bool {
    let mut kept = Vec::new();
    for inside in container.contents.drain(..) {
        let name = catalog.get(inside.item).map(|d| inside.display_name(d, false));
        match inventory.place_in_pool(inside, catalog) {
            Ok(()) => tell(ItemNews::Takes { member: None, item: name.unwrap_or_default() }),
            Err(back) => kept.push(back),
        }
    }
    if !kept.is_empty() {
        tell(ItemNews::NoRoom);
    }
    container.contents = kept;
    container.contents.is_empty()
}

/// Retail throws a dropped item at most 2500 units, stops it 125 short of
/// what it hits, and rests it 125 above the ground below (`DropHeldItem`).
const THROW: f32 = 5.0;
const CLEARANCE: f32 = 0.25;

/// Where an item dropped along `ray` comes to rest, if there is ground
/// under it. Retail aims the throw from the cursor by its own angles; here
/// it goes along the pointer's ray. Retail's search for a free spot near
/// other items (`FindNearbyFreePosition`) is not ported.
fn drop_point(spatial: &SpatialQuery, ray: Ray3d, solid: &dyn Fn(Entity) -> bool) -> Option<Vec3> {
    let filter = SpatialQueryFilter::default();
    let reach = spatial
        .cast_ray_predicate(ray.origin, ray.direction, THROW, true, &filter, solid)
        .map_or(THROW, |hit| hit.distance);
    let above = ray.get_point((reach - CLEARANCE).max(0.0));
    let ground = spatial.cast_ray_predicate(above, Dir3::NEG_Y, 100.0, true, &filter, solid)?;
    Some(above - Vec3::Y * (ground.distance - CLEARANCE))
}

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

    #[test]
    fn a_bag_empties_into_the_pool_as_far_as_it_fits() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut bag = WorldItem {
            item: item(&mut ids, BAG_OF_ITEMS.0),
            hidden: false,
            contents: vec![item(&mut ids, 2), item(&mut ids, 3)],
        };
        let mut told = Vec::new();
        let mut party = PartyInventory::default();
        assert!(empty_into_pool(&mut bag, &mut party, &catalog, &mut |n| told.push(n)));
        assert_eq!(party.pool.len(), 2);
        assert_eq!(told.len(), 2);
        // A full pool keeps the rest in the bag.
        let mut full = PartyInventory {
            pool: vec![item(&mut ids, 1); crate::game::inventory::POOL_LIMIT],
            ..default()
        };
        bag.contents = vec![item(&mut ids, 2)];
        assert!(!empty_into_pool(&mut bag, &mut full, &catalog, &mut |n| told.push(n)));
        assert_eq!(bag.contents.len(), 1);
        assert_eq!(told.last(), Some(&ItemNews::NoRoom));
    }
}
