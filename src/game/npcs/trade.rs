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
        inventory::PartyInventory,
        items::{Charges, Give, Items, Known, Lose, Place, Put, make, split},
        party::roster::PartyRoster,
        rng::GameRng,
        time::{DAY_MS, GameClock},
    },
};
use bevy::{ecs::system::SystemParam, prelude::*};
/// On a merchant: when its stock was last filled up and last turned over.
/// What it has to sell is the items stowed with it.
#[derive(Component, Debug, Default)]
pub struct MerchantStock {
    restocked: u64,
    maintained: u64,
}

/// What a merchant has of one kind to sell: its items of that kind taken
/// together, or one bundle of ammunition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shelf {
    pub item: ItemId,
    /// The items on it, the first naming it.
    pub items: Vec<Entity>,
    pub units: u32,
}

/// A merchant's shelves, in the order its stock came in.
pub fn shelves(held: &Items, merchant: Entity, catalog: &ItemCatalog) -> Vec<Shelf> {
    let mut shelves: Vec<Shelf> = Vec::new();
    for &item in held.stowed(merchant) {
        let Some(id) = held.id(item) else { continue };
        let units = held.units(item);
        let bundled = catalog[id].class == EquipClass::Ammunition;
        match shelves.iter_mut().find(|shelf| shelf.item == id).filter(|_| !bundled) {
            Some(shelf) => {
                shelf.items.push(item);
                shelf.units += units;
            }
            None => shelves.push(Shelf { item: id, items: vec![item], units }),
        }
    }
    shelves
}

/// The shelf an item of a merchant's stock is on.
pub fn shelf(held: &Items, merchant: Entity, of: Entity, catalog: &ItemCatalog) -> Option<Shelf> {
    shelves(held, merchant, catalog).into_iter().find(|shelf| shelf.items.contains(&of))
}

/// Put `units` of an item on a merchant's shelves: ammunition come from the
/// maker in bundles of 25, anything that stacks as one stack, anything else
/// one by one.
fn stock(commands: &mut Commands, merchant: Entity, definition: &ItemDefinition, units: u32) {
    let mut put = |amount: u16| {
        let item = make(commands, definition, amount, true);
        commands.trigger(Put { item, at: Place::Container(merchant) });
    };
    if definition.class == EquipClass::Ammunition {
        (0..units).for_each(|_| put(25));
    } else if definition.quantity == QuantityKind::Stack {
        if units > 0 {
            put(units as u16);
        }
    } else {
        let charges = match definition.quantity {
            QuantityKind::None => 1,
            _ => definition.initial_quantity.maximum() as u16,
        };
        (0..units).for_each(|_| put(charges));
    }
}

/// A merchant opens: its stock as its record says, each thing it does not
/// always keep there with the chance the party's level gives it.
pub fn open(
    commands: &mut Commands,
    merchant: Entity,
    d: &NpcDefinition,
    items: &ItemCatalog,
    level: u32,
    now: u64,
    rng: &mut GameRng,
) {
    commands.entity(merchant).insert(MerchantStock { restocked: now, maintained: now });
    for rule in &d.stock {
        let probability = if rule.persistent { 100 } else { chance(&items[rule.item], level) };
        let count = (0..rule.quantity).filter(|_| rng.below(100) < probability).count() as u32;
        stock(commands, merchant, &items[rule.item], count);
    }
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
    /// Something of the merchant's stock, which names its shelf.
    Stock(Entity),
    /// An item the party has: in the pool, or packed (not worn) by a member.
    Party(Entity),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
    pub merchant: Entity,
    pub source: TradeSource,
    pub quantity: u32,
    pub gold: u32,
}
/// Something a merchant is to have on its shelves from a time on (retail
/// `AddNpcItemWithDelay`): it is there when the merchant is next visited
/// after that time.
#[derive(Component, Clone, Copy, Debug)]
pub struct Delivery {
    pub merchant: Entity,
    pub item: ItemId,
    pub units: u32,
    /// Game milliseconds.
    pub due: u64,
}

