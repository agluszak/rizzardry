//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::prelude::*;

use super::interaction::{AttemptOn, DeviceInteraction, Work};
use crate::game::data::Catalogs;
use crate::game::inventory::{
    Carried, ItemIds, ItemNews, ItemOwner, ItemStack, PartyInventory, StackId, make_item,
};
use crate::game::loot::roll_loot;
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

/// The player takes the chosen stacks from the container being looted.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct TakeFromContainer {
    pub on: Entity,
    pub picks: Vec<(StackId, u16)>,
    pub to: ItemOwner,
}

/// Take the chosen stacks, each whole or `amount` of it, in the container's
/// order (retail `TransferSelectedItems`). Each stack goes as far as there
/// is room; what does not fit stays in the container exactly as it was, and
/// when nothing is left the container is done with. A member must be able
/// to act to receive anything, and nothing is taken while the party travels
/// or fights.
pub fn take_from_containers(
    mut requests: MessageReader<TakeFromContainer>,
    attempts: Query<(&DeviceInteraction, &AttemptOn)>,
    mut containers: Query<&mut Container>,
    mut party: Single<&mut PartyInventory>,
    mut members: Query<(
        &crate::game::characters::Character,
        &crate::game::characters::Vitals,
        &mut Carried,
    )>,
    (content, mut ids): (Res<Catalogs>, ResMut<ItemIds>),
    occupied: Query<(), Or<(With<super::level::Destination>, With<crate::game::combat::Battle>)>>,
    (mut news, mut commands): (MessageWriter<ItemNews>, Commands),
) {
    for TakeFromContainer { on, picks, to } in requests.read() {
        let device = match attempts.get(*on) {
            Ok((DeviceInteraction { work: Work::Looting, .. }, AttemptOn(device))) => *device,
            _ => continue,
        };
        if !occupied.is_empty() || picks.is_empty() {
            continue;
        }
        if let ItemOwner::Member(member) = *to
            && !members.get(member).unwrap().1.can_act()
        {
            continue;
        }
        let catalog = &content.items;
        let items = &mut containers.get_mut(device).unwrap().into_inner().contents.items;
        if items.is_empty() {
            continue;
        }
        let mut refused = false;
        for &(id, amount) in picks {
            let Some(index) = items.iter().position(|s| s.id == id) else { continue };
            let name = items[index].display_name(&catalog[items[index].item], false);
            let mut receive = |part: ItemStack| match *to {
                ItemOwner::Pool => party.pool.receive(part, catalog),
                ItemOwner::Member(member) => {
                    members.get_mut(member).unwrap().2.into_inner().receive(part, catalog)
                }
            };
            let moved = if amount == items[index].count() {
                let whole = items.remove(index);
                let units = whole.count();
                let rest = receive(whole);
                let left = rest.as_ref().map_or(0, ItemStack::count);
                if let Some(rest) = rest {
                    items.insert(index, rest);
                }
                units - left
            } else {
                items[index].give_part(amount, &mut ids, receive)
            };
            if moved > 0 {
                let member = match *to {
                    ItemOwner::Pool => None,
                    ItemOwner::Member(member) => Some(members.get(member).unwrap().0.name.clone()),
                };
                news.write(ItemNews::Takes { member, item: name });
            }
            refused |= moved < amount;
        }
        if refused {
            news.write(ItemNews::NoRoom);
        }
        // Stop looking through the container once it is empty.
        if items.is_empty() {
            commands.entity(*on).despawn();
        }
    }
}
