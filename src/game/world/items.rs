//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use avian3d::prelude::{RigidBody, SpatialQuery, SpatialQueryFilter};
use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::items::{ItemCatalog, ItemDefinition, QuantityKind};
use crate::content::level::ItemPlacement;
use crate::game::data::GameData;
use crate::game::inventory::{Held, ItemIds, ItemInstance, PartyInventory, Quantity};
use crate::game::party::body::{Interact, PartyCamera};
use crate::game::world::level::Level;
use crate::game::world::props::{Prop, drawn_under_ray, owner, ray_under_pointer};

/// An item lying in the level.
#[derive(Component, Debug)]
#[require(Visibility)]
pub struct WorldItem {
    pub item: ItemInstance,
    /// Unseen, and out of reach, until found by searching.
    pub hidden: bool,
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
    items: Query<&WorldItem>,
    (data, catalogs, mut inventory): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        ResMut<PartyInventory>,
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
            WorldItem { item: held.item, hidden: false },
            Transform::from_translation(at),
            ChildOf(level),
        ));
        return;
    }
    let Some((hit, distance)) = hit else { return };
    let Some(entity) = owner(hit, |e| items.contains(e), &parents) else { return };
    let Ok(item) = items.get(entity) else { return };
    let Some(definition) = data.and_then(|d| catalogs.get(&d.items)?.get(item.item.item)) else {
        return;
    };
    // Retail's item picker for containers is not ported.
    if item.hidden || definition.container {
        return;
    }
    if distance > REACH {
        debug!("{} is out of reach: {distance:.1} m", definition.name);
        return;
    }
    info!("took {}", definition.name);
    inventory.held = Some(Held { item: item.item.clone(), origin: None });
    commands.entity(entity).despawn();
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
