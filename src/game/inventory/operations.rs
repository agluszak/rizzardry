//! The meaningful inventory activation used by UI cells and headless callers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::*;
use crate::game::data::GameData;
use crate::game::party::roster::PartyRoster;

#[derive(SystemParam)]
pub struct InventoryOperations<'w, 's> {
    party: ResMut<'w, PartyInventory>,
    roster: Res<'w, PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals, &'static mut Carried)>,
    data: Option<Res<'w, GameData>>,
    catalogs: Res<'w, Assets<ItemCatalog>>,
    battle: Option<Res<'w, crate::game::combat::Battle>>,
}

impl InventoryOperations<'_, '_> {
    /// Give the held item to an identified actor. Eligibility and battle pool
    /// precedence are rules here; the clicked portrait only supplies its target.
    pub fn give_to(&mut self, member: Entity) -> Result<Option<ItemNews>, Refusal> {
        let Some(held) = self.party.hand.held.as_ref() else { return Ok(None) };
        if !self.roster.0.contains(&member) {
            return Err(Refusal::NotMember);
        }
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        let (profile, vitals, mut carried) =
            self.members.get_mut(member).map_err(|_| Refusal::NotMember)?;
        let definition = catalog.get(held.item).ok_or(Refusal::MissingContent)?;
        let item = held.display_name(definition, false);
        let name = profile.name.clone();
        let eligible = crate::game::camp::can_act(vitals);
        let pool_first = !eligible || self.battle.is_some();
        let result = self.party.give_held(eligible.then_some(&mut *carried), pool_first, catalog);
        Ok(result.map(|given| match given {
            Given::Member => ItemNews::Takes { member: Some(name), item },
            Given::Pool => ItemNews::Takes { member: None, item },
            Given::NoRoom => ItemNews::NoRoom,
        }))
    }

    /// Return the hand to the reviewed member, falling back to the pool. A
    /// departed member cannot receive it; overflow remains owned by the hand.
    pub fn put_away(&mut self, member: Entity) -> Result<(), Refusal> {
        if self.party.hand.held.is_none() {
            return Ok(());
        }
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        if self.roster.0.contains(&member)
            && let Ok((_, _, mut carried)) = self.members.get_mut(member)
        {
            self.party.put_away_held(&mut carried, catalog);
        } else {
            self.party.give_held(None, false, catalog);
        }
        Ok(())
    }

    /// Resolve a pool row immediately at the UI boundary. Delayed requests use
    /// `take` or `deposit` and retain identities, never packed row indices.
    pub fn click_pool(&mut self, row: usize) -> Result<(), Refusal> {
        let anchor = self.party.pool.stacks.get(row).map(|s| s.id);
        if self.party.hand.held.is_some() {
            self.deposit(anchor.map_or(PoolPosition::End, PoolPosition::Before)).map(|_| ())
        } else {
            self.take(ItemRef { owner: ItemOwner::Pool, id: anchor.ok_or(Refusal::Nothing)? })
        }
    }
    pub fn deposit(&mut self, position: PoolPosition) -> Result<u32, Refusal> {
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        self.party.deposit_hand(position, catalog)
    }
    pub fn interact_slot(&mut self, destination: MemberSlot) -> Result<(), Refusal> {
        if !self.roster.0.contains(&destination.member) {
            return Err(Refusal::NotMember);
        }
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        let (character, vitals, mut carried) =
            self.members.get_mut(destination.member).map_err(|_| Refusal::NotMember)?;
        self.party.interact_slot(character, vitals, &mut carried, destination.slot, catalog)
    }
    pub fn switch_weapon_sets(&mut self, member: Entity) -> Result<(), Refusal> {
        if !self.roster.0.contains(&member) {
            return Err(Refusal::NotMember);
        }
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        let (_, vitals, mut carried) =
            self.members.get_mut(member).map_err(|_| Refusal::NotMember)?;
        carried.switch_weapon_sets(catalog, vitals.is_dead())
    }
    pub fn take(&mut self, source: ItemRef) -> Result<(), Refusal> {
        if self.party.hand.held.is_some() {
            return Err(Refusal::HandFull);
        }
        match source.owner {
            ItemOwner::Pool => self.party.take_from_pool(source.id),
            ItemOwner::Member(member) => {
                if !self.roster.0.contains(&member) {
                    return Err(Refusal::NotMember);
                }
                let (_, _, carried) = self.members.get(member).map_err(|_| Refusal::NotMember)?;
                let slot = carried.find(source.id).ok_or(Refusal::Nothing)?;
                self.interact_slot(MemberSlot { member, slot })
            }
        }
    }
    pub fn take_units(
        &mut self,
        source: ItemRef,
        amount: u16,
        ids: &mut ItemIds,
    ) -> Result<(), Refusal> {
        let catalog = self
            .data
            .as_ref()
            .and_then(|d| self.catalogs.get(&d.items))
            .ok_or(Refusal::MissingContent)?;
        let PartyInventory { hand, pool, .. } = &mut *self.party;
        match source.owner {
            ItemOwner::Pool => {
                let index =
                    pool.stacks.iter().position(|s| s.id == source.id).ok_or(Refusal::Nothing)?;
                let stack = &pool.stacks[index];
                let definition = catalog.get(stack.item).ok_or(Refusal::MissingContent)?;
                hand.check_take_units(stack, amount, definition)?;
                let stack = pool.stacks.remove(index);
                if let Some(rest) = hand.take_units(stack, amount, ids) {
                    pool.stacks.insert(index, rest);
                }
            }
            ItemOwner::Member(member) => {
                if !self.roster.0.contains(&member) {
                    return Err(Refusal::NotMember);
                }
                let (_, vitals, mut carried) =
                    self.members.get_mut(member).map_err(|_| Refusal::NotMember)?;
                let slot = carried.find(source.id).ok_or(Refusal::Nothing)?;
                let stack = carried.slot(slot).unwrap();
                let definition = catalog.get(stack.item).ok_or(Refusal::MissingContent)?;
                hand.check_take_units(stack, amount, definition)?;
                if stack.count() == amount {
                    carried.check_removal(slot, catalog, vitals.is_dead())?;
                }
                carried.offer_slot(slot, |stack| hand.take_units(stack, amount, ids));
            }
        }
        Ok(())
    }
}

/// Drain owned contents through the same pool-transfer rules for chests and
/// world bags. Partial stack merges are reported; every remainder stays owned
/// by the source, in its original order.
pub fn transfer_contents(
    contents: &mut Vec<ItemStack>,
    party: &mut PartyInventory,
    catalog: &ItemCatalog,
) -> Vec<(String, u32)> {
    let mut taken = Vec::new();
    let mut remaining = Vec::new();
    for item in contents.drain(..) {
        let before = item.units();
        let name = catalog.get(item.item).map(|d| item.display_name(d, false)).unwrap_or_default();
        let after = match party.pool.receive(item, catalog) {
            None => 0,
            Some(item) => {
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
