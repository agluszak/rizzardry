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
use crate::game::inventory::PartyInventory;
use crate::game::items::{ItemNews, ItemStack, Items, Place, Put, receive, unpack};
use crate::game::loot::roll_loot;
use crate::game::party::roster::PartyRoster;
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

/// Retail `Trigger::RunAction` case 3: a held item goes in; otherwise the
/// contents are drawn once, their gold goes to the party, a lone item to
/// the hand, and several are looked through.
pub(crate) fn use_container(
    In(entity): In<Entity>,
    mut containers: Query<(&mut Container, &PropMotion)>,
    content: If<Res<crate::game::data::Catalogs>>,
    attempts: Query<&DeviceAttempt>,
    mut purse: Single<&mut PartyInventory>,
    party: Single<(Entity, &PartyRoster)>,
    (profiles, held): (Query<&Character>, Items),
    mut rng: ResMut<GameRng>,
    mut outcomes: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) -> ContainerUse {
    let (party, roster) = *party;
    let (mut container, motion) = containers.get_mut(entity).unwrap();
    if let Some(item) = held.held(party) {
        container.contents.items.extend(held.stack(item, &content.items));
        commands.entity(item).despawn();
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
            found.into_iter().map(|id| ItemStack::rolled(&catalog[id], &mut rng, false, false)),
        );
        contents.gold += gold;
        container.table = None;
    }
    let contents = &mut container.contents;
    if contents.gold > 0 {
        purse.gold += contents.gold;
        outcomes.write(TriggerEffect::Gold(contents.gold));
        contents.gold = 0;
    }
    match contents.items.len() {
        0 => ContainerUse::Toggle,
        1 => {
            let stack = contents.items.pop().unwrap();
            let item = unpack(&mut commands, &catalog[stack.item], &stack);
            commands.trigger(Put { item, at: Place::Hand { party } });
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

/// Take the chosen stacks, each whole or `amount` of it, in the container's
/// order (retail `TransferSelectedItems`). Each stack goes as far as there
/// is room; what does not fit stays in the container exactly as it was, and
/// when nothing is left the container is done with. A member must be able
/// to act to receive anything.
pub(crate) fn take_from_containers(
    mut commands: Commands,
    mut requests: MessageReader<super::interaction::DeviceRequest>,
    attempts: Query<(&DeviceInteraction, &AttemptOn)>,
    mut containers: Query<&mut Container>,
    party: Single<Entity, With<PartyRoster>>,
    held: Items,
    members: Query<&crate::game::characters::Vitals, With<Character>>,
    (entering, battle): (
        Query<(), With<super::level::Destination>>,
        Query<(), With<crate::game::combat::Battle>>,
    ),
    content: Res<Catalogs>,
    mut news: MessageWriter<ItemNews>,
) {
    use super::interaction::DeviceAct;
    let party = *party;
    for request in requests.read() {
        let DeviceAct::Take { picks, to } = &request.act else { continue };
        let Ok((DeviceInteraction { work: Work::Looting }, &AttemptOn(device))) =
            attempts.get(request.on)
        else {
            continue;
        };
        if !entering.is_empty() || !battle.is_empty() {
            continue;
        }
        let receiver = match *to {
            Recipient::Party => None,
            Recipient::Member(member) => match members.get(member) {
                Ok(vitals) if vitals.can_act() => Some(member),
                _ => continue,
            },
        };
        let Ok(mut container) = containers.get_mut(device) else { continue };
        let catalog = &content.items;
        let items = &mut container.contents.items;
        // What is asked of each pick, and how much of it would be taken.
        let wanted: Vec<(usize, ItemStack)> = picks
            .iter()
            .filter_map(|&(index, amount)| {
                let mut stack = items.get(index)?.clone();
                match amount {
                    0 => None,
                    amount if amount == stack.count() => Some((index, stack)),
                    amount => Some((index, stack.split_off(amount)?)),
                }
            })
            .collect();
        let stacks: Vec<_> = wanted.iter().map(|(_, stack)| stack.clone()).collect();
        let taken = receive(&held, &mut commands, catalog, (party, receiver), (&stacks, true));
        let mut emptied = Vec::new();
        let mut refused = false;
        for ((index, asked), taking) in wanted.into_iter().zip(taken) {
            refused |= taking < asked.count();
            if taking == items[index].count() {
                emptied.push(index);
            } else if taking > 0 {
                items[index].split_off(taking);
            }
        }
        emptied.sort_unstable();
        for index in emptied.into_iter().rev() {
            items.remove(index);
        }
        if refused {
            news.write(ItemNews::NoRoom);
        }
        if items.is_empty() {
            commands.entity(request.on).despawn();
        }
    }
}
