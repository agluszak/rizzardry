//! Looking through a container (retail `W8TriggerItemPickerDialog`). The
//! container owns its contents throughout; taking moves chosen stacks, or
//! part of one, to the party's pool or a member, and whatever does not fit
//! stays where it was.

use bevy::ecs::relationship::RelationshipTarget;
use bevy::prelude::*;

use super::interaction::{AttemptOn, DeviceAttempt, DeviceInteraction, Work};
use super::props::PropMotion;
use super::triggers::{Act, Open, TriggerEffect};
use crate::game::characters::Calling;
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

/// A container's lid is used (retail `Trigger::RunAction` case 3): a held
/// item goes in, and that is all. Otherwise its contents are drawn, once,
/// their gold goes to the party, a lone item to the hand, and the lid moves;
/// several are looked through, the lid opening first if it is shut. One
/// already being looked through does nothing.
#[allow(clippy::type_complexity)]
pub(crate) fn open(
    act: On<Act<Open>>,
    mut containers: Query<(&mut Container, &PropMotion, Has<DeviceAttempt>)>,
    content: Res<crate::game::data::Catalogs>,
    mut purse: Single<&mut PartyInventory>,
    party: Single<(Entity, &PartyRoster)>,
    (profiles, held): (Query<&Calling>, Items),
    mut rng: ResMut<GameRng>,
    mut outcomes: MessageWriter<TriggerEffect>,
    mut commands: Commands,
) -> Result {
    let (entity, doing, Open(animation)) = (act.device, act.doing, act.what);
    let (party, roster) = *party;
    let (mut container, motion, busy) = containers.get_mut(entity)?;
    if let Some(item) = held.held(party) {
        commands.trigger(Put { item, at: Place::Container(entity) });
        return Ok(());
    }
    if busy {
        doing.decline(&mut outcomes);
        return Ok(());
    }
    let catalog = &content.items;
    let mut inside = held.stowed(entity).to_vec();
    if let Some(table) = container.table {
        // Level-scaled tables draw for the party's average level (retail
        // `GetAveragePartyLevel`). Retail reseeds its generator from the
        // trigger's own seed first; here the draw comes from the shared
        // gameplay generator, so contents depend on when the chest is first
        // opened.
        let party_level = crate::game::party::average_level(
            profiles.iter_many(roster.iter()).matched().map(|c| c.level()),
        );
        let (found, gold) = roll_loot(&content.loot.0[table], catalog, party_level, &mut rng);
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
    let lid = Act { device: entity, doing, what: animation };
    match inside[..] {
        [] => commands.trigger(lid),
        [item] => {
            commands.trigger(Put { item, at: Place::Hand });
            commands.trigger(lid);
        }
        _ => {
            commands.spawn((DeviceInteraction { work: Work::Looting }, AttemptOn(entity)));
            if motion.target <= 0.0 {
                commands.trigger(lid);
            }
        }
    }
    Ok(())
}

/// Where taken items go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipient {
    /// The party's shared pool.
    Party,
    Member(Entity),
}

/// The party takes the chosen items, and how many of each, from the
/// container it is looking through.
#[derive(EntityEvent, Clone, Debug)]
pub struct Take {
    #[event_target]
    pub attempt: Entity,
    pub picks: Vec<(Entity, u16)>,
    pub to: Recipient,
}

/// Take the chosen items, each whole or `amount` of it, in the order asked
/// (retail `TransferSelectedItems`). Each goes as far as there is room; what
/// does not fit stays in the container exactly as it was. A member must be
/// able to act to receive anything.
pub(crate) fn take(
    take: On<Take>,
    mut commands: Commands,
    attempts: Query<(&DeviceInteraction, &AttemptOn)>,
    held: Items,
    able: Query<(), (With<Calling>, With<crate::game::actors::CanAct>)>,
    (entering, battle): (
        Query<(), With<super::level::Destination>>,
        Option<Res<crate::game::combat::Battle>>,
    ),
) -> Result {
    let (interaction, &AttemptOn(device)) = attempts.get(take.attempt)?;
    // Only an attempt at looting takes anything.
    let DeviceInteraction { work: Work::Looting } = interaction else { return Ok(()) };
    if !entering.is_empty() || battle.is_some() {
        return Ok(());
    }
    let receiver = match take.to {
        Recipient::Party => None,
        Recipient::Member(member) if able.contains(member) => Some(member),
        Recipient::Member(_) => return Ok(()),
    };
    let inside = held.stowed(device);
    for &(item, amount) in &take.picks {
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
            pool_first: receiver.is_none(),
            told: true,
        });
        if part.is_some() {
            commands.trigger(Rejoin { part: taking, whole: item });
        }
    }
    Ok(())
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
