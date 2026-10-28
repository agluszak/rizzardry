//! Items lying in the world. Each is an entity owning one item until the
//! party takes it (retail `W8WorldItem`, `ItemManager.cpp`).

use crate::game::party::roster::the_party;
use crate::game::world::level::LevelOwned;
use bevy::prelude::*;

use crate::game::data::GameData;

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
            GroundLoot::Pile { identified, .. } => {
                if *identified { &definition.name } else { &definition.unidentified_name }.clone()
            }
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
    let lying = match items.len() {
        0 => return,
        1 => WorldItem::loose(items.remove(0), false),
        _ if catalog.get(BAG_OF_ITEMS).is_some() => WorldItem {
            loot: GroundLoot::Pile {
                appearance: BAG_OF_ITEMS,
                identified: catalog.get(BAG_OF_ITEMS).unwrap().auto_identify,
                contents: items,
            },
            hidden: false,
        },
        _ => {
            for item in items {
                commands.spawn((
                    Name::new("loot"),
                    WorldItem::loose(item, false),
                    Transform::from_translation(feet + Vec3::Y * CLEARANCE),
                    LevelOwned(level),
                ));
            }
            return;
        }
    };
    commands.spawn((
        Name::new("loot"),
        lying,
        Transform::from_translation(feet + Vec3::Y * CLEARANCE),
        LevelOwned(level),
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
pub fn take_item(world: &mut World, target: Entity, distance: f32) -> Result<(), Refusal> {
    if !world.run_system_cached(crate::game::screen::party_free).unwrap_or(false) {
        return Err(Refusal::DoesNotFit);
    }
    if !distance.is_finite() || !(0.0..=REACH).contains(&distance) {
        return Err(Refusal::DoesNotFit);
    }
    let catalog = GameData::items(world).ok_or(Refusal::MissingContent)?;
    let item = world.get::<WorldItem>(target).ok_or(Refusal::Nothing)?;
    let owner = world.get::<LevelOwned>(target).ok_or(Refusal::Nothing)?.0;
    if world.get::<super::level::Level>(owner).is_none() || item.hidden {
        return Err(Refusal::DoesNotFit);
    }
    catalog.get(item.appearance()).ok_or(Refusal::MissingContent)?;
    let party = the_party(world).ok_or(Refusal::Nothing)?;
    if world.get::<PartyInventory>(party).unwrap().hand.held.is_some() {
        return Err(Refusal::HandFull);
    }
    let mut item = world.entity_mut(target).take::<WorldItem>().unwrap();
    if let GroundLoot::Pile { contents, .. } = &mut item.loot {
        let handle = world.resource::<GameData>().items.clone();
        let taken = world.resource_scope(|world, catalogs: Mut<Assets<ItemCatalog>>| {
            let catalog = catalogs.get(&handle).unwrap();
            world.get_mut::<PartyInventory>(party).unwrap().pool.collect(contents, catalog)
        });
        for (name, _) in taken {
            world.write_message(ItemNews::Takes { member: None, item: name });
        }
        if !contents.is_empty() {
            world.entity_mut(target).insert(item);
            world.write_message(ItemNews::NoRoom);
            return Ok(());
        }
    } else if let GroundLoot::Loose(stack) = item.loot {
        world.get_mut::<PartyInventory>(party).unwrap().hand.held = Some(stack);
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
    let party = the_party(world).ok_or(Refusal::Nothing)?;
    let held =
        world.get::<PartyInventory>(party).unwrap().hand.held.as_ref().ok_or(Refusal::Nothing)?;
    let definition =
        GameData::items(world).and_then(|c| c.get(held.item)).ok_or(Refusal::MissingContent)?;
    if definition.crucial {
        return Err(Refusal::Crucial);
    }
    let held =
        world.get_mut::<PartyInventory>(party).unwrap().hand.held.take().ok_or(Refusal::Nothing)?;
    Ok(world
        .spawn((
            Name::new("dropped item"),
            WorldItem::loose(held, false),
            Transform::from_translation(at),
            LevelOwned(level),
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
    use crate::game::{inventory::make_item, rng::GameRng};

    fn catalog() -> ItemCatalog {
        let mut items: Vec<ItemDefinition> = (0..=BAG_OF_ITEMS.0)
            .map(|id| ItemDefinition { id: ItemId(id), name: format!("item {id}"), ..default() })
            .collect();
        items[usize::from(BAG_OF_ITEMS.0)].container = true;
        ItemCatalog(items)
    }

    fn item(ids: &mut ItemIds, id: u16) -> ItemStack {
        let definition = ItemDefinition { id: ItemId(id), ..default() };
        make_item(&definition, ids, &mut GameRng::seeded(1), false, true)
    }

    #[test]
    fn several_drops_lie_in_a_bag_one_lies_alone() {
        let (catalog, mut ids) = (catalog(), ItemIds::default());
        let mut world = World::new();
        let level = world.spawn_empty().id();
        let (one, two) = (item(&mut ids, 1), vec![item(&mut ids, 2), item(&mut ids, 3)]);
        let mut commands = world.commands();
        drop_loot(&mut commands, level, Vec3::ZERO, vec![one], &catalog);
        drop_loot(&mut commands, level, Vec3::X, two, &catalog);
        drop_loot(&mut commands, level, Vec3::Y, vec![], &catalog);
        world.flush();
        let mut lying: Vec<(u16, usize)> = world
            .query::<&WorldItem>()
            .iter(&world)
            .map(|w| {
                (
                    w.appearance().0,
                    match &w.loot {
                        GroundLoot::Loose(_) => 0,
                        GroundLoot::Pile { contents, .. } => contents.len(),
                    },
                )
            })
            .collect();
        lying.sort();
        assert_eq!(lying, [(1, 0), (BAG_OF_ITEMS.0, 2)]);
        // Missing container art/definition must not lose gameplay ownership.
        drop_loot(
            &mut world.commands(),
            level,
            Vec3::Z,
            vec![item(&mut ids, 4), item(&mut ids, 5)],
            &ItemCatalog::default(),
        );
        world.flush();
        assert_eq!(world.query::<&WorldItem>().iter(&world).count(), 4);
        assert!(
            world
                .query_filtered::<&LevelOwned, With<WorldItem>>()
                .iter(&world)
                .all(|owner| owner.0 == level)
        );
        assert_eq!(
            world.get::<crate::game::world::level::LevelEntities>(level).unwrap().entities().len(),
            4
        );
    }
}
