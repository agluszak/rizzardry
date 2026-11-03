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
/// order (retail `TransferSelectedItems`). Each stack goes as far as there
/// is room; what does not fit stays in the container exactly as it was, and
/// when nothing is left the container is done with.
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
    let taken = world.run_system_cached_with(transfer, (device, picks.to_vec(), to)).unwrap()?;
    if taken.emptied {
        close_container(world, on);
    }
    Ok(())
}

/// What a take came to.
struct Taken {
    emptied: bool,
}

/// Move the picks from the container to their recipient, the container
/// staying where it is. A member must be able to act to receive anything.
/// Some of it moving is a take; none of it is a refusal.
fn transfer(
    In((device, picks, to)): In<(Entity, Vec<(StackId, u16)>, Recipient)>,
    mut containers: Query<&mut Container>,
    mut party: Single<&mut PartyInventory>,
    mut members: Query<(&Character, &crate::game::characters::Vitals, &mut Carried)>,
    content: Res<Catalogs>,
    mut ids: ResMut<ItemIds>,
    mut news: MessageWriter<ItemNews>,
) -> Result<Taken, Refusal> {
    let catalog = &content.items;
    let taker = match to {
        Recipient::Party => None,
        Recipient::Member(member) => {
            let (character, vitals, _) = members.get(member).map_err(|_| Refusal::DoesNotFit)?;
            if !vitals.can_act() {
                return Err(Refusal::DoesNotFit);
            }
            Some(character.name.clone())
        }
    };
    let mut container = containers.get_mut(device).unwrap();
    let items = &mut container.contents.items;
    let mut refused = false;
    let mut moved_any = false;
    for (id, amount) in picks {
        let index = items.iter().position(|s| s.id == id).unwrap();
        let name = items[index].display_name(&catalog[items[index].item], false);
        let mut receive = |part: ItemStack| match to {
            Recipient::Party => party.pool.receive(part, catalog),
            Recipient::Member(member) => members.get_mut(member).unwrap().2.receive(part, catalog),
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
        moved_any |= moved > 0;
        if moved > 0 {
            news.write(ItemNews::Takes { member: taker.clone(), item: name });
        }
        if moved < amount {
            refused = true;
        }
    }
    if refused {
        news.write(ItemNews::NoRoom);
    }
    if moved_any { Ok(Taken { emptied: items.is_empty() }) } else { Err(Refusal::NoRoom) }
}

/// Stop looking through the container, leaving the rest in it.
pub fn close_container(world: &mut World, on: Entity) {
    if looting(world, on).is_some() {
        world.despawn(on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, QuantityKind};
    use crate::game::characters::{Gender, Profession, Race, Vitals};
    use bevy::ecs::system::RunSystemOnce;

    /// Taking moves what fits and leaves the rest in the container, which
    /// stays on its device throughout; someone who cannot act takes nothing.
    #[test]
    fn a_take_moves_the_picks_and_leaves_the_rest_in_the_attached_container() {
        let mut world = World::new();
        world.init_resource::<ItemIds>();
        world.init_resource::<Messages<ItemNews>>();
        let definition = ItemDefinition {
            id: ItemId(0),
            name: "Ration".into(),
            class: EquipClass::Potion,
            quantity: QuantityKind::Stack,
            stack_limit: 5,
            ..default()
        };
        let mut ids = ItemIds::default();
        let rations = ItemStack::new(&definition, &mut ids, 3, true);
        let more = ItemStack::new(&definition, &mut ids, 2, true);
        let (taken_id, left_id) = (rations.id, more.id);
        world.insert_resource(Catalogs { items: ItemCatalog(vec![definition]), ..default() });
        let chest = world
            .spawn(Container {
                table: None,
                contents: Contents { items: vec![rations, more], gold: 0 },
            })
            .id();
        let character = Character::new("Out", Race::Human, Gender::Male, Profession::Fighter);
        let mut vitals = Vitals::new(&character);
        vitals.inflict(
            crate::game::characters::Condition::Unconscious,
            crate::game::characters::Lasting::Rounds(3),
        );
        let member = world.spawn((character, vitals, Carried::default())).id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);

        let picks = vec![(taken_id, 3)];
        let refused = world
            .run_system_once_with(transfer, (chest, picks.clone(), Recipient::Member(member)))
            .unwrap();
        assert!(matches!(refused, Err(Refusal::DoesNotFit)));
        assert_eq!(world.get::<Container>(chest).unwrap().contents.items.len(), 2);

        let taken = world.run_system_once_with(transfer, (chest, picks, Recipient::Party)).unwrap();
        assert!(!taken.unwrap().emptied);
        let left: Vec<_> =
            world.get::<Container>(chest).unwrap().contents.items.iter().map(|s| s.id).collect();
        assert_eq!(left, vec![left_id]);
        let pool = &world.get::<PartyInventory>(party).unwrap().pool;
        assert_eq!(pool.stacks.iter().map(ItemStack::count).sum::<u16>(), 3);
        let news: Vec<_> = world.resource_mut::<Messages<ItemNews>>().drain().collect();
        assert_eq!(news, vec![ItemNews::Takes { member: None, item: "Ration".into() }]);
    }
}
