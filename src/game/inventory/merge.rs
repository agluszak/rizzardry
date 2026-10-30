//! Making an item from the held one and another (retail `MergeItems`).

use super::*;
use crate::content::items::Recipe;

/// What merging came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merged {
    /// The item both are an ingredient of was made.
    Made(ItemId),
    /// Held units joined a stack of the same item.
    Stacked,
    /// Nothing changed: a full stack.
    Unchanged,
    /// "Items must be identified first."
    Unidentified,
    /// No item is made from these two.
    NoRecipe,
    /// The member lacks the skill making it takes.
    Unskilled,
}

/// The first item made from these two, in item order (retail scans every
/// record and takes the first whose ingredients match in either order).
pub fn made_from(catalog: &ItemCatalog, a: ItemId, b: ItemId) -> Option<(&ItemDefinition, Recipe)> {
    catalog.0.iter().find_map(|definition| {
        let recipe = definition.recipe?;
        (recipe.ingredients == [a, b] || recipe.ingredients == [b, a])
            .then_some((definition, recipe))
    })
}

impl PartyInventory {
    /// Merge the held item with `target` for `character` (retail
    /// `MergeItems`). Both must be identified. When an item is made from the
    /// two and the member has the skill it takes (practising it), both give
    /// up as many units as the smaller stack holds (one of anything else),
    /// and the made item takes the place of whichever is used up. Otherwise
    /// two of one item stack together.
    ///
    /// Deviations, recorded: retail puts the made item into an equipment
    /// slot whatever it is, and loses it when the used-up ingredient was the
    /// pool's last row; here it goes to the hand when the slot cannot hold
    /// it (the merge is refused, changing nothing, when the hand stays full),
    /// and it takes the row's place in the pool.
    pub fn merge(
        &mut self,
        character: &mut Character,
        carried: &mut Carried,
        target: ItemAt,
        catalog: &ItemCatalog,
        (ids, rng): (&mut ItemIds, &mut GameRng),
    ) -> Result<Merged, Refusal> {
        let held = self.hand.held.as_ref().ok_or(Refusal::Nothing)?;
        let row = match target {
            ItemAt::Hand => return Err(Refusal::Nothing),
            ItemAt::Pool(id) => {
                Some(self.pool.stacks.iter().position(|s| s.id == id).ok_or(Refusal::Nothing)?)
            }
            ItemAt::Slot(_) => None,
        };
        let stack = match (target, row) {
            (_, Some(row)) => &self.pool.stacks[row],
            (ItemAt::Slot(slot), None) => carried.slot(slot).ok_or(Refusal::Nothing)?,
            _ => unreachable!("the pool has its row"),
        };
        if !held.identified || !stack.identified {
            return Ok(Merged::Unidentified);
        }
        let held_definition = &catalog[held.item];
        let definition = &catalog[stack.item];
        let made = made_from(catalog, held.item, stack.item);
        let skilled = made.is_some_and(|(_, recipe)| {
            recipe.skill.is_none_or(|(skill, level)| character.skill(skill) >= u32::from(level))
        });
        let Some((product, recipe)) = made.filter(|_| skilled) else {
            if held.item == stack.item {
                return Ok(self.stack_onto(carried, target, row, held_definition));
            }
            return Ok(if made.is_some() { Merged::Unskilled } else { Merged::NoRecipe });
        };
        // Two stacks give up the smaller's units, which then holds the made
        // item; anything else gives up one, and the made item goes where the
        // non-stack was (the hand when both are).
        let stacks = (held_definition.quantity, definition.quantity);
        let (units, into_hand) = match stacks {
            (QuantityKind::Stack, QuantityKind::Stack) => {
                (held.count().min(stack.count()), stack.count() > held.count())
            }
            (QuantityKind::Stack, _) => (1, false),
            _ => (1, true),
        };
        let held_survives = stacks.0 == QuantityKind::Stack && held.count() > units;
        let into_hand = into_hand
            || match target {
                ItemAt::Slot(Slot::Equipment(slot)) => {
                    let fits = character.can_use(product)
                        && allowed_slots(product, carried, catalog, &[slot]).contains(&slot);
                    if !fits && held_survives {
                        return Err(Refusal::DoesNotFit);
                    }
                    !fits
                }
                _ => false,
            };
        if let Some((skill, level)) = recipe.skill {
            character.practise(skill, u32::from(level / 10), rng);
        }
        let spend = |stack: ItemStack, quantity: QuantityKind| match quantity {
            QuantityKind::Stack => stack.remove_units(units),
            _ => None,
        };
        let held = self.hand.held.take().unwrap();
        self.hand.held = spend(held, stacks.0);
        match (target, row) {
            (_, Some(row)) => {
                let stack = self.pool.stacks.remove(row);
                if let Some(rest) = spend(stack, stacks.1) {
                    self.pool.stacks.insert(row, rest);
                }
            }
            (ItemAt::Slot(slot), None) => carried.offer_slot(slot, |s| spend(s, stacks.1)),
            _ => unreachable!("the pool has its row"),
        }
        let mut made = make_item(product, ids, rng, false, false);
        if product.quantity == QuantityKind::Stack {
            made.count = NonZeroU16::new(units).unwrap();
        }
        let id = made.item;
        match (target, row) {
            _ if into_hand => self.hand.held = Some(made),
            (_, Some(row)) => self.pool.stacks.insert(row, made),
            (ItemAt::Slot(slot), None) => carried.put(slot, Some(made)),
            _ => unreachable!("the pool has its row"),
        }
        Ok(Merged::Made(id))
    }

