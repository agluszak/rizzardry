//! The meaningful inventory activation used by UI cells and headless callers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::*;
use crate::game::data::GameData;
use crate::game::party::roster::PartyRoster;

#[derive(SystemParam)]
pub struct InventoryOperations<'w, 's> {
    party: Single<'w, 's, &'static mut PartyInventory>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, &'static mut Carried)>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
    battle: Option<Single<'w, 's, &'static crate::game::combat::Battle>>,
}

impl InventoryOperations<'_, '_> {
    /// Give the held item to an identified actor. Eligibility and battle pool
    /// precedence are rules here; the clicked portrait only supplies its target.
    pub fn give_to(&mut self, member: Entity) -> Result<Option<ItemNews>, Refusal> {
        let Some(held) = self.party.held.as_ref() else { return Ok(None) };
        if !self.roster.contains(&member) {
            return Err(Refusal::NotMember);
        }
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        let (profile, vitals, mut carried) =
            self.members.get_mut(member).map_err(|_| Refusal::NotMember)?;
        let definition = catalog.get(held.item.item).ok_or(Refusal::MissingContent)?;
        let item = held.item.display_name(definition, false);
        let name = profile.name.clone();
        let eligible = crate::game::camp::can_act(vitals);
        let pool_first = !eligible || self.battle.is_some();
        // Try the pool before mutating carried items when that is the rule.
        let mut result = if pool_first { self.party.give_held(None, true, catalog) } else { None };
        if eligible && (result.is_none() || result == Some(Given::NoRoom)) {
            result = self.party.give_held(Some(&mut carried), false, catalog);
        }
        Ok(result.map(|given| match given {
            Given::Member => ItemNews::Takes { member: Some(name), item },
            Given::Pool => ItemNews::Takes { member: None, item },
            Given::NoRoom => ItemNews::NoRoom,
        }))
    }

    /// Return the hand to the reviewed member, falling back to the pool. A
    /// departed member cannot receive it; overflow remains owned by the hand.
    pub fn put_away(&mut self, member: Entity) -> Result<(), Refusal> {
        if self.party.held.is_none() {
            return Ok(());
        }
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        if self.roster.contains(&member)
            && let Ok((_, _, mut carried)) = self.members.get_mut(member)
        {
            self.party.put_away_held(&mut carried, catalog);
        } else {
            self.party.give_held(None, false, catalog);
        }
        Ok(())
    }

    /// Activate a resolved destination against the authoritative held item.
    /// Slot choice, merge/swap/deposit rules and all validation belong here.
    /// Returns the actual item and quantity left in hand after the transfer.
    pub fn activate(
        &mut self,
        destination: ItemLocation,
    ) -> Result<Option<(ItemInstanceId, Quantity)>, Refusal> {
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        match destination {
            ItemLocation::Pool(row) => {
                if let Some(held) = self.party.held.as_ref() {
                    let before = held.item.units();
                    let row = row.min(self.party.pool.len());
                    let result = self.party.deposit_held(row, catalog);
                    let after = self.party.held.as_ref().map_or(0, |h| h.item.units());
                    if before == after {
                        result?;
                    }
                } else {
                    self.party.take_from_pool(row)?;
                }
            }
            ItemLocation::Member(member, slot) => {
                if !self.roster.contains(&member) {
                    return Err(Refusal::NotMember);
                }
                let (character, vitals, mut carried) =
                    self.members.get_mut(member).map_err(|_| Refusal::NotMember)?;
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
                    self.party.merge_held(Some(&mut carried), destination, catalog);
                } else {
                    self.party.check_swap(character, vitals, &carried, slot, catalog)?;
                    self.party.swap_with_hand(
                        character,
                        vitals,
                        &mut carried,
                        member,
                        slot,
                        catalog,
                    )?;
                }
            }
        }
        Ok(self.party.held.as_ref().map(|h| (h.item.id, h.item.quantity)))
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
