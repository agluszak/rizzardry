//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

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
use crate::game::world::props::{drawn_under_ray, owner, ray_under_pointer};

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

/// Take the item under the pointer into the hand, within reach
/// (retail `InteractWithWorldItem`). Retail only gets here with an empty
/// hand: a click with an item in hand drops it instead. Containers open
/// retail's item picker, which is not ported.
#[allow(clippy::too_many_arguments)]
pub fn take_item_in_view(
    mut commands: Commands,
    interact: Single<&ActionEvents, With<Action<Interact>>>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    camera: Query<(&Camera, &GlobalTransform), With<PartyCamera>>,
    mut meshes: MeshRayCast,
    sky: Query<(), With<RenderLayers>>,
    parents: Query<&ChildOf>,
    items: Query<&WorldItem>,
    (data, catalogs, mut inventory): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        ResMut<PartyInventory>,
    ),
) {
    if !interact.contains(ActionEvents::START) || inventory.held.is_some() {
        return;
    }
    let Ok((camera, transform)) = camera.single() else { return };
    let Some(ray) = ray_under_pointer(&pointers, camera, transform) else { return };
    let Some((hit, distance)) = drawn_under_ray(ray, &mut meshes, &sky) else { return };
    let Some(entity) = owner(hit, |e| items.contains(e), &parents) else { return };
    let Ok(item) = items.get(entity) else { return };
    let Some(definition) = data.and_then(|d| catalogs.get(&d.items)?.get(item.item.item)) else {
        return;
    };
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
