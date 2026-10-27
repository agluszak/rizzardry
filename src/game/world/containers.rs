//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::prelude::*;

use super::interaction::{DeviceInteraction, Work};
use super::triggers::Container;
use crate::content::items::ItemCatalog;
use crate::game::data::GameData;
use crate::game::inventory::{Carried, ItemIds, ItemNews, PartyInventory, Refusal, StackId};
use crate::game::party::roster::PartyRoster;

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
/// order (retail `TransferSelectedItems`). Each stack that does not fit
/// stays, its remainder merged back; when nothing is left the container
/// is done with. A member must be able to act to receive anything.
pub fn take_from_container(
    world: &mut World,
    picks: &[(StackId, u16)],
    to: Recipient,
) -> Result<(), Refusal> {
    let device = looting(world).ok_or(Refusal::Nothing)?;
    if let Recipient::Member(member) = to {
        if !world.resource::<PartyRoster>().0.contains(&member) {
            return Err(Refusal::NotMember);
        }
        let vitals = world.get(member).ok_or(Refusal::NotMember)?;
        if !crate::game::camp::can_act(vitals) || world.get::<Carried>(member).is_none() {
            return Err(Refusal::DoesNotFit);
        }
    }
    let handle = world.get_resource::<GameData>().ok_or(Refusal::MissingContent)?.items.clone();
    let mut container = world.entity_mut(device).take::<Container>().ok_or(Refusal::Nothing)?;
    let mut news = Vec::new();
    let mut refused = false;
    world.resource_scope(|world, catalogs: Mut<Assets<ItemCatalog>>| {
        let Some(catalog) = catalogs.get(&handle) else { return };
        let items = &mut container.contents.items;
        for &(id, amount) in picks {
            let Some(index) = items.iter().position(|s| s.id == id) else { continue };
            let Some(definition) = catalog.get(items[index].item) else { continue };
            let split = world.resource_scope(|_, mut ids: Mut<ItemIds>| {
                items[index].split_off(amount, &mut ids)
            });
            let (taking, whole) = match split {
                Some(part) => (part, false),
                None => (items.remove(index), true),
            };
            let name = taking.display_name(definition, false);
            let before = taking.units();
            let rest = match to {
                Recipient::Party => {
                    world.resource_mut::<PartyInventory>().pool.receive(taking, catalog)
                }
                Recipient::Member(member) => {
                    world.get_mut::<Carried>(member).unwrap().receive(taking, catalog)
                }
            };
            if rest.as_ref().is_none_or(|rest| rest.units() < before) {
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
            if let Some(rest) = rest {
                refused = true;
                if whole {
                    items.insert(index, rest);
                } else if let Some(rest) = items[index].absorb(rest, definition) {
                    items.insert(index + 1, rest);
                }
            }
        }
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