    /// Stack the held units onto the target (retail `MergeItemStacks`).
    fn stack_onto(
        &mut self,
        carried: &mut Carried,
        target: ItemAt,
        row: Option<usize>,
        definition: &ItemDefinition,
    ) -> Merged {
        let stack = match (target, row) {
            (_, Some(row)) => &mut self.pool.stacks[row],
            (ItemAt::Slot(slot), None) => carried.stack_mut(slot).unwrap(),
            _ => unreachable!("the pool has its row"),
        };
        match self.hand.offer(|held| stack.absorb(held, definition)) {
            0 => Merged::Unchanged,
            _ => Merged::Stacked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::{Gender, Profession, Race};
    use enumset::EnumSet;

    /// Powder and a reagent (stacks), a flask (one thing), a draught made
    /// from powder and reagent at Alchemy 10, and a bomb from flask and
    /// powder with no skill.
    fn catalog() -> ItemCatalog {
        let item = |id: u16, quantity| ItemDefinition {
            id: ItemId(id),
            name: format!("item {id}"),
            quantity,
            stack_limit: 20,
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            ..default()
        };
        let mut items: Vec<_> = [
            QuantityKind::Stack,
            QuantityKind::Stack,
            QuantityKind::None,
            QuantityKind::Stack,
            QuantityKind::None,
        ]
        .into_iter()
        .enumerate()
        .map(|(id, quantity)| item(id as u16, quantity))
        .collect();
        items[3].recipe =
            Some(Recipe { ingredients: [ItemId(0), ItemId(1)], skill: Some((Skill::Alchemy, 10)) });
        items[4].recipe = Some(Recipe { ingredients: [ItemId(2), ItemId(0)], skill: None });
        ItemCatalog(items)
    }

    /// A Fighter with Alchemy points, which no profession bonus raises.
    fn skilled(points: u8) -> Character {
        let mut character = Character::new("A", Race::Human, Gender::Female, Profession::Fighter);
        character.skill_points[Skill::Alchemy] = points;
        character
    }

    struct Bench {
        catalog: ItemCatalog,
        ids: ItemIds,
        rng: GameRng,
        party: PartyInventory,
        carried: Carried,
    }

    impl Bench {
        fn new() -> Self {
            Bench {
                catalog: catalog(),
                ids: ItemIds::default(),
                rng: GameRng::seeded(1),
                party: default(),
                carried: default(),
            }
        }
        fn stack(&mut self, item: usize, count: u16) -> ItemStack {
            ItemStack::new(&self.catalog.0[item], &mut self.ids, count, true)
        }
        fn merge(&mut self, character: &mut Character, target: ItemAt) -> Result<Merged, Refusal> {
            self.party.merge(
                character,
                &mut self.carried,
                target,
                &self.catalog,
                (&mut self.ids, &mut self.rng),
            )
        }
        fn held(&self) -> Option<(ItemId, u16)> {
            self.party.hand.held.as_ref().map(|s| (s.item, s.count()))
        }
        fn slot(&self, i: usize) -> Option<(ItemId, u16)> {
            self.carried.backpack[i].as_ref().map(|s| (s.item, s.count()))
        }
    }

    /// Two stacks make as many as the smaller holds, which the made item
    /// replaces; the larger keeps the rest.
    #[test]
    fn stacks_make_as_many_as_the_smaller_holds_in_its_place() {
        let mut bench = Bench::new();
        let mut character = skilled(20);
        bench.party.hand.held = Some(bench.stack(0, 5));
        bench.carried.backpack[0] = Some(bench.stack(1, 3));
        let backpack = ItemAt::Slot(Slot::Backpack(0));
        assert_eq!(bench.merge(&mut character, backpack), Ok(Merged::Made(ItemId(3))));
        assert_eq!(bench.slot(0), Some((ItemId(3), 3)));
        assert_eq!(bench.held(), Some((ItemId(0), 2)));
        // The other way round, the hand holds the smaller stack.
        bench.carried.backpack[0] = Some(bench.stack(1, 6));
        assert_eq!(bench.merge(&mut character, backpack), Ok(Merged::Made(ItemId(3))));
        assert_eq!(bench.held(), Some((ItemId(3), 2)));
        assert_eq!(bench.slot(0), Some((ItemId(1), 4)));
    }

    /// One thing and a stack: one unit goes, and the made item takes the
    /// thing's place, in a pool row as anywhere.
    #[test]
    fn a_single_ingredient_gives_its_place_to_the_made_item() {
        let mut bench = Bench::new();
        let mut character = skilled(0);
        bench.party.hand.held = Some(bench.stack(2, 1));
        let powder = bench.stack(0, 4);
        let row = powder.id;
        bench.party.pool.stacks = vec![bench.stack(1, 1), powder];
        assert_eq!(bench.merge(&mut character, ItemAt::Pool(row)), Ok(Merged::Made(ItemId(4))));
        assert_eq!(bench.held().map(|(item, _)| item), Some(ItemId(4)));
        assert_eq!(bench.party.pool.stacks[1].count(), 3);
        // Powder held over a flask in the last pool row: the bomb takes it.
        bench.party.hand.held = Some(bench.stack(0, 3));
        let flask = bench.stack(2, 1);
        let row = flask.id;
        bench.party.pool.stacks.push(flask);
        assert_eq!(bench.merge(&mut character, ItemAt::Pool(row)), Ok(Merged::Made(ItemId(4))));
        assert_eq!(bench.party.pool.stacks.last().map(|s| s.item), Some(ItemId(4)));
        assert_eq!(bench.held(), Some((ItemId(0), 2)));
    }

    /// Without both identified, the skill, or a recipe nothing changes;
    /// two of one item still stack.
    #[test]
    fn refusals_change_nothing_and_like_items_stack() {
        let mut bench = Bench::new();
        let backpack = ItemAt::Slot(Slot::Backpack(0));
        bench.party.hand.held = Some(bench.stack(0, 5));
        bench.carried.backpack[0] = Some(bench.stack(1, 3));
        bench.carried.backpack[0].as_mut().unwrap().identified = false;
        let unchanged = |bench: &Bench, party: &PartyInventory, carried: &Carried| {
            bench.party == *party && bench.carried == *carried
        };
        let (party, carried) = (bench.party.clone(), bench.carried.clone());
        assert_eq!(bench.merge(&mut skilled(20), backpack), Ok(Merged::Unidentified));
        assert!(unchanged(&bench, &party, &carried));
        bench.carried.backpack[0].as_mut().unwrap().identified = true;
        let (party, carried) = (bench.party.clone(), bench.carried.clone());
        assert_eq!(bench.merge(&mut skilled(5), backpack), Ok(Merged::Unskilled));
        assert!(unchanged(&bench, &party, &carried));
        bench.carried.backpack[0] = Some(bench.stack(2, 1));
        bench.party.hand.held = Some(bench.stack(1, 5));
        let (party, carried) = (bench.party.clone(), bench.carried.clone());
        assert_eq!(bench.merge(&mut skilled(20), backpack), Ok(Merged::NoRecipe));
        assert!(unchanged(&bench, &party, &carried));
        bench.carried.backpack[0] = Some(bench.stack(1, 2));
        assert_eq!(bench.merge(&mut skilled(20), backpack), Ok(Merged::Stacked));
        assert_eq!((bench.slot(0), bench.held()), (Some((ItemId(1), 7)), None));
    }
}
