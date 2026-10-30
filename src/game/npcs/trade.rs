//! Stock is merchandise, not the NPC's personal equipment. Commit transfers
//! and payment synchronously after validating the entire request.
use crate::content::{
    items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, QuantityKind},
    npcs::NpcDefinition,
};
use crate::game::time::DAY_MS;
use crate::game::{
    inventory::{ItemIds, ItemRef, ItemStack, PartyInventory},
    rng::GameRng,
};
use bevy::prelude::*;

/// The one restocked item retail holds back until the Templar fact is set
/// (`RestockNpcItems`).
pub(crate) const TEMPLAR_GATED: ItemId = ItemId(0x1fc);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockLotId(pub u64);
#[derive(Clone, Debug, PartialEq)]
pub struct StockLot {
    pub id: StockLotId,
    pub item: ItemId,
    pub units: u32,
    pub available_at: u64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MerchantStock {
    pub lots: Vec<StockLot>,
    pub restock_at: Option<u64>,
    pub maintenance_at: u64,
    next: u64,
    /// Trades made with the merchant: a quote confirms only the next one.
    pub trades: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeError {
    StaleSelection,
    Unavailable,
    InvalidQuantity,
    PriceChanged,
    Funds,
    Capacity,
    Rejected,
    Busy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeSource {
    Stock(StockLotId),
    Party(ItemRef),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeQuote {
    pub conversation: Entity,
    /// The merchant's [`MerchantStock::trades`] when quoted.
    pub trade: u64,
    pub source: TradeSource,
    pub quantity: u32,
    pub gold: u32,
}
fn tier(def: &ItemDefinition, level: u32) -> u32 {
    let base = (((level as i8) as i32 * 100 / 30) as i8) / 5;
    let lower = base.clamp(1, 16);
    let upper = base.wrapping_add(4).clamp(4, 20);
    let difficulty = def.identify_difficulty as i8;
    if difficulty > upper {
        0
    } else if difficulty < lower {
        1 + u32::from(difficulty.saturating_add(4) >= lower)
    } else {
        3
    }
}
fn jitter(n: u32, rng: &mut GameRng) -> u32 {
    match rng.below(3) {
        0 => n + n / 2,
        1 => n - n / 2,
        _ => n,
    }
}
impl MerchantStock {
    pub fn add(
        &mut self,
        item: ItemId,
        units: u32,
        catalog: &ItemCatalog,
        ammunition_bundle: bool,
    ) {
        let d = &catalog[item];
        if units == 0 {
            return;
        }
        if d.class == EquipClass::Ammunition && ammunition_bundle {
            for _ in 0..units {
                self.new_lot(d, 25)
            }
        } else if d.class != EquipClass::Ammunition
            && let Some(lot) = self.lots.iter_mut().find(|l| l.item == item && l.available_at == 0)
        {
            lot.units += units;
        } else {
            self.new_lot(d, units)
        }
    }
    fn new_lot(&mut self, d: &ItemDefinition, units: u32) {
        self.next += 1;
        self.lots.push(StockLot { id: StockLotId(self.next), item: d.id, units, available_at: 0 });
    }
    pub fn maintain(
        &mut self,
        def: &NpcDefinition,
        catalog: &ItemCatalog,
        level: u32,
        now: u64,
        templar: bool,
        rng: &mut GameRng,
    ) {
        let Some(restock_at) = self.restock_at else {
            for r in &def.stock {
                let chance = if r.persistent { 100 } else { tier(&catalog[r.item], level) * 25 };
                let n = (0..r.quantity).filter(|_| rng.below(100) < chance).count() as u32;
                self.add(r.item, n, catalog, true);
            }
            self.restock_at = Some(now);
            self.maintenance_at = now;
            return;
        };
        let daily = now.saturating_sub(self.maintenance_at) >= DAY_MS;
        if now.saturating_sub(restock_at) > DAY_MS / 2 {
            self.replenish(def, catalog, level, true, templar, rng);
            self.restock_at = Some(now);
        }
        if daily {
            for lot in &mut self.lots {
                let d = &catalog[lot.item];
                if lot.available_at != 0
                    || d.crucial
                    || d.merchant_cannot_sell
                    || def.stock.iter().any(|r| r.item == lot.item && r.persistent)
                {
                    continue;
                }
                // Retail consumes one of the non-stack counts even for a stack;
                // its Random(100)<100 still draws for each removed count.
                let count = if d.quantity == QuantityKind::Stack { 1 } else { lot.units };
                for _ in 0..count {
                    rng.below(100);
                }
                lot.units = lot.units.saturating_sub(count);
            }
            self.replenish(def, catalog, level, false, templar, rng);
            self.lots.retain(|l| l.units > 0);
            self.maintenance_at = now;
            self.restock_at = Some(now);
        }
        for lot in &mut self.lots {
            if lot.available_at != 0 && lot.available_at < now {
                lot.available_at = 0;
            }
        }
    }
    fn replenish(
        &mut self,
        def: &NpcDefinition,
        catalog: &ItemCatalog,
        level: u32,
        persistent: bool,
        templar: bool,
        rng: &mut GameRng,
    ) {
        for r in &def.stock {
            if r.persistent != persistent {
                continue;
            }
            // Retail RestockNpcItems gates only this non-persistent rule.
            // Initial population and persistent replenishment do not use the gate.
            if !persistent && r.item == TEMPLAR_GATED && !templar {
                continue;
            }
            let held = self.lots.iter().find(|l| l.item == r.item).map_or(0, |l| {
                if catalog[r.item].quantity == QuantityKind::Stack { 1 } else { l.units }
            });
            let configured = u32::from(r.quantity);
            if configured == 0 || held > configured / 2 {
                continue;
            }
            if !persistent && rng.below(100) >= tier(&catalog[r.item], level) * 25 {
                continue;
            }
            let n = jitter(configured - held, rng);
            self.add(r.item, n, catalog, true);
        }
    }
}
/// Retail price arithmetic: truncation per unit before multiplying ordinary
/// stacks, best party Communication, quarter price for unidentified sales.
pub fn price(
    def: &NpcDefinition,
    item: &ItemDefinition,
    quantity: u32,
    identified: bool,
    buy: bool,
    communication: u32,
) -> Result<u32, TradeError> {
    if quantity == 0 {
        return Err(TradeError::InvalidQuantity);
    }
    let mut adjustment = communication as f32 * 0.002;
    if adjustment > (def.sell_factor - def.buy_factor).abs() * 0.5 {
        adjustment *= 0.5;
    }
    let mut factor = if buy { def.sell_factor - adjustment } else { def.buy_factor + adjustment };
    if factor <= 0.0 {
        factor = 0.1;
    }
    let value = if item.class == EquipClass::Ammunition {
        let bundled = (u64::from(item.value) * u64::from(quantity))
            .div_ceil(u64::from(crate::content::items::AMMUNITION_BUNDLE));
        (factor * bundled as f32) as u64
    } else {
        // Ordinary stacks and separate items are priced per unit. Charges do
        // not reduce the source game's trade valuation.
        u64::from((factor * item.value as f32) as u32) * u64::from(quantity)
    };
    let value = if identified { value } else { value / 4 };
    Ok(value.max(1) as u32)
}
/// Whether the merchant offers this lot now.
pub fn on_sale(lot: &StockLot, item: &ItemDefinition) -> bool {
    lot.units > 0 && lot.available_at == 0 && !item.merchant_cannot_sell
}
/// How much of `units` one trade moves: ammunition goes as a whole stack,
/// anything else one at a time.
pub fn trade_quantity(item: &ItemDefinition, units: u32) -> u32 {
    if item.class == EquipClass::Ammunition { units } else { 1 }
}
/// What `quantity` of the lot costs, if the merchant will sell that many.
pub fn purchase_price(
    source: &StockLot,
    def: &NpcDefinition,
    catalog: &ItemCatalog,
    quantity: u32,
    communication: u32,
) -> Result<u32, TradeError> {
    let item = &catalog[source.item];
    if source.available_at != 0 || item.merchant_cannot_sell {
        return Err(TradeError::Unavailable);
    }
    if quantity == 0
        || quantity > source.units
        || (item.class == EquipClass::Ammunition && quantity != source.units)
    {
        return Err(TradeError::InvalidQuantity);
    }
    price(def, item, quantity, true, true, communication)
}
/// Check the complete delivery against insertion capacity, then commit directly.
pub fn buy(
    stock: &mut MerchantStock,
    party: &mut PartyInventory,
    ids: &mut ItemIds,
    def: &NpcDefinition,
    catalog: &ItemCatalog,
    lot: StockLotId,
    quantity: u32,
    quoted: u32,
    communication: u32,
) -> Result<(), TradeError> {
    let row = stock.lots.iter().position(|l| l.id == lot).ok_or(TradeError::StaleSelection)?;
    let source = &stock.lots[row];
    let cost = purchase_price(source, def, catalog, quantity, communication)?;
    let item = &catalog[source.item];
    if cost != quoted {
        return Err(TradeError::PriceChanged);
    }
    if cost > party.gold {
        return Err(TradeError::Funds);
    }
    if party.pool.room_for(item, true) < quantity {
        return Err(TradeError::Capacity);
    }
    let mut remaining = quantity;
    while remaining > 0 {
        let units = if item.quantity == QuantityKind::Stack {
            remaining.min(u32::from(item.stack_limit).max(1))
        } else {
            1
        };
        let amount = if item.quantity == QuantityKind::Stack {
            units as u16
        } else if item.quantity == QuantityKind::None {
            1
        } else {
            item.initial_quantity.maximum().min(255) as u16
        };
        let delivered = ItemStack::new(item, ids, amount, true);
        let _ = party.pool.receive(delivered, catalog);
        remaining -= units;
    }
    party.gold -= cost;
    stock.lots[row].units -= quantity;
    stock.lots.retain(|l| l.units != 0);
    Ok(())
}
pub fn validate_sale(
    def: &NpcDefinition,
    item: &ItemStack,
    quantity: u32,
    catalog: &ItemCatalog,
    communication: u32,
) -> Result<u32, TradeError> {
    let d = &catalog[item.item];
    if d.crucial || !d.class.merchant_class() || !def.accepted_classes.contains(d.class) {
        return Err(TradeError::Rejected);
    }
    if quantity == 0
        || quantity > item.units()
        || (d.class == EquipClass::Ammunition && quantity != item.units())
    {
        return Err(TradeError::InvalidQuantity);
    }
    price(def, d, quantity, item.identified, false, communication)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::Dice;
    use crate::game::inventory::{ItemPool, make_item};
    fn catalog() -> ItemCatalog {
        ItemCatalog(vec![ItemDefinition {
            id: ItemId(0),
            value: 10,
            quantity: QuantityKind::Stack,
            initial_quantity: Dice { base: 20, ..default() },
            stack_limit: 20,
            ..default()
        }])
    }
    fn merchant() -> NpcDefinition {
        NpcDefinition {
            buy_factor: 0.8,
            sell_factor: 1.2,
            accepted_classes: enumset::EnumSet::all(),
            ..default()
        }
    }
    #[test]
    fn zero_clock_initialization_half_day_and_delayed_stock_keep_their_clocks() {
        use crate::content::npcs::StockRule;
        let mut catalog = catalog();
        catalog.0[0].quantity = QuantityKind::None;
        let def = NpcDefinition {
            stock: vec![StockRule { item: ItemId(0), quantity: 2, persistent: true }],
            ..merchant()
        };
        let mut stock = MerchantStock::default();
        let mut rng = GameRng::seeded(7);
        let mut expected = rng.clone();
        expected.below(100);
        expected.below(100);
        stock.maintain(&def, &catalog, 1, 0, false, &mut rng);
        assert_eq!(rng, expected);
        assert_eq!(stock.restock_at, Some(0));
        assert_eq!(stock.lots[0].units, 2);
        let lot = stock.lots[0].id;
        let initial = stock.clone();
        stock.maintain(&def, &catalog, 1, 0, false, &mut rng);
        assert_eq!(stock, initial);
        assert_eq!(rng, expected);
        stock.lots[0].units = 0;
        stock.maintain(&def, &catalog, 1, DAY_MS / 2, false, &mut rng);
        assert_eq!(stock.lots[0].units, 0);
        assert_eq!(rng, expected);
        let units = jitter(2, &mut expected);
        stock.maintain(&def, &catalog, 1, DAY_MS / 2 + 1, false, &mut rng);
        assert_eq!(stock.lots[0].units, units);
        assert_eq!(rng, expected);
        assert_eq!(stock.lots[0].id, lot);
        assert_eq!(stock.maintenance_at, 0);
        stock.lots[0].units = 10;
        stock.lots[0].available_at = DAY_MS + 10;
        stock.maintain(&def, &catalog, 1, DAY_MS, false, &mut rng);
        assert_eq!(stock.restock_at, Some(DAY_MS));
        assert_eq!(stock.maintenance_at, DAY_MS);
        assert_eq!(stock.lots[0].units, 10);
        assert_eq!(rng, expected);
        stock.maintain(&def, &catalog, 1, DAY_MS + 10, false, &mut rng);
        assert!(!on_sale(&stock.lots[0], &catalog.0[0]));
        stock.maintain(&def, &catalog, 1, DAY_MS + 11, false, &mut rng);
        assert!(on_sale(&stock.lots[0], &catalog.0[0]));
        assert_eq!(stock.lots[0].id, lot);
        assert_eq!(rng, expected);
    }

    #[test]
    fn purchased_charges_come_from_the_definition_maximum() {
        for (maximum, expected) in [(0, 0), (20, 20), (300, 255)] {
            let mut catalog = catalog();
            catalog.0[0].quantity = QuantityKind::Charges;
            catalog.0[0].initial_quantity = Dice { base: maximum, ..default() };
            let def = merchant();
            let mut stock = MerchantStock::default();
            stock.add(ItemId(0), 1, &catalog, false);
            let lot = stock.lots[0].id;
            let cost = price(&def, &catalog.0[0], 1, true, true, 0).unwrap();
            let mut party = PartyInventory { gold: 1000, ..default() };
            buy(&mut stock, &mut party, &mut ItemIds::default(), &def, &catalog, lot, 1, cost, 0)
                .unwrap();
            assert_eq!(party.pool.stacks[0].remaining_uses(), Some(expected));
        }
    }

    #[test]
    fn purchase_revalidates_and_settles_complete_delivery() {
        let catalog = catalog();
        let def = merchant();
        let mut stock = MerchantStock::default();
        stock.add(ItemId(0), 30, &catalog, false);
        let lot = stock.lots[0].id;
        let mut party = PartyInventory { gold: 1000, ..default() };
        let mut ids = ItemIds::default();
        let cost = price(&def, &catalog.0[0], 30, true, true, 0).unwrap();
        let before = stock.clone();
        assert_eq!(
            buy(&mut stock, &mut party, &mut ids, &def, &catalog, lot, 30, cost + 1, 0),
            Err(TradeError::PriceChanged)
        );
        assert_eq!(stock, before);
        assert!(party.pool.stacks.is_empty());
        assert_eq!(party.gold, 1000);
        party.gold = cost - 1;
        assert_eq!(
            buy(&mut stock, &mut party, &mut ids, &def, &catalog, lot, 30, cost, 0),
            Err(TradeError::Funds)
        );
        assert_eq!(stock, before);
        party.gold = 1000;
        buy(&mut stock, &mut party, &mut ids, &def, &catalog, lot, 30, cost, 0).unwrap();
        assert_eq!(party.gold, 1000 - cost);
        assert_eq!(party.pool.stacks.iter().map(|i| i.units()).sum::<u32>(), 30);
        assert!(stock.lots.is_empty());
        assert_eq!(
            buy(&mut stock, &mut party, &mut ids, &def, &catalog, lot, 1, 12, 0),
            Err(TradeError::StaleSelection)
        );
    }
    #[test]
    fn a_purchase_that_cannot_fit_leaves_items_gold_stock_and_ids_untouched() {
        let catalog = catalog();
        let def = merchant();
        let mut stock = MerchantStock::default();
        stock.add(ItemId(0), 30, &catalog, false);
        let mut ids = ItemIds::default();
        let mut rng = GameRng::seeded(1);
        let full = make_item(&catalog.0[0], &mut ids, &mut rng, true, true);
        let mut party = PartyInventory {
            pool: ItemPool {
                stacks: (0..crate::game::inventory::POOL_LIMIT)
                    .map(|_| {
                        let mut item = full.clone();
                        item.id = ids.allocate();
                        item
                    })
                    .collect(),
            },
            gold: 1000,
            ..default()
        };
        party.pool.stacks[0] = ItemStack::new(&catalog.0[0], &mut ids, 19, true);
        let before = party.clone();
        let before_stock = stock.clone();
        let next = ids.clone().allocate();
        let lot = stock.lots[0].id;
        assert_eq!(
            buy(&mut stock, &mut party, &mut ids, &def, &catalog, lot, 30, 360, 0),
            Err(TradeError::Capacity)
        );
        assert_eq!(party, before);
        assert_eq!(stock, before_stock);
        assert_eq!(ids.allocate(), next);
    }
    #[test]
    fn price_rounding_and_stock_normalization_follow_retail() {
        let catalog = catalog();
        let def = merchant();
        let mut ids = ItemIds::default();
        let item = ItemStack::new(&catalog.0[0], &mut ids, 3, false);
        assert_eq!(validate_sale(&def, &item, 3, &catalog, 0), Ok(6));
        let mut stock = MerchantStock::default();
        stock.add(item.item, 3, &catalog, false);
        assert_eq!(stock.lots[0].units, 3);
        // Communication is applied per unit, then truncated before multiplication.
        assert_eq!(price(&def, &catalog.0[0], 3, true, true, 20), Ok(33));
    }
    /// A lot whose item the catalog lacks refuses maintenance before any
    /// stock changes or any random number is drawn.
    #[test]
    fn templar_gate_skips_one_daily_rule_without_disabling_other_stock() {
        use crate::content::npcs::StockRule;
        let mut catalog = catalog();
        catalog.0.resize_with(0x1fd, ItemDefinition::default);
        catalog.0[TEMPLAR_GATED.0 as usize] =
            ItemDefinition { id: TEMPLAR_GATED, ..catalog.0[0].clone() };
        let def = NpcDefinition {
            stock: vec![
                StockRule { item: TEMPLAR_GATED, quantity: 4, persistent: false },
                StockRule { item: ItemId(0), quantity: 4, persistent: true },
            ],
            ..merchant()
        };
        let mut stock = MerchantStock { restock_at: Some(0), ..default() };
        let mut rng = GameRng::seeded(8);
        stock.maintain(&def, &catalog, 1, DAY_MS, false, &mut rng);
        assert!(stock.lots.iter().all(|lot| lot.item != TEMPLAR_GATED));
        assert!(stock.lots.iter().any(|lot| lot.item == ItemId(0) && lot.units > 0));
        assert_eq!(stock.maintenance_at, DAY_MS);
        for day in 2..=10 {
            stock.maintain(&def, &catalog, 1, day * DAY_MS, true, &mut rng);
            if stock.lots.iter().any(|lot| lot.item == TEMPLAR_GATED && lot.units > 0) {
                return;
            }
        }
        panic!("the open gate must allow replenishment with the seeded rolls");
    }
}
