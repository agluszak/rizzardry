//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::ecs::relationship::RelationshipTarget;
use bevy::prelude::*;

use super::interaction::{AttemptOn, DeviceAct, DeviceAttempt, DeviceInteraction, Work};
use super::props::PropMotion;
use super::triggers::TriggerEffect;
use crate::game::characters::Character;
use crate::game::inventory::PartyInventory;
use crate::game::items::{
    Bag, Give, Items, Lying, Place, Put, Rejoin, Stows, make, roll_amount, split,
};
use crate::game::loot::roll_loot;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;

/// A container: its contents are items stowed in it, drawn from its loot
/// table the first time it is opened and kept from then on (retail
/// `GenerateItemGroup`); whatever the party leaves or puts in it stays.
#[derive(Component, Clone, Debug)]
pub struct Container {
    /// The loot table still to be drawn; the first opening uses it up.
    pub(crate) table: Option<usize>,
    /// The gold drawn with it, which goes to the party's purse when it is
    /// opened.
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
        commands.trigger(Put { item, at: Place::Container(entity) });
        return ContainerUse::Deposited;
    }
    if attempts.contains(entity) {
        return ContainerUse::Busy;
    }
    // Devices with containers run only once the databases have loaded.
    let catalog = &content.items;
    let mut inside = held.stowed(entity).to_vec();
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
        for id in found {
            let definition = &catalog[id];
            let item =
                make(&mut commands, definition, roll_amount(definition, &mut rng, false), false);
            commands.trigger(Put { item, at: Place::Container(entity) });
            inside.push(item);
        }
        container.gold += gold;
        container.table = None;
    }
    if container.gold > 0 {
        purse.gold += container.gold;
        outcomes.write(TriggerEffect::Gold(container.gold));
        container.gold = 0;
    }
    match inside[..] {
        [] => ContainerUse::Toggle,
        [item] => {
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

/// Take the chosen items, each whole or `amount` of it, in the order asked
/// (retail `TransferSelectedItems`). Each goes as far as there is room; what
/// does not fit stays in the container exactly as it was. A member must be
/// able to act to receive anything.
pub(crate) fn take_from_containers(
    mut commands: Commands,
    mut requests: MessageReader<super::interaction::DeviceRequest>,
    attempts: Query<(&DeviceInteraction, &AttemptOn)>,
    party: Single<Entity, With<PartyRoster>>,
    held: Items,
    members: Query<&crate::game::characters::Vitals, With<Character>>,
    (entering, battle): (
        Query<(), With<super::level::Destination>>,
        Query<(), With<crate::game::combat::Battle>>,
    ),
) {
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
        let inside = held.stowed(device);
        for &(item, amount) in picks {
            let units = held.units(item);
            if !inside.contains(&item) || amount == 0 || u32::from(amount) > units {
                continue;
            }
            // Of a stack, a part is taken as an item of its own; what does
            // not fit of it goes back to the rest.
            let part = (u32::from(amount) < units).then(|| split(&mut commands, item, amount));
            let taking = part.unwrap_or(item);
            commands.trigger(Give {
                item: taking,
                to: receiver,
                party,
                pool_first: receiver.is_none(),
                told: true,
            });
            if part.is_some() {
                commands.trigger(Rejoin { part: taking, whole: item });
            }
        }
    }
}

/// A looked-through container with nothing left in it is done with.
pub(crate) fn emptied(
    event: On<Remove<Stows>>,
    containers: Query<&DeviceAttempt, With<Container>>,
    mut commands: Commands,
) {
    for attempt in containers.get(event.entity).iter().flat_map(|attempts| attempts.iter()) {
        commands.entity(attempt).try_despawn();
    }
}

/// A bag lying in the world with nothing left in it is gone.
pub(crate) fn bag_emptied(
    event: On<Remove<Stows>>,
    bags: Query<(), (With<Bag>, With<Lying>)>,
    mut commands: Commands,
) {
    if bags.contains(event.entity) {
        commands.entity(event.entity).try_despawn();
    }
}
