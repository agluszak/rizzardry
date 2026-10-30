//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::prelude::*;

use super::interaction::{AttemptOn, DeviceAttempt, DeviceInteraction, Work};
use super::props::PropMotion;
use super::triggers::TriggerEffect;
use crate::game::characters::Character;
use crate::game::data::Catalogs;
use crate::game::inventory::{
    Carried, ItemIds, ItemNews, ItemStack, PartyInventory, Refusal, StackId, make_item,
};
use crate::game::loot::roll_loot;
use crate::game::party::roster::{PartyMember, PartyRoster, the_party};
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

#[cfg(test)]
impl Container {
    /// An opened container holding these stacks.
    pub(crate) fn holding(items: Vec<ItemStack>) -> Self {
        Container { table: None, contents: Contents { items, gold: 0 } }
    }
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
    content: If<Res<crate::game::data::Catalogs>>,
    attempts: Query<&DeviceAttempt>,
    mut party: Single<&mut PartyInventory>,
    (roster, profiles): (Single<&PartyRoster>, Query<&Character>),
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    mut outcomes: MessageWriter<TriggerEffect>,
) -> ContainerUse {
    let (mut container, motion) = containers.get_mut(entity).unwrap();
    if let Some(held) = party.hand.held.take() {
        container.contents.items.push(held);
        return ContainerUse::Deposited;
    }
    if attempts.contains(entity) {
        return ContainerUse::Busy;
    }
    // Devices with containers run only once the databases have loaded.
    let catalog = &content.items;
    if let Some(table) = container.table {
        let loot = &content.loot;
        // Level-scaled tables draw for the party's average level (retail
        // `GetAveragePartyLevel`). Retail reseeds its generator from the
        // trigger's own seed first; here the draw comes from the shared
        // gameplay generator, so contents depend on when the chest is first
        // opened.
        let party_level = crate::game::party::average_level(
            profiles.iter_many(roster.iter()).matched().map(|c| c.level()),
        );
        let (found, gold) = roll_loot(&loot.0[table], catalog, party_level, &mut rng);
        let contents = &mut container.contents;
        contents.items.extend(
            found
                .into_iter()
                .filter_map(|id| catalog.get(id))
                .map(|d| make_item(d, &mut ids, &mut rng, false, false)),
        );
        contents.gold += gold;
        container.table = None;
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

/// The container the named attempt looks through, if that attempt is
/// still under way.
pub fn looting(world: &World, on: Entity) -> Option<Entity> {
    let entity = world.get_entity(on).ok()?;
    match (entity.get::<DeviceInteraction>()?, entity.get::<AttemptOn>()?) {
        (DeviceInteraction { work: Work::Looting }, AttemptOn(device)) => Some(*device),
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
    on: Entity,
    picks: &[(StackId, u16)],
    to: Recipient,
) -> Result<(), Refusal> {
    if crate::game::world::party_occupied(world) {
        return Err(Refusal::Nothing);
    }
    let device = looting(world, on).ok_or(Refusal::Nothing)?;
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
    let container = world.get::<Container>(device).ok_or(Refusal::Nothing)?;
    for (index, &(id, amount)) in picks.iter().enumerate() {
        // One pick per stack: two would be checked against the same count.
        if picks[..index].iter().any(|&(earlier, _)| earlier == id) {
            return Err(Refusal::Nothing);
        }
        let stack = container.contents.items.iter().find(|s| s.id == id).ok_or(Refusal::Nothing)?;
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
    let mut moved_any = false;
    Catalogs::scope(world, |world, catalogs| {
        world.resource_scope(|world, mut ids: Mut<ItemIds>| {
            let catalog = &catalogs.items;
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
                moved_any |= moved > 0;
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
