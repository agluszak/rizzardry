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
    let base = level * 2 / 3;
    let lower = base.clamp(1, 16);
    let upper = (base + 4).clamp(4, 20);
    let difficulty = def.identify_difficulty as u32;
    if difficulty > upper {
        0
    } else if difficulty < lower {
        1 + (difficulty + 4 >= lower) as u32
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
        let daily = now - self.maintenance_at >= DAY_MS;
        if now - restock_at > DAY_MS / 2 {
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
                lot.units -= count.min(lot.units);
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
