//! Stock is merchandise, not the NPC's personal equipment. Commit transfers
//! and payment synchronously after validating the entire request.
use crate::content::{
    items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, QuantityKind},
    npcs::NpcDefinition,
};
use crate::game::{
    inventory::{ItemIds, ItemStack, PartyInventory, StackId},
    rng::GameRng,
};
use bevy::prelude::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockLotId(pub u64);
#[derive(Clone, Debug, PartialEq)]
pub struct StockLot {
    pub id: StockLotId,
    pub item: ItemId,
    pub units: u32,
    pub charges: u8,
    pub available_at: u64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MerchantStock {
    pub lots: Vec<StockLot>,
    pub initialized: bool,
    pub restock_at: u64,
    pub maintenance_at: u64,
    next: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeError {
    MissingContent,
    StaleSelection,
    Unavailable,
    InvalidQuantity,
    PriceChanged,
    Funds,
    Capacity,
    Rejected,
    Busy,
    Overflow,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeSource {
    Stock(StockLotId),
    Pool(StackId),
    Backpack { member: Entity, item: StackId },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeQuote {
    pub conversation: Entity,
    pub revision: u64,
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
    ) -> Result<(), TradeError> {
        let d = catalog.get(item).ok_or(TradeError::MissingContent)?;
        if units == 0 {
            return Ok(());
        }
        if d.class == EquipClass::Ammunition && ammunition_bundle {
            for _ in 0..units {
                self.new_lot(d, 25)
            }
        } else if d.class != EquipClass::Ammunition
            && let Some(lot) = self.lots.iter_mut().find(|l| l.item == item && l.available_at == 0)
        {
            lot.units = lot.units.checked_add(units).ok_or(TradeError::Overflow)?;
        } else {
            self.new_lot(d, units)
        }
        Ok(())
    }
    fn new_lot(&mut self, d: &ItemDefinition, units: u32) {
        self.next += 1;
        self.lots.push(StockLot {
            id: StockLotId(self.next),
            item: d.id,
            units,
            charges: d.initial_quantity.maximum().min(255) as u8,
            available_at: 0,
        });
    }
    pub fn maintain(
        &mut self,
        def: &NpcDefinition,
        catalog: &ItemCatalog,
        level: u32,
        now: u64,
        rng: &mut GameRng,
    ) -> Result<(), TradeError> {
        // Validate before consuming RNG or changing stock.
        if def.stock.iter().any(|r| catalog.get(r.item).is_none()) {
            return Err(TradeError::MissingContent);
        }
        if !self.initialized {
            for r in &def.stock {
                let chance =
                    if r.persistent { 100 } else { tier(catalog.get(r.item).unwrap(), level) * 25 };
                let n = (0..r.quantity).filter(|_| rng.below(100) < chance).count() as u32;
                self.add(r.item, n, catalog, true)?;
            }
            self.initialized = true;
            self.restock_at = now;
            self.maintenance_at = now;
            return Ok(());
        }
        let daily = now.saturating_sub(self.maintenance_at) >= 86_400_000;
        if daily && def.stock.iter().any(|r| !r.persistent && r.item == ItemId(0x1fc)) {
            return Err(TradeError::Unavailable);
        }
        if now.saturating_sub(self.restock_at) > 43_200_000 {
            self.replenish(def, catalog, level, true, rng)?;
            self.restock_at = now;
        }
        if daily {
            for lot in &mut self.lots {
                let d = catalog.get(lot.item).ok_or(TradeError::MissingContent)?;
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
            self.replenish(def, catalog, level, false, rng)?;
            self.lots.retain(|l| l.units > 0);
            self.maintenance_at = now;
            self.restock_at = now;
        }
        for lot in &mut self.lots {
            if lot.available_at != 0 && lot.available_at < now {
                lot.available_at = 0;
            }
        }
        Ok(())
    }
    fn replenish(
        &mut self,
        def: &NpcDefinition,
        catalog: &ItemCatalog,
        level: u32,
        persistent: bool,
        rng: &mut GameRng,
    ) -> Result<(), TradeError> {
        for r in &def.stock {
            if r.persistent != persistent {
                continue;
            }
            // This original restock gate requires story semantics not yet ported.
            if r.item == ItemId(0x1fc) {
                return Err(TradeError::Unavailable);
            }
            let held = self.lots.iter().find(|l| l.item == r.item).map_or(0, |l| {
                if catalog.get(r.item).unwrap().quantity == QuantityKind::Stack {
                    1
                } else {
                    l.units
                }
            });
            let configured = u32::from(r.quantity);
            if configured == 0 || held > configured / 2 {
                continue;
            }
            if !persistent && rng.below(100) >= tier(catalog.get(r.item).unwrap(), level) * 25 {
                continue;
            }
            let n = jitter(configured - held, rng);
            self.add(r.item, n, catalog, true)?;
        }
        Ok(())
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
        let bundled = (u64::from(item.value) * u64::from(quantity)).div_ceil(25);
        (factor * bundled as f32) as u64
    } else {
        // Ordinary stacks and separate items are priced per unit. Charges do
        // not reduce the source game's trade valuation.
        u64::from((factor * item.value as f32) as u32) * u64::from(quantity)
    };
    let value = if identified { value } else { value / 4 };
    u32::try_from(value.max(1)).map_err(|_| TradeError::Overflow)
}
/// The delivery draft uses existing pool insertion, including partial merging.
/// No authoritative item, allocator, stock or payment changes until it all fits.
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
    let item = catalog.get(source.item).ok_or(TradeError::MissingContent)?;
    if source.available_at != 0 || item.merchant_cannot_sell {
        return Err(TradeError::Unavailable);
    }
    if quantity == 0
        || quantity > source.units
        || (item.class == EquipClass::Ammunition && quantity != source.units)
    {
        return Err(TradeError::InvalidQuantity);
    }
    let cost = price(def, item, quantity, true, true, communication)?;
    if cost != quoted {
        return Err(TradeError::PriceChanged);
    }
    if cost > party.gold {
        return Err(TradeError::Funds);
    }
    let mut draft = party.clone();
    let mut next_ids = ids.clone();
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
            u16::from(source.charges)
        };
        let delivered = ItemStack::new(item, &mut next_ids, amount, true);
        if draft.pool.receive(delivered, catalog).is_some() {
            return Err(TradeError::Capacity);
        }
        remaining -= units;
    }
    draft.gold -= cost;
    *party = draft;
    *ids = next_ids;
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
    let d = catalog.get(item.item).ok_or(TradeError::MissingContent)?;
    if d.crucial || u8::from(d.class) >= 26 || def.accepted_classes & (1 << u8::from(d.class)) == 0
    {
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
        NpcDefinition { buy_factor: 0.8, sell_factor: 1.2, accepted_classes: u32::MAX, ..default() }
    }
    #[test]
    fn purchase_revalidates_and_settles_complete_delivery() {
        let catalog = catalog();
        let def = merchant();
        let mut stock = MerchantStock::default();
        stock.add(ItemId(0), 30, &catalog, false).unwrap();
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
    fn full_destination_rolls_back_partial_merges_and_allocator() {
        let catalog = catalog();
        let def = merchant();
        let mut stock = MerchantStock::default();
        stock.add(ItemId(0), 30, &catalog, false).unwrap();
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
        stock.add(item.item, 3, &catalog, false).unwrap();
        assert_eq!(stock.lots[0].units, 3);
        assert_eq!(stock.lots[0].charges, 20);
        // Communication is applied per unit, then truncated before multiplication.
        assert_eq!(price(&def, &catalog.0[0], 3, true, true, 20), Ok(33));
    }
}
