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
