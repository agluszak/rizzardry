//! The meaningful inventory activation used by UI cells and headless callers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::*;
use crate::game::data::Content;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;

#[derive(SystemParam)]
pub struct InventoryOperations<'w, 's> {
    party: Single<'w, 's, &'static mut PartyInventory>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Members<'w, 's>,
    content: Content<'w>,
    battle: Option<Single<'w, 's, &'static crate::game::combat::Battle>>,
    news: MessageWriter<'w, ItemNews>,
}

impl InventoryOperations<'_, '_> {
    /// Give the held item to an identified actor, telling who took it.
    /// Eligibility and battle pool precedence are rules here; the clicked
    /// portrait only supplies its target. False when nothing was held.
    pub fn give_to(&mut self, member: Entity) -> Result<bool, Refusal> {
        let Some(held) = self.party.hand.held.as_ref() else { return Ok(false) };
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let (profile, vitals, mut carried) = self::member(&self.roster, &mut self.members, member)?;
        let definition = catalog.get(held.item).ok_or(Refusal::MissingContent)?;
        let item = held.display_name(definition, false);
        let name = profile.name.clone();
        let eligible = vitals.can_act();
        let pool_first = !eligible || self.battle.is_some();
        let Some(given) =
            self.party.give_held(eligible.then_some(&mut *carried), pool_first, catalog)
        else {
            return Ok(false);
        };
        self.news.write(match given {
            Given::Member => ItemNews::Takes { member: Some(name), item },
            Given::Pool => ItemNews::Takes { member: None, item },
            Given::NoRoom => ItemNews::NoRoom,
        });
        Ok(true)
    }

    /// Return the hand to the reviewed member, falling back to the pool. A
    /// departed member cannot receive it; overflow remains owned by the hand.
    pub fn put_away(&mut self, member: Entity) -> Result<(), Refusal> {
        if self.party.hand.held.is_none() {
            return Ok(());
        }
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let mut carried = self::member(&self.roster, &mut self.members, member).ok();
        self.party.give_held(carried.as_mut().map(|(_, _, c)| &mut **c), false, catalog);
        Ok(())
    }

    /// Resolve a pool row immediately at the UI boundary. Delayed requests use
    /// `take` or `deposit` and retain identities, never packed row indices.
    pub fn click_pool(&mut self, row: usize) -> Result<(), Refusal> {
        let anchor = self.party.pool.stacks.get(row).map(|s| s.id);
        if self.party.hand.held.is_some() {
            // Ammunition over its gun loads it, however many charges that takes.
            let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
            if let Some(anchor) = anchor
                && self.party.load_row(anchor, catalog).is_some()
            {
                return Ok(());
            }
            match self.deposit(anchor.map_or(PoolPosition::End, PoolPosition::Before))? {
                0 => Err(Refusal::NoRoom),
                _ => Ok(()),
            }
        } else {
            self.take(ItemRef { owner: ItemOwner::Pool, id: anchor.ok_or(Refusal::Nothing)? })
        }
    }
    pub fn deposit(&mut self, position: PoolPosition) -> Result<u32, Refusal> {
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        self.party.deposit_hand(position, catalog)
    }
    pub fn interact_slot(&mut self, destination: MemberSlot) -> Result<(), Refusal> {
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let (character, vitals, mut carried) =
            member(&self.roster, &mut self.members, destination.member)?;
        self.party.interact_slot(&character, vitals, &mut carried, destination.slot, catalog)
    }

    /// Whether the hand holds an item.
    pub fn holds(&self) -> bool {
        self.party.hand.held.is_some()
    }

    /// The stack at a pool row, as the screen shows it.
    pub fn pool_row(&self, row: usize) -> Option<StackId> {
        self.party.pool.stacks.get(row).map(|stack| stack.id)
    }

    /// Merge the held item with one of the member's or a pool row for them
    /// (retail `MergeItems`), telling the party why when nothing is made.
    pub fn merge(
        &mut self,
        member: Entity,
        target: ItemAt,
        (ids, rng): (&mut ItemIds, &mut GameRng),
    ) -> Result<merge::Merged, Refusal> {
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let (mut character, _, mut carried) =
            self::member(&self.roster, &mut self.members, member)?;
        let merged = self.party.merge(&mut character, &mut carried, target, catalog, (ids, rng))?;
        match merged {
            merge::Merged::Unidentified => {
                self.news.write(ItemNews::Unidentified);
            }
            merge::Merged::NoRecipe => {
                self.news.write(ItemNews::CannotMerge);
            }
            merge::Merged::Unskilled => {
                self.news.write(ItemNews::Unskilled { member: character.name.clone() });
            }
            _ => {}
        }
        Ok(merged)
    }

