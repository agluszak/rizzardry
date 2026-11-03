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
/// overflow remains owned by the hand.
pub(crate) fn put_away_held(request: On<PutAwayHeld>, mut inventory: InventoryOperations) {
    let InventoryOperations { party, members, content, .. } = &mut inventory;
    match members.get_mut(request.member) {
        Ok((_, _, mut carried)) => party.give_held(Some(&mut carried), false, &content.items),
        Err(_) => party.give_held(None, true, &content.items),
    };
}

#[derive(SystemParam)]
pub struct InventoryOperations<'w, 's> {
    party: Single<'w, 's, &'static mut PartyInventory>,
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Members<'w, 's>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    battle: Option<Single<'w, 's, &'static crate::game::combat::Battle>>,
    news: MessageWriter<'w, ItemNews>,
}

impl InventoryOperations<'_, '_> {
    /// Give the held item to an identified actor, telling who took it.
    /// Eligibility and battle pool precedence are rules here; the clicked
    /// portrait only supplies its target. A former or missing member cannot
    /// receive party items. False when the request is no longer applicable.
    pub fn give_to(&mut self, member: Entity) -> bool {
        let Some(held) = self.party.hand.held.as_ref() else { return false };
        let catalog = &self.content.items;
        let Ok((profile, vitals, mut carried)) = self.members.get_mut(member) else { return false };
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

type Members<'w, 's> = Query<
    'w,
    's,
    (&'static mut Character, &'static Vitals, &'static mut Carried),
    With<PartyMember>,
>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::data::Catalogs;
    use crate::game::party::roster;

    fn carrying() -> (App, Entity, Entity, StackId) {
        let mut app = App::new();
        app.add_message::<ItemNews>()
            .init_resource::<ItemIds>()
            .init_resource::<GameRng>()
            .add_observer(put_away_held);
        let definition = ItemDefinition::default();
        let held = ItemStack::new(&definition, &mut ItemIds::default(), 1, true);
        let id = held.id;
        app.insert_resource(Catalogs { items: ItemCatalog(vec![definition]), ..default() });
        let world = app.world_mut();
        let profile = Character::new("Companion", Race::Human, Gender::Male, Profession::Fighter);
        let member = world.spawn((Vitals::new(&profile), profile, Carried::default())).id();
        let leader = world.spawn_empty().id();
        let party = roster::test_party(world, &[leader, member]);
        world.get_mut::<PartyInventory>(party).unwrap().hand.held = Some(held);
        (app, party, member, id)
    }

    fn give(In(member): In<Entity>, mut inventory: InventoryOperations) -> bool {
        inventory.give_to(member)
    }

    #[test]
    fn leaving_the_party_revokes_inventory_access_until_rejoining() {
        let (mut app, party, member, held) = carrying();
        let world = app.world_mut();
        world.entity_mut(member).remove::<PartyMember>();
        assert!(!world.run_system_cached_with(give, member).unwrap());
        world
            .run_system_cached_with(
                |In(member): In<Entity>,
                 mut inventory: InventoryOperations,
                 mut ids: ResMut<ItemIds>,
                 mut rng: ResMut<GameRng>| {
                    let slot = Slot::Backpack(0);
                    assert_eq!(
                        inventory.interact_slot(MemberSlot { member, slot }),
                        Err(Refusal::Nothing)
                    );
                    assert_eq!(
                        inventory.merge(member, ItemAt::Slot(slot), (&mut ids, &mut rng)),
                        Err(Refusal::Nothing)
                    );
                    assert_eq!(
                        inventory.identify(member, ItemAt::Slot(slot), &mut rng),
                        Err(Refusal::Nothing)
                    );
                },
                member,
            )
            .unwrap();
        assert_eq!(
            world.get::<PartyInventory>(party).unwrap().hand.held.as_ref().unwrap().id,
            held
        );
        assert_eq!(world.get::<Carried>(member).unwrap().stacks().count(), 0);

        world.entity_mut(member).insert(PartyMember(party));
        assert!(world.run_system_cached_with(give, member).unwrap());
        assert!(world.get::<PartyInventory>(party).unwrap().hand.held.is_none());
        assert_eq!(
            world.get::<Carried>(member).unwrap().stacks().map(|s| s.id).collect::<Vec<_>>(),
            [held]
        );
    }

    #[test]
    fn putting_away_an_item_uses_the_pool_when_its_recipient_has_left_or_gone() {
        for despawn in [false, true] {
            let (mut app, party, member, held) = carrying();
            let world = app.world_mut();
            world.entity_mut(member).remove::<PartyMember>();
            if despawn {
                world.despawn(member);
            }
            world.trigger(PutAwayHeld { member });
            world.flush();
            let inventory = world.get::<PartyInventory>(party).unwrap();
            assert!(inventory.hand.held.is_none());
            assert_eq!(inventory.pool.stacks.iter().map(|s| s.id).collect::<Vec<_>>(), [held]);
        }
    }
}
