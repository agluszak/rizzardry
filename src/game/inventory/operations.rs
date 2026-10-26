//! The meaningful inventory activation used by UI cells and headless callers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::*;
use crate::game::data::GameData;
use crate::game::party::roster::PartyRoster;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    PickedUp,
    Swapped,
    Merged { units: u32 },
    Deposited { units: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferOutcome {
    pub kind: TransferKind,
    /// The actual item and quantity still held, including partial transfers.
    pub held: Option<(ItemInstanceId, Quantity)>,
}

#[derive(SystemParam)]
pub struct InventoryOperations<'w, 's> {
    party: ResMut<'w, PartyInventory>,
    roster: Res<'w, PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static mut Carried)>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
}

impl InventoryOperations<'_, '_> {
    /// Activate a resolved destination against the authoritative held item.
    /// Slot choice, merge/swap/deposit rules and all validation belong here.
    pub fn activate(&mut self, destination: ItemLocation) -> Result<TransferOutcome, Refusal> {
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        let kind = match destination {
            ItemLocation::Pool(row) => {
                if let Some(held) = self.party.held.as_ref() {
                    let before = held.item.units();
                    let row = row.min(self.party.pool.len());
                    let result = self.party.deposit_held(row, catalog);
                    let after = self.party.held.as_ref().map_or(0, |h| h.item.units());
                    if before == after {
                        result?;
                    }
                    TransferKind::Deposited { units: before - after }
                } else {
                    self.party.take_from_pool(row)?;
                    TransferKind::PickedUp
                }
            }
            ItemLocation::Member(member, slot) => {
                if !self.roster.0.contains(&member) {
                    return Err(Refusal::NotMember);
                }
                let (character, carried) =
                    self.members.get(member).map_err(|_| Refusal::NotMember)?;
                let target = carried.slot(slot).ok_or(Refusal::Nothing)?;
                let merge =
                    self.party.held.as_ref().zip(target.as_ref()).is_some_and(|(held, target)| {
                        held.item.item == target.item
                            && matches!(held.item.quantity, Quantity::Stack(_))
                    });
                if merge {
                    let held = &self.party.held.as_ref().unwrap().item;
                    let target = target.as_ref().unwrap();
                    let definition = catalog.get(held.item).ok_or(Refusal::MissingContent)?;
                    if matches!(slot, Slot::Equipment(_)) && !character.can_use(definition) {
                        return Err(Refusal::DoesNotFit);
                    }
                    if held.identified != target.identified
                        || !matches!(target.quantity, Quantity::Stack(n) if n < definition.maximum_quantity)
                    {
                        return Err(Refusal::DoesNotFit);
                    }
                    let (_, mut carried) = self.members.get_mut(member).unwrap();
                    let units = self.party.merge_held(Some(&mut carried), destination, catalog);
                    TransferKind::Merged { units: u32::from(units) }
                } else {
                    self.party.check_swap(character, carried, slot, catalog)?;
                    let kind = if self.party.held.is_none() {
                        TransferKind::PickedUp
                    } else {
                        TransferKind::Swapped
                    };
                    let (character, mut carried) = self.members.get_mut(member).unwrap();
                    self.party.swap_with_hand(character, &mut carried, member, slot, catalog)?;
                    kind
                }
            }
        };
        Ok(TransferOutcome {
            kind,
            held: self.party.held.as_ref().map(|h| (h.item.id, h.item.quantity)),
        })
    }
}

/// Drain owned contents through the same pool-transfer rules for chests and
/// world bags. Partial stack merges are reported; every remainder stays owned
/// by the source, in its original order.
pub fn transfer_contents(
    contents: &mut Vec<ItemInstance>,
    party: &mut PartyInventory,
    catalog: &ItemCatalog,
) -> Vec<(String, u32)> {
    let mut taken = Vec::new();
    let mut remaining = Vec::new();
    for item in contents.drain(..) {
        let before = item.units();
        let name = catalog.get(item.item).map(|d| item.display_name(d, false)).unwrap_or_default();
        let after = match party.place_in_pool(item, catalog) {
            Ok(()) => 0,
            Err(item) => {
                let units = item.units();
                remaining.push(item);
                units
            }
        };
        if before > after {
            taken.push((name, before - after));
        }
    }
    *contents = remaining;
    taken
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::QuantityKind;
    use crate::game::characters::{Gender, Profession, Race};
    use bevy::ecs::system::RunSystemOnce;

    fn item(ids: &mut ItemIds, count: u8) -> ItemInstance {
        ItemInstance {
            id: ids.allocate(),
            item: ItemId(0),
            quantity: Quantity::Stack(count),
            identified: true,
            spell_known: false,
            curse_known: false,
            curse_released: false,
            remarked: false,
        }
    }
    fn catalog() -> ItemCatalog {
        ItemCatalog(vec![ItemDefinition {
            id: ItemId(0),
            name: "arrows".into(),
            maximum_quantity: 10,
            quantity: QuantityKind::Stack,
            ..default()
        }])
    }
    fn activate(world: &mut World, at: ItemLocation) -> Result<TransferOutcome, Refusal> {
        world.run_system_once(move |mut op: InventoryOperations| op.activate(at)).unwrap()
    }

    #[test]
    fn one_headless_operation_picks_merges_and_deposits_the_actual_remainder() {
        let mut world = World::new();
        let mut catalogs = Assets::<ItemCatalog>::default();
        let items = catalogs.add(catalog());
        world.insert_resource(catalogs);
        world.insert_resource(GameData { items, loot: default(), species: default() });
        world.init_resource::<PartyRoster>();
        world.init_resource::<PartyInventory>();
        let mut ids = ItemIds::default();
        let source = item(&mut ids, 7);
        let source_id = source.id;
        let target = item(&mut ids, 8);
        let mut carried = Carried::default();
        carried.backpack[0] = Some(source);
        carried.backpack[1] = Some(target);
        let actor = world
            .spawn((
                Character::new("Archer", Race::Human, Gender::Male, Profession::Ranger),
                carried,
            ))
            .id();
        world.resource_mut::<PartyRoster>().0.insert(actor);
        let first = ItemLocation::Member(actor, Slot::Backpack(0));
        assert_eq!(
            activate(&mut world, first).unwrap(),
            TransferOutcome {
                kind: TransferKind::PickedUp,
                held: Some((source_id, Quantity::Stack(7)))
            }
        );
        let merged = activate(&mut world, ItemLocation::Member(actor, Slot::Backpack(1))).unwrap();
        assert_eq!(merged.kind, TransferKind::Merged { units: 2 });
        assert_eq!(merged.held, Some((source_id, Quantity::Stack(5))));
        assert_eq!(
            world.get::<Carried>(actor).unwrap().backpack[1].as_ref().unwrap().quantity,
            Quantity::Stack(10)
        );
        let deposited = activate(&mut world, ItemLocation::Pool(0)).unwrap();
        assert_eq!(deposited.kind, TransferKind::Deposited { units: 5 });
        assert_eq!(deposited.held, None);
        assert_eq!(world.resource::<PartyInventory>().pool[0].id, source_id);
        let retained = world
            .spawn((
                Character::new("NPC", Race::Human, Gender::Male, Profession::Ranger),
                Carried::default(),
            ))
            .id();
        let before = world.resource::<PartyInventory>().clone();
        assert_eq!(
            activate(&mut world, ItemLocation::Member(retained, Slot::Backpack(0))),
            Err(Refusal::NotMember)
        );
        assert_eq!(*world.resource::<PartyInventory>(), before);
        assert_eq!(
            activate(&mut world, ItemLocation::Member(actor, Slot::Backpack(999))),
            Err(Refusal::Nothing)
        );
        assert_eq!(*world.resource::<PartyInventory>(), before);
    }

    #[test]
    fn full_pool_transfers_report_partial_merges_and_retain_owned_remainders_in_order() {
        let mut ids = ItemIds::default();
        let mut party = PartyInventory::default();
        party.pool = (0..POOL_LIMIT).map(|_| item(&mut ids, 10)).collect();
        party.pool[17].quantity = Quantity::Stack(9);
        let mut contents = vec![item(&mut ids, 5), item(&mut ids, 3)];
        let identities: Vec<_> = contents.iter().map(|i| i.id).collect();
        assert_eq!(
            transfer_contents(&mut contents, &mut party, &catalog()),
            vec![("arrows".into(), 1)]
        );
        assert_eq!(contents.iter().map(|i| i.id).collect::<Vec<_>>(), identities);
        assert_eq!(contents[0].quantity, Quantity::Stack(4));
        assert_eq!(contents[1].quantity, Quantity::Stack(3));
        assert_eq!(
            party.pool.iter().map(ItemInstance::units).sum::<u32>()
                + contents.iter().map(ItemInstance::units).sum::<u32>(),
            POOL_LIMIT as u32 * 10 + 7
        );
        let before = (party.clone(), contents.clone());
        assert!(transfer_contents(&mut contents, &mut party, &catalog()).is_empty());
        assert_eq!((party, contents), before);
    }
}