    /// The party tries to identify an item within the member's reach
    /// (retail `PartyAttemptsToIdentifyItem`): everyone alive and sane tries
    /// in turn, by their Artifacts skill. Returns whether anyone did.
    pub fn identify(
        &mut self,
        member: Entity,
        item: ItemAt,
        rng: &mut GameRng,
    ) -> Result<bool, Refusal> {
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let stack = match item {
            ItemAt::Hand => self.party.hand.held.as_ref(),
            ItemAt::Pool(id) => self.party.pool.stacks.iter().find(|s| s.id == id),
            ItemAt::Slot(slot) => {
                if !self.roster.contains(&member) {
                    return Err(Refusal::NotMember);
                }
                self.members.get(member).ok().and_then(|(_, _, carried)| carried.slot(slot))
            }
        };
        let stack = stack.ok_or(Refusal::Nothing)?;
        if stack.identified {
            return Ok(false);
        }
        let difficulty =
            catalog.get(stack.item).ok_or(Refusal::MissingContent)?.identify_difficulty;
        let (mut identified, mut binding) = (false, false);
        let attempts = self.members.iter_many_unique_mut(self.roster.iter()).matched();
        for (mut character, vitals, _) in attempts {
            let Some(attempt) = super::identify_attempt(&character, vitals, difficulty) else {
                continue;
            };
            identified = true;
            binding |= attempt.binding;
            if attempt.practises {
                character.practise(crate::content::characters::Skill::Artifacts, 2, rng);
            }
        }
        if identified {
            let stack = self.stack_at(member, item)?;
            stack.identified = true;
            stack.spell_known = true;
            stack.curse_known |= binding;
        }
        Ok(identified)
    }

    /// An item within the member's reach.
    fn stack_at(&mut self, member: Entity, item: ItemAt) -> Result<&mut ItemStack, Refusal> {
        match item {
            ItemAt::Hand => self.party.hand.held.as_mut().ok_or(Refusal::Nothing),
            ItemAt::Pool(id) => {
                self.party.pool.stacks.iter_mut().find(|s| s.id == id).ok_or(Refusal::Nothing)
            }
            ItemAt::Slot(slot) => {
                let (_, _, carried) = self::member(&self.roster, &mut self.members, member)?;
                carried.into_inner().stack_mut(slot).ok_or(Refusal::Nothing)
            }
        }
    }

    pub fn switch_weapon_sets(&mut self, member: Entity) -> Result<(), Refusal> {
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let (_, vitals, mut carried) = self::member(&self.roster, &mut self.members, member)?;
        carried.switch_weapon_sets(catalog, vitals.is_dead())
    }
    pub fn take(&mut self, source: ItemRef) -> Result<(), Refusal> {
        if self.party.hand.held.is_some() {
            return Err(Refusal::HandFull);
        }
        match source.owner {
            ItemOwner::Pool => self.party.take_from_pool(source.id),
            ItemOwner::Member(member) => {
                let (_, _, carried) = self::member(&self.roster, &mut self.members, member)?;
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
        let catalog = self.content.items().ok_or(Refusal::MissingContent)?;
        let PartyInventory { hand, pool, .. } = &mut **self.party;
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
                let (_, vitals, mut carried) =
                    self::member(&self.roster, &mut self.members, member)?;
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

type Members<'w, 's> =
    Query<'w, 's, (&'static mut Character, &'static Vitals, &'static mut Carried)>;

/// A party member's inventory; anyone else is refused.
fn member<'a>(
    roster: &PartyRoster,
    members: &'a mut Members,
    member: Entity,
) -> Result<(Mut<'a, Character>, &'a Vitals, Mut<'a, Carried>), Refusal> {
    if !roster.contains(&member) {
        return Err(Refusal::NotMember);
    }
    members.get_mut(member).map_err(|_| Refusal::NotMember)
}
