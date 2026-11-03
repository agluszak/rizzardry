//! Social choices, gifts and paid answers. Rules take explicit values; these
//! systems borrow only the state required to accept their particular choice.
use super::script::{self, quotes};
use super::*;
use crate::content::{
    items::{AMMUNITION_BUNDLE, EquipClass, ItemDefinition, ItemId, QuantityKind},
    npcs::*,
};
use crate::game::npcs::access::Scripts;
use crate::game::{
    characters::{Character, Vitals},
    factions::{Band, Factions},
    inventory::{Carried, ItemOwner, PartyInventory},
    npcs::{Npc, access::NpcActors, trade},
    party::roster::PartyRoster,
    time::GameClock,
};
use bevy::ecs::system::SystemParam;

#[derive(SystemParam)]
pub(super) struct PartyVoices<'w, 's> {
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals)>,
}
impl PartyVoices<'_, '_> {
    fn members(&self) -> impl Iterator<Item = (Entity, &Character, &Vitals)> {
        self.roster.iter().filter_map(|m| self.members.get(m).ok().map(|(c, v)| (m, c, v)))
    }
    pub fn communication(&self) -> u32 {
        trade::communication(self.members().map(|(_, c, v)| (c, v)))
    }
    pub fn recipients(&self) -> Vec<Entity> {
        self.members().filter(|(_, _, v)| !v.is_dead()).map(|(m, _, _)| m).collect()
    }
    fn best(&self) -> Option<(Entity, u32)> {
        self.members().filter(|(_, _, v)| v.can_act()).fold(None, |best, (m, c, _)| {
            let skill = c.skill(crate::content::characters::Skill::Communication);
            if best.is_none_or(|(_, s)| skill > s) { Some((m, skill)) } else { best }
        })
    }
    fn average(&self) -> u32 {
        crate::game::party::average_level(
            self.members().filter(|(_, _, v)| v.can_fight()).map(|(_, c, _)| c.level()),
        )
    }
}

pub(super) fn band(
    own: u8,
    charm: u32,
    turned: bool,
    def: &NpcDefinition,
    factions: &Factions,
) -> Band {
    let own = if turned { 100 } else { (u32::from(own) + charm).min(100) as i32 };
    let faction = i32::from(def.faction);
    let score =
        if def.faction != 0 && (def.monster_bound || factions.band(faction) == Band::Hostile) {
            factions.score(faction).map_or(own, i32::from)
        } else {
            own
        };
    match score {
        ..=32 => Band::Hostile,
        33..=65 => Band::Neutral,
        _ => Band::Friendly,
    }
}
pub(super) fn greeting(def: &NpcDefinition, npc: &Npc) -> (u16, bool) {
    let first = def.merchant || npc.greeting_pending;
    (
        if first { quotes::FIRST_GREETING } else { quotes::RETURN_GREETING },
        first && !def.monster_bound && !def.voice_script && !def.merchant,
    )
}
pub(super) fn shift(npc: &mut Npc, delta: i8) {
    npc.disposition = (i16::from(npc.disposition) + i16::from(delta)).clamp(0, 99) as u8;
}
pub fn approach_ready(at: Option<u64>, now: u64) -> bool {
    at.is_none_or(|at| now.saturating_sub(at) > 0xa8c0 * 1000)
}
fn debit(pool: u16, own: u8, refill: u16, cold: bool, price: u32, skill: u32) -> (u16, u8) {
    let adjusted = u64::from(price) + u64::from(price) * u64::from(skill) / 500;
    match u64::from(pool).saturating_sub(adjusted) as u16 {
        0 => (refill, if cold { 0x4b } else { own }),
        left => (left, own),
    }
}
fn value(item: &ItemDefinition, count: u16) -> u32 {
    if item.class == EquipClass::Ammunition {
        (item.value * u32::from(count)).div_ceil(AMMUNITION_BUNDLE)
    } else if item.quantity == QuantityKind::Stack {
        item.value * u32::from(count).max(1)
    } else {
        item.value
    }
}

