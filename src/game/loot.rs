//! Rolling loot from tables (retail `GenerateItemsFromTable`).

use super::rng::GameRng;
use crate::content::items::{ItemCatalog, ItemId, LootTable, LootTables};
use crate::content::species::{Drop, Species};

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

/// The items and gold a table yields (a container's contents): its item
/// count rolled, then [`table_items`], then its gold.
pub fn roll_loot(
    table: &LootTable,
    catalog: &ItemCatalog,
    party_level: u32,
    rng: &mut GameRng,
) -> (Vec<ItemId>, u32) {
    let wanted = rng.roll(table.item_count);
    (table_items(table, catalog, party_level, wanted, rng), rng.roll(table.gold))
}

/// Up to `wanted` items from a table (retail `GenerateItemsFromTable`).
/// Items of weight 0 always drop; then draws pick a random remaining entry
/// and keep it when a roll below the total weight is at most its weight. As
/// in retail, the total is not reduced as entries are taken.
pub fn table_items(
    table: &LootTable,
    catalog: &ItemCatalog,
    party_level: u32,
    wanted: u32,
    rng: &mut GameRng,
) -> Vec<ItemId> {
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
    items
}

/// What a slain monster leaves (retail `DropMonsterLoot`, for monsters that
/// are not NPCs): each treasure slot drops when `Random(100)` is at most its
/// chance, as many times as its dice roll; a table slot draws up to that
/// many items from its table each time. Then its gold.
pub fn monster_loot(
    species: &Species,
    tables: &LootTables,
    catalog: &ItemCatalog,
    party_level: u32,
    rng: &mut GameRng,
) -> (Vec<ItemId>, u32) {
    let mut items = Vec::new();
    for slot in &species.treasure {
        if rng.below(100) > u32::from(slot.chance) {
            continue;
        }
        let rolls = rng.roll(slot.dice);
        for _ in 0..rolls {
            match slot.drop {
                Drop::Item(item) => items.push(item),
                Drop::Table(table) => {
                    if let Some(table) = tables.0.get(usize::from(table)) {
                        items.extend(table_items(table, catalog, party_level, rolls, rng));
                    }
                }
                Drop::Nothing => {}
            }
        }
    }
    (items, rng.roll(species.gold))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::{Dice, ItemCatalog};
    use crate::content::species::Treasure;
    use bevy::prelude::default;

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

    #[test]
    fn a_monster_drops_its_sure_treasure_and_gold() {
        let mut crab = Species { gold: Dice { count: 0, sides: 0, base: 7 }, ..default() };
        crab.treasure = vec![
            // Always, twice.
            Treasure {
                drop: Drop::Item(ItemId(4)),
                chance: 100,
                dice: Dice { base: 2, ..default() },
            },
            // One in a hundred (Random(100) at most 0), and it drops nothing.
            Treasure { drop: Drop::Nothing, chance: 0, dice: Dice { base: 1, ..default() } },
        ];
        let mut rng = GameRng::seeded(3);
        let tables = LootTables::default();
        let (items, gold) = monster_loot(&crab, &tables, &ItemCatalog::default(), 1, &mut rng);
        assert_eq!(items, [ItemId(4), ItemId(4)]);
        assert_eq!(gold, 7);
    }
}
