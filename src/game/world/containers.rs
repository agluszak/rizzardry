//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::prelude::*;

use super::interaction::{AttemptOn, DeviceInteraction, Work};
use crate::game::data::Catalogs;
use crate::game::inventory::{
    Carried, ItemIds, ItemNews, ItemStack, PartyInventory, Refusal, StackId, make_item,
};
use crate::game::loot::roll_loot;
use crate::game::party::roster::the_party;
use crate::game::rng::GameRng;

/// A container's contents, drawn from its loot table the first time it is
/// opened and kept from then on (retail `GenerateItemGroup`); whatever the
/// party leaves or puts in it stays.
#[derive(Component, Clone, Debug)]
pub struct Container {
    /// The loot table still to be drawn; the first opening uses it up.
    pub(crate) table: Option<usize>,
    pub contents: Contents,
}

#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub items: Vec<ItemStack>,
    pub gold: u32,
}

/// What using a container came to (retail action 3 with an item table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerUse {
    /// The held item went in; nothing else happens.
    Deposited,
    /// Its contents are already being looked through.
    Busy,
    /// Empty, or its one item went to the hand: it opens or closes.
    Toggle,
    /// Look through its contents, opening it first if it is shut.
    Look { open: bool },
}

/// Draw a container's loot table once (retail `GetAveragePartyLevel`
/// scales level-scaled tables). Retail reseeds its generator from the
/// trigger's own seed first; here the draw comes from the shared gameplay
/// generator, so contents depend on when the chest is first opened.
pub(crate) fn draw_loot(
    contents: &mut Contents,
    table: usize,
    party_level: u32,
    content: &crate::game::data::Catalogs,
    (ids, rng): (&mut ItemIds, &mut GameRng),
) {
    let catalog = &content.items;
    let (found, gold) = roll_loot(&content.loot.0[table], catalog, party_level, rng);
    contents
        .items
        .extend(found.into_iter().map(|id| make_item(&catalog[id], ids, rng, false, false)));
    contents.gold += gold;
}

/// Where taken items go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipient {
    /// The party's shared pool.
    Party,
    Member(Entity),
}

/// The container the named attempt looks through, if that attempt is
/// still under way.
pub fn looting(world: &World, on: Entity) -> Option<Entity> {
    let entity = world.get_entity(on).ok()?;
    match (entity.get::<DeviceInteraction>()?, entity.get::<AttemptOn>()?) {
        (DeviceInteraction { work: Work::Looting }, AttemptOn(device)) => Some(*device),
        _ => None,
    }
}

/// The player takes the chosen stacks from the container being looted.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct TakeFromContainer {
    pub on: Entity,
    pub picks: Vec<(StackId, u16)>,
    pub to: Recipient,
}

pub fn take_from_containers(world: &mut World) {
    let requests: Vec<_> = world.resource_mut::<Messages<TakeFromContainer>>().drain().collect();
    for TakeFromContainer { on, picks, to } in requests {
        if let Err(why) = take_from_container(world, on, &picks, to) {
            debug!("taking from the container refused: {why:?}");
        }
    }
}

/// Take the chosen stacks, each whole or `amount` of it, in the container's
/// order (retail `TransferSelectedItems`). Each stack goes as far as there
/// is room; what does not fit stays in the container exactly as it was, and
/// when nothing is left the container is done with. A member must be able
/// to act to receive anything.
pub fn take_from_container(
    world: &mut World,
    on: Entity,
    picks: &[(StackId, u16)],
    to: Recipient,
) -> Result<(), Refusal> {
    if crate::game::world::party_occupied(world) || picks.is_empty() {
        return Err(Refusal::Nothing);
    }
    let device = looting(world, on).ok_or(Refusal::Nothing)?;
    let party = the_party(world);
    if let Recipient::Member(member) = to
        && !world.get::<crate::game::characters::Vitals>(member).unwrap().can_act()
    {
        return Err(Refusal::DoesNotFit);
    }

    let mut container = world.entity_mut(device).take::<Container>().unwrap();
    let mut news = Vec::new();
    let mut refused = false;
    let mut moved_any = false;
    Catalogs::scope(world, |world, catalogs| {
        world.resource_scope(|world, mut ids: Mut<ItemIds>| {
            let catalog = &catalogs.items;
            let items = &mut container.contents.items;
            for &(id, amount) in picks {
                let index = items.iter().position(|s| s.id == id).unwrap();
                let definition = &catalog[items[index].item];
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
                moved_any |= moved > 0;
                if moved > 0 {
                    news.push(ItemNews::Takes {
                        member: match to {
                            Recipient::Party => None,
                            Recipient::Member(member) => Some(
                                world
                                    .get::<crate::game::characters::Character>(member)
                                    .unwrap()
                                    .name
                                    .clone(),
                            ),
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
        close_container(world, on);
    }
    // Some of it moving is a take; none of it is a refusal.
    if moved_any { Ok(()) } else { Err(Refusal::NoRoom) }
}

/// Stop looking through the container, leaving the rest in it.
pub fn close_container(world: &mut World, on: Entity) {
    if looting(world, on).is_some() {
        world.despawn(on);
    }
}