pub(super) fn welcome(
    In((entity, say_hello)): In<(Entity, bool)>,
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    npcs: Query<&Npc>,
    factions: Res<Factions>,
) -> bool {
    let Ok(mut c) = conversations.get_mut(entity) else { return false };
    let npc = npcs.get(c.actor).unwrap();
    let def = actors.definition(c.actor);
    if !c.menu
        || band(npc.disposition, c.charm(), actors.turned(c.actor), def, &factions)
            != Band::Friendly
    {
        return false;
    }
    c.menu = false;
    let (quote, practice) = greeting(def, npc);
    if say_hello {
        c.queue.push_back(Step::quote(quote));
    }
    say_hello && practice
}

pub(super) fn approach(
    In((entity, how)): In<(Entity, Approach)>,
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    voices: PartyVoices,
    clock: Res<GameClock>,
) -> (Option<(Entity, u32)>, bool) {
    let Ok(mut c) = conversations.get_mut(entity) else { return (None, false) };
    let mut npc = npcs.get_mut(c.actor).unwrap();
    let def = actors.definition(c.actor);
    let (at, scale, usage) = match how {
        Approach::Charm => (npc.charmed_at, def.charm_scale, 5),
        Approach::SmallTalk => (npc.small_talk_at, def.talk_scale, 8),
    };
    if !c.ready() || !c.menu || !approach_ready(at, clock.ms) {
        return (None, false);
    }
    let Some((member, skill)) = voices.best() else { return (None, false) };
    let scale = i32::from(scale);
    let mut delta = scale + skill as i32 * (scale.abs() / 5) / 100;
    if how == Approach::Charm && delta > 0 {
        delta = (voices.average() * delta as u32 / actors.species_level(c.actor)) as i32;
    }
    shift(&mut npc, delta as i8);
    match how {
        Approach::Charm => npc.charmed_at = Some(clock.ms),
        Approach::SmallTalk => npc.small_talk_at = Some(clock.ms),
    }
    let standing = band(npc.disposition, c.charm(), actors.turned(c.actor), def, &factions);
    let (quote, practice) = match (standing, how) {
        (Band::Friendly, Approach::Charm) => (quotes::CHARMED, false),
        (Band::Friendly, Approach::SmallTalk) => greeting(def, &npc),
        (Band::Neutral, Approach::Charm) => (quotes::UNCHARMED, false),
        (Band::Neutral, Approach::SmallTalk) => (quotes::SMALL_TALK, false),
        (Band::Hostile, _) => (quotes::HOSTILE, false),
    };
    c.result = None;
    c.menu = standing != Band::Friendly;
    c.queue.push_back(Step::quote(quote));
    if standing == Band::Hostile {
        c.queue.push_back(Step::End);
    }
    (Some((member, usage)), practice)
}

pub(super) fn offer(
    In((entity, source)): In<(Entity, ItemRef)>,
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    scripts: Scripts,
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    voices: PartyVoices,
    story: Res<StoryFacts>,
    mut inventory: Single<&mut PartyInventory>,
    mut packs: Query<&mut Carried>,
) -> Result<(), TradeError> {
    let mut c = conversations.get_mut(entity).map_err(|_| TradeError::StaleSelection)?;
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    let pack = match source.owner {
        ItemOwner::Pool => None,
        ItemOwner::Member(member) => {
            if !voices.roster.contains(&member) {
                return Err(TradeError::StaleSelection);
            }
            packs.get(member).ok()
        }
    };
    let stack = trade::sale_item(source, &inventory, pack)?;
    let item = &actors.content.items[stack.item];
    let def = actors.definition(c.actor);
    let mut npc = npcs.get_mut(c.actor).unwrap();
    let standing = |own| band(own, c.charm(), actors.turned(c.actor), def, &factions);
    let bribe = !def.voice_script && standing(npc.disposition) != Band::Friendly;
    let answer = scripts.get(c.actor).item_quote(stack.item);
    let worth = value(item, stack.count());
    let mut debited = None;
    let (quote, keeps) = match answer {
        _ if bribe && item.crucial => (quotes::REFUSED_GIFT, false),
        _ if bribe && (def.kind == 20 || (stack.item != ItemId(0x29f) && worth < 300)) => {
            (quotes::UNBRIBED, false)
        }
        Some((q, _)) if bribe => (q, true),
        None if bribe => {
            let next = debit(
                npc.trade_pool,
                npc.disposition,
                def.trade_pool,
                true,
                worth,
                voices.communication(),
            );
            debited = Some(next);
            (
                if standing(next.1) == Band::Friendly {
                    quotes::ACCEPTED_GIFT
                } else {
                    quotes::UNBRIBED
                },
                true,
            )
        }
        Some(answer) => answer,
        None if def.voice_script => return Ok(()),
        None if item.crucial => (quotes::REFUSED_GIFT, false),
        None => (quotes::ACCEPTED_GIFT, true),
    };
    let prepared = scripts.get(c.actor).quotes.get(usize::from(quote)).and_then(|q| {
        script::plan(q, &story, &actors.content.facts, voices.recipients().into_iter(), |name| {
            actors.actor_shown_as(name, &story)
        })
        .ok()
    });
    if keeps && prepared.is_some() {
        match source.owner {
            ItemOwner::Pool => inventory.pool.consume_units(source.id, 1),
            ItemOwner::Member(member) => {
                packs.get_mut(member).unwrap().consume_backpack_units(source.id, 1)
            }
        }
        if let Some((pool, own)) = debited {
            npc.trade_pool = pool;
            npc.disposition = own;
        }
    }
    c.queue.push_back(Step::Quote { quote, line: 0, paid: false, plan: prepared });
    Ok(())
}

