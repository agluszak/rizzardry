//! Merchant inventory and atomic transfers. No dialogue state is read here.
use super::ActorId;
use crate::{
    content::{
        items::{EquipClass, ItemCatalog, ItemDefinition, ItemId, QuantityKind},
        npcs::NpcDefinition,
    },
    game::{
        characters::{Character, Vitals},
        data::Catalogs,
        inventory::{Carried, ItemIds, ItemOwner, ItemRef, ItemStack, PartyInventory},
        party::roster::PartyRoster,
        rng::GameRng,
        time::{DAY_MS, GameClock},
    },
};
use bevy::{ecs::system::SystemParam, prelude::*};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockLotId(pub u64);
#[derive(Clone, Debug)]
pub struct StockLot {
    pub id: StockLotId,
    pub item: ItemId,
    pub units: u32,
    pub available_at: u64,
}
#[derive(Component, Debug, Default)]
pub struct MerchantStock {
    pub lots: Vec<StockLot>,
    next: u64,
    restocked: u64,
    maintained: u64,
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
pub struct Quote {
    pub merchant: Entity,
    pub source: TradeSource,
    pub quantity: u32,
    pub gold: u32,
}
#[derive(Message)]
pub struct Visit(pub Entity);
#[derive(Message)]
pub struct Order {
    pub reply_to: Entity,
    pub quote: Quote,
}
#[derive(Message)]
pub struct Receipt {
    pub reply_to: Entity,
    pub result: Result<(), TradeError>,
}
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Trading;
pub fn install(app: &mut App) {
    app.add_message::<Visit>()
        .add_message::<Order>()
        .add_message::<Receipt>()
        .add_systems(Update, (maintain, settle).chain().in_set(Trading));
}
fn chance(item: &ItemDefinition, level: u32) -> u32 {
    let centre = level * 2 / 3;
    let low = centre.clamp(1, 16);
    let high = (centre + 4).clamp(4, 20);
    let d = u32::from(item.identify_difficulty);
    if d > high {
        0
    } else if d < low {
        if d + 4 >= low { 50 } else { 25 }
    } else {
        75
    }
}
impl MerchantStock {
    pub fn new(
        d: &NpcDefinition,
        items: &ItemCatalog,
        level: u32,
        now: u64,
        rng: &mut GameRng,
    ) -> Self {
        let mut stock = Self { restocked: now, maintained: now, ..default() };
        for rule in &d.stock {
            let probability = if rule.persistent { 100 } else { chance(&items[rule.item], level) };
            let count = (0..rule.quantity).filter(|_| rng.below(100) < probability).count() as u32;
            stock.add(rule.item, count, items, true);
        }
        stock
    }
    pub fn lot(&self, id: StockLotId) -> Result<&StockLot, TradeError> {
        self.lots.iter().find(|l| l.id == id).ok_or(TradeError::StaleSelection)
    }
    fn add(&mut self, item: ItemId, units: u32, catalog: &ItemCatalog, bundles: bool) {
        if units == 0 {
            return;
        }
        let ammo = catalog[item].class == EquipClass::Ammunition;
        if !ammo
            && let Some(l) = self.lots.iter_mut().find(|l| l.item == item && l.available_at == 0)
        {
            l.units += units;
            return;
        }
        let count = if ammo && bundles { units } else { 1 };
        for _ in 0..count {
            self.next += 1;
            self.lots.push(StockLot {
                id: StockLotId(self.next),
                item,
                units: if ammo && bundles { 25 } else { units },
                available_at: 0,
            });
        }
    }
    fn replenish(
        &mut self,
        d: &NpcDefinition,
        items: &ItemCatalog,
        level: u32,
        persistent: bool,
        templar: bool,
        rng: &mut GameRng,
    ) {
        for r in d.stock.iter().filter(|r| r.persistent == persistent) {
            if !persistent && r.item == ItemId(0x1fc) && !templar {
                continue;
            }
            let have = self.lots.iter().find(|l| l.item == r.item).map_or(0, |l| {
                if items[r.item].quantity == QuantityKind::Stack { 1 } else { l.units }
            });
            let wanted = u32::from(r.quantity);
            if wanted == 0 || have > wanted / 2 {
                continue;
            }
            if !persistent && rng.below(100) >= chance(&items[r.item], level) {
                continue;
            }
            let n = wanted - have;
            let n = match rng.below(3) {
                0 => n + n / 2,
                1 => n - n / 2,
                _ => n,
            };
            self.add(r.item, n, items, true);
        }
    }
}
pub fn on_sale(l: &StockLot, d: &ItemDefinition) -> bool {
    l.units > 0 && l.available_at == 0 && !d.merchant_cannot_sell
}
pub fn trade_quantity(d: &ItemDefinition, units: u32) -> u32 {
    if d.class == EquipClass::Ammunition { units } else { 1 }
}
pub fn communication<'a>(members: impl Iterator<Item = (&'a Character, &'a Vitals)>) -> u32 {
    members
        .filter(|(_, v)| v.can_act())
        .map(|(c, _)| c.skill(crate::content::characters::Skill::Communication))
        .max()
        .unwrap_or(0)
}
fn price(
    d: &NpcDefinition,
    item: &ItemDefinition,
    units: u32,
    identified: bool,
    buying: bool,
    skill: u32,
) -> u32 {
    let mut adjustment = skill as f32 * 0.002;
    if adjustment > (d.sell_factor - d.buy_factor).abs() / 2.0 {
        adjustment /= 2.0;
    }
    let factor = if buying { d.sell_factor - adjustment } else { d.buy_factor + adjustment };
    let factor = if factor <= 0.0 { 0.1 } else { factor };
    let amount = if item.class == EquipClass::Ammunition {
        let value = (u64::from(item.value) * u64::from(units))
            .div_ceil(u64::from(crate::content::items::AMMUNITION_BUNDLE));
        (factor * value as f32) as u64
    } else {
        u64::from((factor * item.value as f32) as u32) * u64::from(units)
    };
    (if identified { amount } else { amount / 4 }).clamp(1, u64::from(u32::MAX)) as u32
}
pub fn sale_item<'a>(
    source: ItemRef,
    inventory: &'a PartyInventory,
    pack: Option<&'a Carried>,
) -> Result<&'a ItemStack, TradeError> {
    match source.owner {
        ItemOwner::Pool => inventory.pool.stacks.iter().find(|s| s.id == source.id),
        ItemOwner::Member(_) => pack.and_then(|p| p.backpack_stack(source.id)),
    }
    .ok_or(TradeError::StaleSelection)
}
fn quantity(d: &ItemDefinition, asked: u32, available: u32) -> Result<(), TradeError> {
    if asked == 0 || asked > available || (d.class == EquipClass::Ammunition && asked != available)
    {
        Err(TradeError::InvalidQuantity)
    } else {
        Ok(())
    }
}
/// Read-only economic inputs. Pricing does not reserve stock or mutate inventories.
#[derive(SystemParam)]
pub struct Pricing<'w, 's> {
    stocks: Query<'w, 's, (&'static ActorId, &'static MerchantStock)>,
    party: Single<'w, 's, (&'static PartyRoster, &'static PartyInventory)>,
    members: Query<'w, 's, (&'static Character, &'static Vitals)>,
    packs: Query<'w, 's, &'static Carried>,
    content: Res<'w, Catalogs>,
}
impl Pricing<'_, '_> {
    pub fn quote(
        &self,
        merchant: Entity,
        source: TradeSource,
        units: u32,
    ) -> Result<Quote, TradeError> {
        let (id, stock) = self.stocks.get(merchant).map_err(|_| TradeError::Unavailable)?;
        let (roster, inventory) = &*self.party;
        let skill = communication(roster.iter().filter_map(|m| self.members.get(m).ok()));
        let d = &self.content.npcs[id.0];
        let gold = match source {
            TradeSource::Stock(id) => {
                let lot = stock.lot(id)?;
                let item = &self.content.items[lot.item];
                if !on_sale(lot, item) {
                    return Err(TradeError::Unavailable);
                }
                quantity(item, units, lot.units)?;
                price(d, item, units, true, true, skill)
            }
            TradeSource::Party(source) => {
                let pack = match source.owner {
                    ItemOwner::Pool => None,
                    ItemOwner::Member(m) => {
                        if !roster.contains(&m) {
                            return Err(TradeError::StaleSelection);
                        }
                        self.packs.get(m).ok()
                    }
                };
                let stack = sale_item(source, inventory, pack)?;
                let item = &self.content.items[stack.item];
                if item.crucial
                    || !item.class.merchant_class()
                    || !d.accepted_classes.contains(item.class)
                {
                    return Err(TradeError::Rejected);
                }
                quantity(item, units, stack.units())?;
                price(d, item, units, stack.identified, false, skill)
            }
        };
        Ok(Quote { merchant, source, quantity: units, gold })
    }
}
#[derive(SystemParam)]
struct Settlement<'w, 's> {
    stocks: Query<'w, 's, &'static mut MerchantStock>,
    inventory: Single<'w, 's, &'static mut PartyInventory>,
    packs: Query<'w, 's, &'static mut Carried>,
    content: Res<'w, Catalogs>,
    ids: ResMut<'w, ItemIds>,
}
impl Settlement<'_, '_> {
    fn settle(&mut self, q: Quote, current: Quote) -> Result<(), TradeError> {
        if current.gold != q.gold {
            return Err(TradeError::PriceChanged);
        }
        let mut stock = self.stocks.get_mut(q.merchant).map_err(|_| TradeError::Unavailable)?;
        let inventory = &mut *self.inventory;
        match q.source {
            TradeSource::Stock(id) => {
                let row =
                    stock.lots.iter().position(|l| l.id == id).ok_or(TradeError::StaleSelection)?;
                let item = &self.content.items[stock.lots[row].item];
                if inventory.gold < q.gold {
                    return Err(TradeError::Funds);
                }
                if inventory.pool.room_for(item, true) < q.quantity {
                    return Err(TradeError::Capacity);
                }
                let mut remaining = q.quantity;
                while remaining > 0 {
                    let units = if item.quantity == QuantityKind::Stack {
                        remaining.min(u32::from(item.stack_limit).max(1))
                    } else {
                        1
                    };
                    let count = match item.quantity {
                        QuantityKind::Stack => units as u16,
                        QuantityKind::None => 1,
                        _ => item.initial_quantity.maximum() as u16,
                    };
                    let stack = ItemStack::new(item, &mut self.ids, count, true);
                    inventory.pool.receive(stack, &self.content.items);
                    remaining -= units;
                }
                inventory.gold -= q.gold;
                stock.lots[row].units -= q.quantity;
                stock.lots.retain(|l| l.units > 0);
            }
            TradeSource::Party(source) => {
                let pack = match source.owner {
                    ItemOwner::Pool => None,
                    ItemOwner::Member(m) => self.packs.get(m).ok(),
                };
                let item = sale_item(source, inventory, pack)?.item;
                match source.owner {
                    ItemOwner::Pool => inventory.pool.consume_units(source.id, q.quantity as u16),
                    ItemOwner::Member(m) => self
                        .packs
                        .get_mut(m)
                        .map_err(|_| TradeError::StaleSelection)?
                        .consume_backpack_units(source.id, q.quantity as u16),
                };
                stock.add(item, q.quantity, &self.content.items, false);
                inventory.gold += q.gold;
            }
        }
        Ok(())
    }
}
fn settle(
    mut orders: MessageReader<Order>,
    mut receipts: MessageWriter<Receipt>,
    mut market: ParamSet<(Pricing, Settlement)>,
) {
    for order in orders.read() {
        let q = order.quote;
        let priced = market.p0().quote(q.merchant, q.source, q.quantity);
        let result = priced.and_then(|current| market.p1().settle(q, current));
        receipts.write(Receipt { reply_to: order.reply_to, result });
    }
}
fn maintain(
    mut visits: MessageReader<Visit>,
    mut stocks: Query<(&ActorId, &mut MerchantStock)>,
    content: If<Res<Catalogs>>,
    clock: Res<GameClock>,
    story: Res<super::story::StoryFacts>,
    party: Query<&PartyRoster>,
    members: Query<&Character>,
    mut rng: ResMut<GameRng>,
) {
    let average = crate::game::party::average_level(
        party
            .iter()
            .flat_map(|p| p.iter())
            .filter_map(|m| members.get(m).ok())
            .map(Character::level),
    );
    for visit in visits.read() {
        let Ok((id, mut stock)) = stocks.get_mut(visit.0) else { continue };
        let d = &content.npcs[id.0];
        if clock.ms - stock.restocked > DAY_MS / 2 {
            stock.replenish(d, &content.items, average, true, false, &mut rng);
            stock.restocked = clock.ms;
        }
        if clock.ms - stock.maintained >= DAY_MS {
            for lot in &mut stock.lots {
                let item = &content.items[lot.item];
                if lot.available_at != 0
                    || item.crucial
                    || item.merchant_cannot_sell
                    || d.stock.iter().any(|r| r.persistent && r.item == lot.item)
                {
                    continue;
                }
                lot.units -= if item.quantity == QuantityKind::Stack { 1 } else { lot.units };
            }
            let templar = super::story::fact_named(&story, &content.facts, "FACT_TEMPLAR")
                .is_ok_and(|v| v != 0);
            stock.replenish(d, &content.items, average, false, templar, &mut rng);
            stock.lots.retain(|l| l.units > 0);
            stock.maintained = clock.ms;
            stock.restocked = clock.ms;
        }
        for l in &mut stock.lots {
            if l.available_at != 0 && l.available_at < clock.ms {
                l.available_at = 0;
            }
        }
    }
}
