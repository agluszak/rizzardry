//! Rolling loot from tables (retail `GenerateItemsFromTable`).

use super::rng::GameRng;
use crate::content::items::{ItemCatalog, ItemId, LootTable};

/// Party levels and item values a level-scaled table draws from (Whizzardry
/// `g_item_level_scale_ranges`): from that party level, items worth at least
/// the first value and less than the second.
const LEVEL_SCALE: [(u32, u32, u32); 7] = [
    (1, 0, 500),
    (6, 50, 1000),
    (11, 100, 3000),
    (16, 300, 5000),
    (21, 600, 10000),
    (26, 800, 20000),
    (31, 1000, 1_000_000),
];

/// The items and gold a table yields. Items of weight 0 always drop; then
/// `item_count` draws pick a random remaining entry and keep it when a roll
/// below the total weight is at most its weight. As in retail, the total is
/// not reduced as entries are taken.
pub fn roll_loot(
    table: &LootTable,
    catalog: &ItemCatalog,
    party_level: u32,
    rng: &mut GameRng,
) -> (Vec<ItemId>, u32) {
    let wanted = rng.roll(table.item_count);
    let mut items: Vec<ItemId> =
        table.entries.iter().filter(|(_, w)| *w == 0).map(|(item, _)| *item).collect();
    let fits = |item: ItemId| {
        let value = catalog.get(item).map_or(0, |d| d.value);
        LEVEL_SCALE
            .iter()
            .any(|&(level, low, high)| level <= party_level && low <= value && value < high)
    };
    let mut candidates: Vec<(ItemId, u8)> = table
        .entries
        .iter()
        .filter(|(item, weight)| *weight != 0 && (!table.level_scaled || fits(*item)))
        .copied()
        .collect();
    if table.level_scaled && candidates.len() < wanted as usize {
        candidates = table.entries.clone();
    }
    let total: u32 = candidates.iter().map(|(_, w)| u32::from(*w)).sum();
    let mut taken = 0;
    while taken < wanted && !candidates.is_empty() {
        let pick = rng.below(candidates.len() as u32) as usize;
        if rng.below(total) <= u32::from(candidates[pick].1) {
            items.push(candidates.remove(pick).0);
            taken += 1;
        }
    }
    (items, rng.roll(table.gold))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{Dice, ItemCatalog};

    #[test]
    fn fixed_items_always_drop_and_draws_take_distinct_entries() {
        let table = LootTable {
            name: "Test".into(),
            entries: vec![(ItemId(9), 0), (ItemId(1), 5), (ItemId(2), 5), (ItemId(3), 5)],
            level_scaled: false,
            item_count: Dice { count: 0, sides: 0, base: 2 },
            gold: Dice { count: 1, sides: 10, base: 5 },
        };
        let mut rng = GameRng::seeded(11);
        for _ in 0..50 {
            let (items, gold) = roll_loot(&table, &ItemCatalog::default(), 1, &mut rng);
            assert_eq!(items[0], ItemId(9));
            assert_eq!(items.len(), 3);
            assert_ne!(items[1], items[2]);
            assert!((6..=15).contains(&gold));
        }
    }
}
