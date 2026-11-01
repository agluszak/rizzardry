//! What buying from and selling to an NPC in conversation would come to.
//! [`Dialogue`] settles what is quoted here.
use super::*;

/// Where an NPC stands in the played level: the body of its that is
/// there and alive.
#[derive(bevy::ecs::system::SystemParam)]
pub struct NpcPresence<'w, 's> {
    bodies: Query<'w, 's, (&'static Monster, &'static LevelOwned)>,
    levels: Query<'w, 's, (), With<Level>>,
    sides: crate::game::world::hostility::Sides<'w, 's>,
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

    /// The NPC stands in the played level, alive and not hostile.
    pub(super) fn available(
        &self,
        bodies: Option<&crate::game::npcs::NpcBodies>,
        species: &crate::content::species::MonsterCatalog,
        factions: &Factions,
        now: u64,
    ) -> bool {
        self.body(bodies).is_some_and(|body| {
            self.sides.own(body, species, factions, now)
                != Some(crate::game::world::hostility::Stance::Hostile)
        })
    }
}

/// The party's members as trades and conversations read them.
pub(super) type Members<'w, 's> =
    Query<'w, 's, (&'static Character, &'static Vitals, &'static Carried)>;

/// The member with the best Communication among those who can act, the first
/// on a tie, and that level (retail `GetBestPartySkillLevel`:
/// `hp_current != 0 && highest_condition < TURNCOAT`).
pub(super) fn best_communicator(roster: &PartyRoster, members: &Members) -> Option<(Entity, u32)> {
    let mut best: Option<(Entity, u32)> = None;
    for member in roster.iter() {
        let (character, vitals, _) = members.get(member).unwrap();
        let level = character.skill(crate::content::characters::Skill::Communication);
        if vitals.can_act() && best.is_none_or(|(_, best)| level > best) {
            best = Some((member, level));
        }
    }
    best
}

/// One of the party's own items, in the pool or a member's pack.
pub(super) fn sale_item<'a>(
    inventory: &'a PartyInventory,
    members: Members<'a, '_>,
    source: ItemRef,
) -> Result<&'a crate::game::inventory::ItemStack, TradeError> {
    match source.owner {
        ItemOwner::Pool => inventory.pool.stacks.iter().find(|i| i.id == source.id),
        ItemOwner::Member(member) => members.get_inner(member).unwrap().2.backpack_stack(source.id),
    }
    .ok_or(TradeError::StaleSelection)
}

/// What `quantity` of a stock lot or one of the party's own items fetches now.
pub(super) fn price_of(
    (npc, def): (&Npc, &NpcDefinition),
    (inventory, roster): (&PartyInventory, &PartyRoster),
    members: Members,
    catalog: &crate::content::items::ItemCatalog,
    source: TradeSource,
    quantity: u32,
) -> Result<u32, TradeError> {
    let communication = best_communicator(roster, &members).map_or(0, |(_, level)| level);
    match source {
        TradeSource::Stock(lot) => trade::purchase_price(
            npc.stock.lots.iter().find(|l| l.id == lot).ok_or(TradeError::StaleSelection)?,
            def,
            catalog,
            quantity,
            communication,
        ),
        TradeSource::Party(source) => {
            let item = sale_item(inventory, members, source)?;
            trade::validate_sale(def, item, quantity, catalog, communication)
        }
    }
}

/// What quoting a trade reads: the conversation and its NPC, the NPC's
/// presence and stance, the party's purse, pool and packs, and the catalogs.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Trading<'w, 's> {
    conversations: Query<'w, 's, &'static Conversation>,
    npcs: Query<
        'w,
        's,
        (
            &'static Npc,
            &'static crate::game::npcs::ActorId,
            Option<&'static crate::game::npcs::NpcBodies>,
        ),
    >,
    presence: NpcPresence<'w, 's>,
    factions: Res<'w, Factions>,
    party: Single<'w, 's, (&'static PartyInventory, &'static PartyRoster)>,
    members: Members<'w, 's>,
    content: If<Res<'w, Catalogs>>,
    clock: Res<'w, GameClock>,
    freedom: crate::game::freedom::Freedom<'w, 's>,
}

impl Trading<'_, '_> {
    /// Whether the conversation stands ready for the party to trade in.
    pub fn ready(&self, conversation: Entity) -> bool {
        self.conversations.get(conversation).is_ok_and(|c| c.ready())
    }

    /// The party's gold.
    pub fn gold(&self) -> u32 {
        self.party.0.gold
    }

    /// The item a quote is of and how many of it there are to trade.
    pub fn units(&self, quote: &TradeQuote) -> Option<(ItemId, u32)> {
        let c = self.conversations.get(quote.conversation).ok()?;
        match quote.source {
            TradeSource::Stock(id) => {
                let stock = &self.npcs.get(c.actor).ok()?.0.stock;
                stock.lots.iter().find(|lot| lot.id == id).map(|lot| (lot.item, lot.units))
            }
            TradeSource::Party(item) => sale_item(self.party.0, self.members.as_readonly(), item)
                .ok()
                .map(|stack| (stack.item, stack.units())),
        }
    }

    /// What the party would pay or get for `quantity` of a stock lot or one
    /// of its own items, now.
    pub fn quote(
        &self,
        conversation: Entity,
        source: TradeSource,
        quantity: u32,
    ) -> Result<TradeQuote, TradeError> {
        if !self.freedom.peaceful() {
            return Err(TradeError::Busy);
        }
        let c = self.conversations.get(conversation).map_err(|_| TradeError::StaleSelection)?;
        if !c.ready() {
            return Err(TradeError::Busy);
        }
        let (npc, id, bodies) = self.npcs.get(c.actor).unwrap();
        let content = &self.content;
        let def = &content.npcs[id.npc()];
        if !self.presence.available(bodies, &content.species, &self.factions, self.clock.ms)
            || !def.owns_stock
        {
            return Err(TradeError::Unavailable);
        }
        let gold = price_of(
            (npc, def),
            (self.party.0, self.party.1),
            self.members.as_readonly(),
            &content.items,
            source,
            quantity,
        )?;
        Ok(TradeQuote { conversation, trade: npc.stock.trades, source, quantity, gold })
    }
}
