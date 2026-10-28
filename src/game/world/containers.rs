//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::prelude::*;

use super::interaction::{DeviceInteraction, Work};
use super::triggers::Container;
use crate::content::items::ItemCatalog;
use crate::game::data::GameData;
use crate::game::inventory::{
    Carried, ItemIds, ItemNews, ItemStack, PartyInventory, Refusal, StackId,
};
use crate::game::party::roster::{PartyMember, the_party};

/// Where taken items go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipient {
    /// The party's shared pool.
    Party,
    Member(Entity),
}

/// The container being looked through, if any.
pub fn looting(world: &World) -> Option<Entity> {
    match world.get_resource::<DeviceInteraction>() {
        Some(DeviceInteraction { device, work: Work::Looting }) => Some(*device),
        _ => None,
    }
}

/// Take the chosen stacks, each whole or `amount` of it, in the container's
/// order (retail `TransferSelectedItems`). The whole request is checked
/// first: an unknown stack, a bad amount or missing item content refuses
/// it with nothing changed. Then each stack goes as far as there is room;
/// what does not fit stays in the container exactly as it was, and when
/// nothing is left the container is done with. A member must be able to
/// act to receive anything.
pub fn take_from_container(
    world: &mut World,
    picks: &[(StackId, u16)],
    to: Recipient,
) -> Result<(), Refusal> {
    if world.query::<&super::level::Destination>().iter(world).next().is_some()
        || world.query::<&crate::game::combat::Battle>().iter(world).next().is_some()
    {
        return Err(Refusal::Nothing);
    }
    let device = looting(world).ok_or(Refusal::Nothing)?;
    let party = the_party(world).ok_or(Refusal::NotMember)?;
    if let Recipient::Member(member) = to {
        if !world.get::<PartyMember>(member).is_some_and(|m| m.0 == party) {
            return Err(Refusal::NotMember);
        }
        let vitals = world.get(member).ok_or(Refusal::NotMember)?;
        if !crate::game::camp::can_act(vitals) || world.get::<Carried>(member).is_none() {
            return Err(Refusal::DoesNotFit);
        }
    }
    let handle = world.get_resource::<GameData>().ok_or(Refusal::MissingContent)?.items.clone();
    let catalog = world
        .get_resource::<Assets<ItemCatalog>>()
        .and_then(|catalogs| catalogs.get(&handle))
        .ok_or(Refusal::MissingContent)?;
    let container = world.get::<Container>(device).ok_or(Refusal::Nothing)?;
    for &(id, amount) in picks {
        let stack = container.contents.items.iter().find(|s| s.id == id).ok_or(Refusal::Nothing)?;
        catalog.get(stack.item).ok_or(Refusal::MissingContent)?;
        if amount == 0 || amount > stack.count() {
            return Err(Refusal::Nothing);
        }
    }
    if picks.is_empty() {
        return Err(Refusal::Nothing);
    }

    let mut container = world.entity_mut(device).take::<Container>().unwrap();
    let mut news = Vec::new();
    let mut refused = false;
    world.resource_scope(|world, catalogs: Mut<Assets<ItemCatalog>>| {
        world.resource_scope(|world, mut ids: Mut<ItemIds>| {
            let catalog = catalogs.get(&handle).unwrap();
            let items = &mut container.contents.items;
            for &(id, amount) in picks {
                let Some(index) = items.iter().position(|s| s.id == id) else { continue };
                let definition = catalog.get(items[index].item).unwrap();
                let name = items[index].display_name(definition, false);
                let mut receive = |part: ItemStack| {
                    let part_id = part.id;
                    match to {
                        Recipient::Party => {
                            let pool = &mut world
                                .get_mut::<PartyInventory>(party)
                                .unwrap()
                                .into_inner()
                                .pool;
                            let rest = pool.receive(part, catalog);
                            (rest, pool.stacks.iter().any(|s| s.id == part_id))
                        }
                        Recipient::Member(member) => {
                            let carried = world.get_mut::<Carried>(member).unwrap().into_inner();
                            let rest = carried.receive(part, catalog);
                            (rest, carried.find_stack(part_id).is_some())
                        }
                    }
                };
                let moved = if amount == items[index].count() {
                    let whole = items.remove(index);
                    let units = whole.count();
                    let (rest, _) = receive(whole);
                    let left = rest.as_ref().map_or(0, ItemStack::count);
                    if let Some(rest) = rest {
                        items.insert(index, rest);
                    }
                    units - left
                } else {
                    items[index].give_part(amount, &mut ids, receive)
                };
                if moved > 0 {
                    news.push(ItemNews::Takes {
                        member: match to {
                            Recipient::Party => None,
                            Recipient::Member(member) => world
                                .get::<crate::game::characters::Character>(member)
                                .map(|c| c.name.clone()),
                        },
                        item: name,
                    });
                }
                if moved < amount {
                    refused = true;
                }
            }
        });
    });
    let empty = container.contents.items.is_empty();
    world.entity_mut(device).insert(container);
    for news in news {
        world.write_message(news);
    }
    if refused {
        world.write_message(ItemNews::NoRoom);
    }
    if empty {
        close_container(world);
    }
    Ok(())
}

/// Stop looking through the container, leaving the rest in it.
pub fn close_container(world: &mut World) {
    if looting(world).is_some() {
        world.remove_resource::<DeviceInteraction>();
    }
}