/// Both giving money and confirming an all-gold offer use this operation.
pub(super) fn gold(
    In((entity, amount, confirmed)): In<(Entity, u32, bool)>,
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
    voices: PartyVoices,
    mut inventory: Single<&mut PartyInventory>,
) -> Result<(), TradeError> {
    let mut c = conversations.get_mut(entity).map_err(|_| TradeError::StaleSelection)?;
    if confirmed {
        if c.wait != (ConversationWait::GiveAll { gold: amount }) {
            return Err(TradeError::StaleSelection);
        }
        c.wait = ConversationWait::Ready;
    } else if !c.ready() {
        return Err(TradeError::Busy);
    }
    if amount == 0 {
        return Err(TradeError::InvalidQuantity);
    }
    if amount > inventory.gold {
        return Err(TradeError::Funds);
    }
    if !confirmed && amount == inventory.gold {
        c.wait = ConversationWait::GiveAll { gold: amount };
        return Ok(());
    }
    let def = actors.definition(c.actor);
    let mut npc = npcs.get_mut(c.actor).unwrap();
    let standing = |own| band(own, c.charm(), actors.turned(c.actor), def, &factions);
    let bribe = standing(npc.disposition) != Band::Friendly;
    if bribe && def.trade_pool == 0 {
        c.queue.push_back(Step::paid(quotes::UNBRIBED, None));
        return Ok(());
    }
    inventory.gold -= amount;
    if bribe {
        let (pool, own) = debit(
            npc.trade_pool,
            npc.disposition,
            def.trade_pool,
            true,
            amount,
            voices.communication(),
        );
        npc.trade_pool = pool;
        npc.disposition = own;
    }
    let thanks = standing(npc.disposition) == Band::Friendly;
    c.queue
        .push_back(Step::paid(if thanks { quotes::ACCEPTED_GIFT } else { quotes::UNBRIBED }, None));
    Ok(())
}

pub(super) fn price(
    In((entity, accept)): In<(Entity, bool)>,
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    scripts: Scripts,
    mut npcs: Query<&mut Npc>,
    voices: PartyVoices,
    story: Res<StoryFacts>,
    mut inventory: Single<&mut PartyInventory>,
) {
    let Ok(mut c) = conversations.get_mut(entity) else { return };
    let ConversationWait::Price { quote, gold, always } = c.wait else { return };
    let Some(q) = scripts.get(c.actor).quotes.get(usize::from(quote)) else { return };
    if !accept {
        for op in &q.operations {
            if let DialogueOp::Decline(q) = op {
                c.queue.push_back(Step::quote(*q));
            }
        }
    } else if inventory.gold < gold {
        c.queue.push_back(Step::paid(quotes::TOO_POOR, None));
    } else {
        let prepared = match script::plan(
            q,
            &story,
            &actors.content.facts,
            voices.recipients().into_iter(),
            |name| actors.actor_shown_as(name, &story),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                c.block(quote, error);
                return;
            }
        };
        inventory.gold -= gold;
        if !always {
            npcs.get_mut(c.actor).unwrap().paid_topics.push(quote);
        }
        c.queue.push_back(Step::paid(quote, Some(prepared)));
    }
    c.wait = ConversationWait::Ready;
}
