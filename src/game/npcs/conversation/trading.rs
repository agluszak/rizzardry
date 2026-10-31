//! Buying from and selling to an NPC in conversation.
use super::*;

/// Where an NPC stands in the played level: the body of its that is
/// there and alive.
#[derive(bevy::ecs::system::SystemParam)]
pub struct NpcPresence<'w, 's> {
    bodies: Query<'w, 's, (&'static Monster, &'static crate::game::world::level::LevelOwned)>,
    levels: Query<'w, 's, (), With<Level>>,
}

impl NpcPresence<'_, '_> {
    pub fn body(&self, bodies: Option<&crate::game::npcs::NpcBodies>) -> Option<Entity> {
        bodies.into_iter().flat_map(|b| b.iter()).find(|&body| {
            self.bodies
                .get(body)
                .is_ok_and(|(monster, owner)| self.levels.contains(owner.0) && !monster.is_dead())
        })
    }

    pub fn monster(&self, body: Entity) -> &Monster {
        self.bodies.get(body).unwrap().0
    }
}

/// What a trade reads and changes: the conversation and its NPC, the
/// NPC's presence and stance, the party's purse, pool and packs, and the
/// catalogs.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Trading<'w, 's> {
    conversations: Query<'w, 's, (&'static mut Conversation, Has<Closing>)>,
    npcs: Query<
        'w,
        's,
        (
            &'static mut Npc,
            &'static crate::game::npcs::ActorId,
            Option<&'static crate::game::npcs::NpcBodies>,
        ),
    >,
    presence: NpcPresence<'w, 's>,
    stances: crate::game::world::hostility::Stances<'w, 's>,
    party: Single<'w, 's, (&'static mut PartyInventory, &'static PartyRoster)>,
    members: Query<'w, 's, (&'static Character, &'static Vitals)>,
    packs: Query<'w, 's, &'static mut Carried>,
    ids: ResMut<'w, ItemIds>,
    content: If<Res<'w, Catalogs>>,
    clock: Res<'w, GameClock>,
    entering: Query<'w, 's, (), With<crate::game::world::level::Destination>>,
    battles: Query<'w, 's, (), With<crate::game::combat::Battle>>,
    camp: Option<Res<'w, crate::game::camp::Camp>>,
}

impl Trading<'_, '_> {
    /// The NPC stands in the played level, alive and not hostile, and owns
    /// stock to trade with.
    fn open_for_trade(&self, actor: Entity) -> bool {
        let (_, id, bodies) = self.npcs.get(actor).unwrap();
        let species = &self.content.species;
        let standing = self.presence.body(bodies).is_some_and(|body| {
            self.stances.own(body, species, self.clock.ms)
                != Some(crate::game::world::hostility::Stance::Hostile)
        });
        standing && self.content.npcs[id.npc()].owns_stock
    }

    fn communication(&self) -> u32 {
        crate::game::party::Lineup::of(self.party.1, |m| self.members.get(m).unwrap())
            .best(crate::content::characters::Skill::Communication)
            .map_or(0, |(_, level)| level)
    }

    fn sale_item(&self, source: ItemRef) -> Result<&crate::game::inventory::ItemStack, TradeError> {
        match source.owner {
            ItemOwner::Pool => self.party.0.pool.stacks.iter().find(|i| i.id == source.id),
            ItemOwner::Member(member) => self.packs.get(member).unwrap().backpack_stack(source.id),
        }
        .ok_or(TradeError::StaleSelection)
    }

    /// What the party would pay or get for `quantity` of a stock lot or one
    /// of its own items, now.
    pub fn quote(
        &self,
        conversation: Entity,
        source: TradeSource,
        quantity: u32,
    ) -> Result<TradeQuote, TradeError> {
        if !self.entering.is_empty() || !self.battles.is_empty() {
            return Err(TradeError::Busy);
        }
        let (c, _) =
            self.conversations.get(conversation).map_err(|_| TradeError::StaleSelection)?;
        if !c.ready() {
            return Err(TradeError::Busy);
        }
        let actor = c.actor;
        if !self.open_for_trade(actor) {
            return Err(TradeError::Unavailable);
        }
        let (npc, id, _) = self.npcs.get(actor).unwrap();
        let def = &self.content.npcs[id.npc()];
        let catalog = &self.content.items;
        let communication = self.communication();
        let gold = match source {
            TradeSource::Stock(lot) => trade::purchase_price(
                npc.stock.lots.iter().find(|l| l.id == lot).ok_or(TradeError::StaleSelection)?,
                def,
                catalog,
                quantity,
                communication,
            ),
            TradeSource::Party(source) => {
                let item = self.sale_item(source)?;
                trade::validate_sale(def, item, quantity, catalog, communication)
            }
        }?;
        Ok(TradeQuote { conversation, trade: npc.stock.trades, source, quantity, gold })
    }

