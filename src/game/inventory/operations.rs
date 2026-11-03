//! The meaningful inventory activation used by UI cells and headless callers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::*;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;

/// Explicitly finish handling the held stack, preferring this member and then
/// the pool. A view disappearing is not this request. Missing content and full
/// containers leave the unplaced stack in the hand, as `put_away` specifies.
#[derive(Event, Clone, Copy, Debug)]
pub struct PutAwayHeld {
    pub member: Entity,
}

/// Return the hand to the reviewed member, falling back to the pool;
/// overflow remains owned by the hand. Someone who has left the party
/// since the hand was filled takes nothing; the pool does.
pub(crate) fn put_away_held(request: On<PutAwayHeld>, mut inventory: InventoryOperations) {
    let InventoryOperations { party, members, content, .. } = &mut inventory;
    let mut carried = members.get_mut(request.member).ok().map(|(_, _, carried)| carried);
    party.give_held(carried.as_deref_mut(), false, &content.items);
}

/// Party inventory operations act for current party members only: a
/// recruited NPC keeps its character and pack after it is dismissed, so
/// having them does not make it a member.
#[derive(SystemParam)]
pub struct InventoryOperations<'w, 's> {
    party: Single<'w, 's, &'static mut PartyInventory>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<
        'w,
        's,
        (&'static mut Character, &'static Vitals, &'static mut Carried),
        With<PartyMember>,
    >,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    battle: Option<Single<'w, 's, &'static crate::game::combat::Battle>>,
    news: MessageWriter<'w, ItemNews>,
}

impl InventoryOperations<'_, '_> {
    /// Give the held item to an identified actor, telling who took it.
    /// Eligibility and battle pool precedence are rules here; the clicked
    /// portrait only supplies its target. False when nothing was held.
    pub fn give_to(&mut self, member: Entity) -> bool {
        let Some(held) = self.party.hand.held.as_ref() else { return false };
        let catalog = &self.content.items;
        let Ok((profile, vitals, mut carried)) = self.members.get_mut(member) else {
            return false;
        };
        let definition = &catalog[held.item];
        let item = held.display_name(definition, false);
        let name = profile.name.clone();
        let eligible = vitals.can_act();
        let pool_first = !eligible || self.battle.is_some();
        let Some(given) =
            self.party.give_held(eligible.then_some(&mut *carried), pool_first, catalog)
        else {
            return false;
        };
        self.news.write(match given {
            Given::Member => ItemNews::Takes { member: Some(name), item },
            Given::Pool => ItemNews::Takes { member: None, item },
            Given::NoRoom => ItemNews::NoRoom,
        });
        true
    }

    /// Resolve a pool row immediately at the UI boundary. Delayed requests use
    /// `take` or `deposit` and retain identities, never packed row indices.
    pub fn click_pool(&mut self, row: usize) -> Result<(), Refusal> {
        let anchor = self.party.pool.stacks.get(row).map(|s| s.id);
        if self.party.hand.held.is_some() {
            // Ammunition over its gun loads it, however many charges that takes.
            let catalog = &self.content.items;
            if let Some(anchor) = anchor
                && self.party.load_row(anchor, catalog).is_some()
            {
                return Ok(());
            }
            let position = anchor.map_or(PoolPosition::End, PoolPosition::Before);
            match self.party.deposit_hand(position, &self.content.items)? {
                0 => Err(Refusal::NoRoom),
                _ => Ok(()),
            }
        } else {
            self.party.take_from_pool(anchor.ok_or(Refusal::Nothing)?)
        }
    }
    pub fn interact_slot(&mut self, destination: MemberSlot) -> Result<(), Refusal> {
        let catalog = &self.content.items;
        let (character, vitals, mut carried) =
            self.members.get_mut(destination.member).map_err(|_| Refusal::Nothing)?;
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
        let catalog = &self.content.items;
        let (mut character, _, mut carried) =
            self.members.get_mut(member).map_err(|_| Refusal::Nothing)?;
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
        let catalog = &self.content.items;
        let stack = match item {
            ItemAt::Hand => self.party.hand.held.as_ref(),
            ItemAt::Pool(id) => self.party.pool.stacks.iter().find(|s| s.id == id),
            ItemAt::Slot(slot) => {
                self.members.get(member).map_err(|_| Refusal::Nothing)?.2.slot(slot)
            }
        };
        let stack = stack.ok_or(Refusal::Nothing)?;
        if stack.identified {
            return Ok(false);
        }
        let difficulty = catalog[stack.item].identify_difficulty;
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
                let (_, _, carried) = self.members.get_mut(member).map_err(|_| Refusal::Nothing)?;
                carried.into_inner().stack_mut(slot).ok_or(Refusal::Nothing)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Gender, Profession, Race};
    use bevy::ecs::system::RunSystemOnce;

    /// A dismissed recruit keeps its character and pack; party inventory
    /// operations still refuse it rather than reaching into a non-member.
    #[test]
    fn party_operations_refuse_someone_who_is_not_a_member() {
        let mut world = World::new();
        world.init_resource::<Messages<ItemNews>>();
        world.insert_resource(crate::game::data::Catalogs::default());
        let character = Character::new("Left", Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&character);
        let member = world.spawn((character, vitals, Carried::default())).id();
        let staying = world.spawn(Carried::default()).id();
        let party = crate::game::party::roster::test_party(&mut world, &[staying, member]);
        world.entity_mut(member).remove::<crate::game::party::roster::PartyMember>();
        world.get_mut::<PartyInventory>(party).unwrap().gold = 7;

        let slot = MemberSlot { member, slot: Slot::Backpack(0) };
        let result = world
            .run_system_once(move |mut ops: InventoryOperations| {
                (ops.interact_slot(slot), ops.give_to(member))
            })
            .unwrap();
        assert_eq!(result, (Err(Refusal::Nothing), false));
    }
}