/// The party visits someone, who keeps shop if a merchant.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Visit {
    #[event_target]
    pub merchant: Entity,
}
pub fn install(app: &mut App) {
    app.add_observer(maintain);
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
/// What a merchant is short of, by the rules of its record that are
/// `persistent` or not, comes in: nothing while it has more than half of
/// what a rule wants (of a thing that stacks, any at all), a thing it does
/// not always keep only with its chance, and then what it lacks, half as
/// much again or half as much less a third of the time each. `have` says how
/// many of an item it has.
fn replenish(
    commands: &mut Commands,
    merchant: Entity,
    d: &NpcDefinition,
    items: &ItemCatalog,
    (level, persistent, templar): (u32, bool, bool),
    have: impl Fn(ItemId) -> u32,
    rng: &mut GameRng,
) {
    for r in d.stock.iter().filter(|r| r.persistent == persistent) {
        if !persistent && r.item == ItemId(0x1fc) && !templar {
            continue;
        }
        let stacks = items[r.item].quantity == QuantityKind::Stack;
        let have = if stacks { have(r.item).min(1) } else { have(r.item) };
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
        stock(commands, merchant, &items[r.item], n);
    }
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
/// Whether the party has the item to sell or give: in its pool, or in a
/// pack of one of its members (worn items are not for sale).
pub fn for_sale(item: Entity, roster: &PartyRoster, held: &Items) -> Result<(), TradeError> {
    match held.place(item) {
        Some(Place::Pool { .. }) => Ok(()),
        Some(Place::Pack { member, .. }) if roster.contains(&member) => Ok(()),
        _ => Err(TradeError::StaleSelection),
    }
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
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals)>,
    held: Items<'w, 's>,
    content: Res<'w, Catalogs>,
}
impl Pricing<'_, '_> {
    pub fn quote(
        &self,
        merchant: Entity,
        source: TradeSource,
        units: u32,
    ) -> Result<Quote, TradeError> {
        let (id, _) = self.stocks.get(merchant).map_err(|_| TradeError::Unavailable)?;
        let roster = *self.roster;
        let skill = communication(roster.iter().filter_map(|m| self.members.get(m).ok()));
        let d = &self.content.npcs[id.0];
        let gold = match source {
            TradeSource::Stock(of) => {
                let shelf = shelf(&self.held, merchant, of, &self.content.items)
                    .ok_or(TradeError::StaleSelection)?;
                let item = &self.content.items[shelf.item];
                if item.merchant_cannot_sell {
                    return Err(TradeError::Unavailable);
                }
                quantity(item, units, shelf.units)?;
                price(d, item, units, true, true, skill)
            }
            TradeSource::Party(source) => {
                for_sale(source, roster, &self.held)?;
                let (Some(id), Some(known)) = (self.held.id(source), self.held.known(source))
                else {
                    return Err(TradeError::StaleSelection);
                };
                let item = &self.content.items[id];
                if item.crucial
                    || !item.class.merchant_class()
                    || !d.accepted_classes.contains(item.class)
                {
                    return Err(TradeError::Rejected);
                }
                quantity(item, units, self.held.units(source))?;
                price(d, item, units, known.identified, false, skill)
            }
        };
        Ok(Quote { merchant, source, quantity: units, gold })
    }
}
#[derive(SystemParam)]
pub struct Settlement<'w, 's> {
    stocks: Query<'w, 's, (), With<MerchantStock>>,
    party: Single<'w, 's, (Entity, &'static PartyRoster, &'static mut PartyInventory)>,
    held: Items<'w, 's>,
    content: Res<'w, Catalogs>,
    commands: Commands<'w, 's>,
}
impl Settlement<'_, '_> {
    fn settle(&mut self, q: Quote, current: Quote) -> Result<(), TradeError> {
        if current.gold != q.gold {
            return Err(TradeError::PriceChanged);
        }
        if !self.stocks.contains(q.merchant) {
            return Err(TradeError::Unavailable);
        }
        let (party, roster, inventory) = &mut *self.party;
        match q.source {
            // What is bought is the merchant's own items, handed over: whole
            // ones first, then part of a stack.
            TradeSource::Stock(of) => {
                let shelf = shelf(&self.held, q.merchant, of, &self.content.items)
                    .ok_or(TradeError::StaleSelection)?;
                let item = &self.content.items[shelf.item];
                if inventory.gold < q.gold {
                    return Err(TradeError::Funds);
                }
                let per_row = if item.quantity == QuantityKind::Stack {
                    u32::from(item.stack_limit).max(1)
                } else {
                    1
                };
                if self.held.pool_room(*party, item.id, true, per_row) < q.quantity {
                    return Err(TradeError::Capacity);
                }
                let mut remaining = q.quantity;
                for &on_shelf in &shelf.items {
                    if remaining == 0 {
                        break;
                    }
                    let units = self.held.units(on_shelf);
                    let bought = if units <= remaining {
                        on_shelf
                    } else {
                        split(&mut self.commands, on_shelf, remaining as u16)
                    };
                    self.commands.trigger(Give {
                        item: bought,
                        to: None,
                        party: *party,
                        pool_first: true,
                        told: false,
                    });
                    remaining -= units.min(remaining);
                }
                inventory.gold -= q.gold;
            }
            // What is sold goes onto the merchant's shelves as the thing it
            // is, known for what it is and with its charges made up.
            TradeSource::Party(source) => {
                for_sale(source, roster, &self.held)?;
                let sold = if q.quantity < self.held.units(source) {
                    split(&mut self.commands, source, q.quantity as u16)
                } else {
                    source
                };
                let mut item = self.commands.entity(sold);
                item.entry::<Known>().and_modify(|mut known| known.identified = true);
                item.entry::<Charges>().and_modify(|mut charges| charges.left = charges.full);
                self.commands.trigger(Put { item: sold, at: Place::Container(q.merchant) });
                inventory.gold += q.gold;
            }
        }
        Ok(())
    }
}
/// Buy or sell at a quoted price, if it is still the price.
pub fn settle(market: &mut ParamSet<(Pricing, Settlement)>, q: Quote) -> Result<(), TradeError> {
    let current = market.p0().quote(q.merchant, q.source, q.quantity)?;
    market.p1().settle(q, current)
}
/// A merchant visited keeps shop: after half a day what it always keeps is
/// made up; after a day what it does not always keep turns over (one of each
/// stack goes, and all of anything else, but for what is crucial or not for
/// sale) and what it lacks may come in.
fn maintain(
    visit: On<Visit>,
    deliveries: Query<(Entity, &Delivery)>,
    mut stocks: Query<(&ActorId, &mut MerchantStock)>,
    held: Items,
    content: If<Res<Catalogs>>,
    clock: Res<GameClock>,
    story: Res<super::story::StoryFacts>,
    party: Query<&PartyRoster>,
    members: Query<&Character>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let average = crate::game::party::average_level(
        party
            .iter()
            .flat_map(|p| p.iter())
            .filter_map(|m| members.get(m).ok())
            .map(Character::level),
    );
    let items = &content.items;
    {
        let merchant = visit.merchant;
        let Ok((id, mut stock)) = stocks.get_mut(merchant) else { return };
        // What was to come has come.
        for (delivery, due) in &deliveries {
            if due.merchant == merchant && due.due <= clock.ms {
                self::stock(&mut commands, merchant, &items[due.item], due.units);
                commands.entity(delivery).despawn();
            }
        }
        let d = &content.npcs[id.0];
        let shelves = shelves(&held, merchant, items);
        let have = |item| shelves.iter().filter(|s| s.item == item).map(|s| s.units).sum::<u32>();
        if clock.ms - stock.restocked > DAY_MS / 2 {
            replenish(&mut commands, merchant, d, items, (average, true, false), have, &mut rng);
            stock.restocked = clock.ms;
        }
        if clock.ms - stock.maintained >= DAY_MS {
            let mut gone: Vec<(ItemId, u32)> = Vec::new();
            for shelf in &shelves {
                let item = &items[shelf.item];
                if item.crucial
                    || item.merchant_cannot_sell
                    || d.stock.iter().any(|r| r.persistent && r.item == shelf.item)
                {
                    continue;
                }
                if item.quantity == QuantityKind::Stack {
                    commands.trigger(Lose { item: shelf.items[0], amount: 1 });
                    gone.push((shelf.item, 1));
                } else {
                    shelf.items.iter().for_each(|&item| commands.entity(item).despawn());
                    gone.push((shelf.item, shelf.units));
                }
            }
            let templar = super::story::fact_named(&story, &content.facts, "FACT_TEMPLAR")
                .is_ok_and(|v| v != 0);
            let left =
                |item| have(item) - gone.iter().filter(|g| g.0 == item).map(|g| g.1).sum::<u32>();
            replenish(&mut commands, merchant, d, items, (average, false, templar), left, &mut rng);
            stock.maintained = clock.ms;
            stock.restocked = clock.ms;
        }
    }
}
