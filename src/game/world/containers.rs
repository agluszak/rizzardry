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
use crate::game::inventory::{Carried, ItemIds, ItemNews, ItemStack, PartyInventory, make_item};
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
            found.into_iter().map(|id| make_item(&catalog[id], &mut ids, &mut rng, false, false)),
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
    mut inventory: Single<&mut PartyInventory>,
    mut members: Query<(&Character, &crate::game::characters::Vitals, &mut Carried)>,
    (entering, battle): (
        Query<(), With<super::level::Destination>>,
        Query<(), With<crate::game::combat::Battle>>,
    ),
    content: Res<Catalogs>,
    mut ids: ResMut<ItemIds>,
    mut news: MessageWriter<ItemNews>,
) {
    use super::interaction::DeviceAct;
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
        let mut receiver = match *to {
            Recipient::Party => None,
            Recipient::Member(member) => match members.get_mut(member) {
                Ok((character, vitals, carried)) if vitals.can_act() => {
                    Some((character.name.clone(), carried))
                }
                _ => continue,
            },
        };
        let Ok(mut container) = containers.get_mut(device) else { continue };
        let catalog = &content.items;
        let items = &mut container.contents.items;
        let mut refused = false;
        for &(id, amount) in picks {
            let Some(index) = items.iter().position(|s| s.id == id) else { continue };
            let definition = &catalog[items[index].item];
            let name = items[index].display_name(definition, false);
            let mut receive = |part: ItemStack| match receiver.as_mut() {
                None => inventory.pool.receive(part, catalog),
                Some((_, carried)) => carried.receive(part, catalog),
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
                let member = receiver.as_ref().map(|(name, _)| name.clone());
                news.write(ItemNews::Takes { member, item: name });
            }
            refused |= moved < amount;
        }
        if refused {
            news.write(ItemNews::NoRoom);
        }
        if items.is_empty() {
            commands.entity(request.on).despawn();
        }
    }
}
