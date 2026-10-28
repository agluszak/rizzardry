//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::prelude::*;

use super::interaction::{DeviceInteraction, Work};
use super::props::PropMotion;
use super::triggers::TriggerEffect;
use crate::content::items::ItemCatalog;
use crate::game::data::GameData;
use crate::game::inventory::{
    Carried, ItemIds, ItemNews, ItemStack, PartyInventory, Refusal, StackId, make_item,
};
use crate::game::loot::roll_loot;
use crate::game::party::roster::{PartyMember, the_party};
use crate::game::rng::GameRng;

/// A container's contents, drawn from its loot table the first time it is
/// opened and kept from then on (retail `GenerateItemGroup`); whatever the
/// party leaves or puts in it stays.
#[derive(Component, Clone, Debug)]
pub struct Container {
    pub(crate) table: Option<usize>,
    pub(crate) generated: bool,
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

/// Retail `Trigger::RunAction` case 3: a held item goes in; otherwise the
/// contents are drawn once, their gold goes to the party, a lone item to
/// the hand, and several are looked through.
pub(crate) fn use_container(
    In(entity): In<Entity>,
    mut containers: Query<(&mut Container, &PropMotion)>,
    content: crate::game::data::Content,
    interaction: Option<Res<DeviceInteraction>>,
    mut party: Single<&mut PartyInventory>,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    mut outcomes: MessageWriter<TriggerEffect>,
) -> ContainerUse {
    let (mut container, motion) = containers.get_mut(entity).unwrap();
    if let Some(held) = party.hand.held.take() {
        container.contents.items.push(held);
        return ContainerUse::Deposited;
    }
    if interaction.is_some_and(|i| i.device == entity) {
        return ContainerUse::Busy;
    }
    // Devices with containers run only once the databases have loaded.
    let catalog = content.items().unwrap();
    if !container.generated
        && let Some(table) = container.table
    {
        let loot = content.loot().unwrap();
        let (found, gold) = roll_loot(&loot.0[table], catalog, 1, &mut rng);
        let contents = &mut container.contents;
        contents.items.extend(
            found
                .into_iter()
                .filter_map(|id| catalog.get(id))
                .map(|d| make_item(d, &mut ids, &mut rng, false, false)),
        );
        contents.gold += gold;
        container.generated = true;
    }
    let contents = &mut container.contents;
    if contents.gold > 0 {
        party.gold += contents.gold;
        outcomes.write(TriggerEffect::Gold(contents.gold));
        contents.gold = 0;
    }
    match contents.items.len() {
        0 => ContainerUse::Toggle,
        1 => {
            party.hand.held = contents.items.pop();
            ContainerUse::Toggle
        }
        _ => ContainerUse::Look { open: motion.target > 0.0 },
    }
}

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
    if crate::game::world::party_occupied(world) {
        return Err(Refusal::Nothing);
    }
    let device = looting(world).ok_or(Refusal::Nothing)?;
    let party = the_party(world).ok_or(Refusal::NotMember)?;
    if let Recipient::Member(member) = to {
        if !world.get::<PartyMember>(member).is_some_and(|m| m.0 == party) {
            return Err(Refusal::NotMember);
        }
        let vitals =
            world.get::<crate::game::characters::Vitals>(member).ok_or(Refusal::NotMember)?;
        if !vitals.can_act() || world.get::<Carried>(member).is_none() {
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
            let count = items[index].count();
            if amount == 0 || amount > count {
                continue;
            }
            let (taking, whole) = if amount == count {
                (items.remove(index), true)
            } else {
                let part = world.resource_scope(|_, mut ids: Mut<ItemIds>| {
                    items[index].split_off(amount, &mut ids).expect("validated strict partial")
                });
                (part, false)
            };
            let name = taking.display_name(definition, false);
            let before = taking.units();
            let rest = match to {
                Recipient::Party => {
                    world.get_mut::<PartyInventory>(party).unwrap().pool.receive(taking, catalog)
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