    /// Settle a quoted trade, if nothing it was quoted on has changed since.
    fn settle(&mut self, quote: TradeQuote) -> Result<(), TradeError> {
        let (c, _) =
            self.conversations.get(quote.conversation).map_err(|_| TradeError::StaleSelection)?;
        if !c.ready() {
            return Err(TradeError::StaleSelection);
        }
        let actor = c.actor;
        if !self.entering.is_empty() || !self.battles.is_empty() || !self.open_for_trade(actor) {
            return Err(TradeError::Unavailable);
        }
        if self.npcs.get(actor).unwrap().0.stock.trades != quote.trade {
            return Err(TradeError::StaleSelection);
        }
        let communication = self.communication();
        let content = &self.content;
        let catalog = &content.items;
        let (mut npc, id, _) = self.npcs.get_mut(actor).unwrap();
        let def = &content.npcs[id.npc()];
        match quote.source {
            TradeSource::Stock(lot) => trade::buy(
                &mut npc.stock,
                &mut self.party.0,
                &mut self.ids,
                def,
                catalog,
                lot,
                quote.quantity,
                quote.gold,
                communication,
            )?,
            TradeSource::Party(source) => {
                let units =
                    u16::try_from(quote.quantity).map_err(|_| TradeError::InvalidQuantity)?;
                let item = match source.owner {
                    ItemOwner::Pool => self.party.0.pool.stacks.iter().find(|i| i.id == source.id),
                    ItemOwner::Member(member) => {
                        self.packs.get(member).unwrap().backpack_stack(source.id)
                    }
                }
                .ok_or(TradeError::StaleSelection)?;
                if trade::validate_sale(def, item, quote.quantity, catalog, communication)?
                    != quote.gold
                {
                    return Err(TradeError::PriceChanged);
                }
                npc.stock.add(item.item, quote.quantity, catalog, false);
                match source.owner {
                    ItemOwner::Pool => self.party.0.pool.consume_units(source.id, units),
                    ItemOwner::Member(member) => {
                        self.packs.get_mut(member).unwrap().consume_backpack_units(source.id, units)
                    }
                }
                self.party.0.gold += quote.gold;
            }
        }
        npc.stock.trades += 1;
        Ok(())
    }
}

/// The party settles a quoted trade.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct TradeRequest(pub TradeQuote);

/// Settle the party's trades and tell its conversation how each went. A
/// dialogue the party has left takes none, and none happen in a camp.
pub fn settle_trades(mut requests: MessageReader<TradeRequest>, mut trading: Trading) {
    for &TradeRequest(quote) in requests.read() {
        let Ok((_, closing)) = trading.conversations.get(quote.conversation) else { continue };
        if closing {
            continue;
        }
        let outcome = if trading.camp.is_some() {
            Outcome::TradeRefused(TradeError::Unavailable)
        } else {
            match trading.settle(quote) {
                Ok(()) => Outcome::Traded,
                Err(e) => Outcome::TradeRefused(e),
            }
        };
        let (mut c, _) = trading.conversations.get_mut(quote.conversation).unwrap();
        c.result = None;
        c.report(outcome);
    }
}

/// [`Trading::quote`], for operations that hold the whole world.
pub fn quote_trade(
    world: &mut World,
    conversation: Entity,
    source: TradeSource,
    quantity: u32,
) -> Result<TradeQuote, TradeError> {
    world
        .run_system_cached_with(
            |In((conversation, source, quantity)): In<(Entity, TradeSource, u32)>,
             trading: Trading| { trading.quote(conversation, source, quantity) },
            (conversation, source, quantity),
        )
        .unwrap()
}
