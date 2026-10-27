//! Authored continuation and waits belong to gameplay, not the dialogue view.
use super::{
    Npc, NpcAssets,
    trade::{self, TradeError, TradeQuote, TradeSource},
};
use crate::content::{items::ItemCatalog, npcs::*};
use crate::game::{
    characters::{Character, Vitals},
    data::GameData,
    inventory::{Carried, ItemIds, PartyInventory},
    party::roster::PartyRoster,
    rng::GameRng,
    world::{ActorPose, level::Level, population::Monster},
};
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use std::collections::{BTreeMap, VecDeque};

/// Engine semantics refer to the database's symbolic keys; script operands and
/// campaign storage keep the actual decoded identifiers. No Burz record or
/// fact number is embedded in the interpreter.
#[derive(Resource, Debug, Default)]
pub struct StoryFacts {
    values: BTreeMap<FactId, u8>,
    pub topics: Vec<(TopicCategory, String)>,
    pub journal: Vec<String>,
    initialized: bool,
}
impl StoryFacts {
    pub fn initialize(&mut self, catalog: &FactCatalog) {
        if self.initialized {
            return;
        }
        for fact in &catalog.0 {
            if matches!(
                fact.symbol.as_str(),
                "FACT_VIRGIN"
                    | "FACT_QUEST_VIRGIN_1"
                    | "FACT_QUEST_VIRGIN_2"
                    | "FACT_QUEST_VIRGIN_3"
            ) {
                self.values.insert(fact.id, 1);
            }
        }
        self.initialized = true;
    }
    pub fn get(&self, id: FactId, catalog: &FactCatalog) -> Result<u8, String> {
        let fact = catalog.get(id).ok_or_else(|| format!("missing fact definition {id:?}"))?;
        match fact.symbol.as_str() {
            "FACT_VIRGIN" | "FACT_IMPORT" | "FACT_BURZ_SAID130" => {
                Ok(*self.values.get(&id).unwrap_or(&0))
            }
            name => Err(format!("unported fact predicate {name}")),
        }
    }
    fn can_set(id: FactId, catalog: &FactCatalog) -> bool {
        catalog.get(id).is_some_and(|f| f.symbol == "FACT_BURZ_SAID130")
    }
    fn set(&mut self, id: FactId, value: u8, catalog: &FactCatalog) -> Result<(), String> {
        if !Self::can_set(id, catalog) {
            return Err(format!("unported fact assignment {id:?}"));
        }
        let previous = self.values.insert(id, value).unwrap_or(0);
        if previous != value
            && let Some(fact) = catalog.get(id)
        {
            let text = if value == 0 { &fact.alternate_description } else { &fact.description };
            if !text.is_empty() {
                self.journal.push(text.clone());
            }
        }
        Ok(())
    }
}
pub fn initialize_facts(
    assets: Res<NpcAssets>,
    catalogs: Res<Assets<FactCatalog>>,
    mut facts: ResMut<StoryFacts>,
) {
    if let Some(catalog) = catalogs.get(&assets.facts) {
        facts.initialize(catalog);
    }
}
fn fact_catalog(world: &World) -> Option<&FactCatalog> {
    let assets = world.resource::<NpcAssets>();
    world.resource::<Assets<FactCatalog>>().get(&assets.facts)
}
fn fact(world: &World, id: FactId) -> Result<u8, String> {
    world
        .resource::<StoryFacts>()
        .get(id, fact_catalog(world).ok_or_else(|| "fact database unavailable".to_string())?)
}
#[derive(Clone, Debug)]
enum Step {
    Quote { quote: u16, subquote: usize, paid: bool },
    Options { quote: u16, words: Vec<String> },
    Price { quote: u16, gold: u32, always: bool },
    End,
}
#[derive(Clone, Debug, PartialEq)]
pub enum LinePhase {
    Text { remaining: f32 },
    Audio,
    Hold { remaining: f32 },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationWait {
    Ready,
    Line { speech: Entity, phase: LinePhase },
    Options { quote: u16, words: Vec<String> },
    Price { quote: u16, gold: u32, always: bool },
    Blocked { quote: u16, entry: Option<usize>, reason: String },
}
#[derive(Component, Debug)]
pub struct Conversation {
    pub actor: Entity,
    pub script: Handle<DialogueScript>,
    pub revision: u64,
    pub current: Option<(u16, usize)>,
    pub wait: ConversationWait,
    pub transcript: Vec<String>,
    pub result: Option<String>,
    queue: VecDeque<Step>,
}
impl Conversation {
    pub fn ready(&self) -> bool {
        self.wait == ConversationWait::Ready && self.queue.is_empty()
    }
}
/// Presentation may attach audio to this identity. No artwork/audio is required
/// to progress: the default wait is the original text-duration fallback.
#[derive(Component, Debug)]
pub struct ConversationSpeech {
    pub voice: String,
    pub text: String,
}
#[derive(Clone, Copy, Debug)]
pub enum PlaybackEvent {
    Started,
    Finished,
    Unavailable,
}
#[derive(Message, Clone, Copy, Debug)]
pub struct LinePlayback {
    pub speech: Entity,
    pub event: PlaybackEvent,
}
#[derive(Message, Clone, Debug)]
pub enum ConversationRequest {
    Begin { actor: Entity },
    Topic { conversation: Entity, revision: u64, text: String },
    Reply { conversation: Entity, revision: u64, text: String },
    PriceAnswer { conversation: Entity, revision: u64, accept: bool },
    Advance { conversation: Entity, speech: Entity },
    Close { conversation: Entity },
    Trade(TradeQuote),
}
fn definition(world: &World, actor: Entity) -> Result<NpcDefinition, TradeError> {
    let npc = world.get::<Npc>(actor).ok_or(TradeError::Unavailable)?;
    let assets = world.resource::<NpcAssets>();
    world
        .resource::<Assets<NpcCatalog>>()
        .get(&assets.catalog)
        .and_then(|c| c.get(npc.definition))
        .cloned()
        .ok_or(TradeError::MissingContent)
}
fn with_items<T>(
    world: &mut World,
    f: impl FnOnce(&mut World, &ItemCatalog) -> Result<T, TradeError>,
) -> Result<T, TradeError> {
    let handle = world.get_resource::<GameData>().ok_or(TradeError::MissingContent)?.items.clone();
    world.resource_scope(|world, catalogs: Mut<Assets<ItemCatalog>>| {
        f(world, catalogs.get(&handle).ok_or(TradeError::MissingContent)?)
    })
}
pub fn party_communication(world: &World) -> u32 {
    let Some(roster) = world.get_resource::<PartyRoster>() else { return 0 };
    roster
        .0
        .iter()
        .filter_map(|e| Some((world.get::<Character>(*e)?, world.get::<Vitals>(*e)?)))
        .filter(|(_, v)| crate::game::camp::can_act(v))
        .map(|(c, _)| c.skill(crate::content::characters::Skill::Communication))
        .max()
        .unwrap_or(0)
}
fn average_level(world: &World) -> u32 {
    let roster = world.resource::<PartyRoster>();
    let (sum, count) = roster
        .0
        .iter()
        .filter_map(|e| world.get::<Character>(*e))
        .fold((0, 0), |(s, n), c| (s + u32::from(c.level), n + 1));
    sum.checked_div(count).unwrap_or(1)
}
fn actor_level(world: &World, actor: Entity) -> Option<Entity> {
    world
        .get::<super::NpcPresence>(actor)
        .map(|p| p.0)
        .or_else(|| world.get::<ChildOf>(actor).map(ChildOf::parent))
}
fn available(world: &World, actor: Entity) -> bool {
    world.get::<Monster>(actor).is_some_and(|m| !m.is_dead())
        && world.get::<crate::game::world::population::Provoked>(actor).is_none()
        && world.get::<Npc>(actor).is_some_and(|n| n.disposition >= 0x42)
        && actor_level(world, actor).is_some_and(|level| world.get::<Level>(level).is_some())
}
pub fn begin(world: &mut World, actor: Entity) -> Result<Entity, TradeError> {
    if !world.run_system_once(crate::game::screen::party_free).map_err(|_| TradeError::Busy)?
        || world.contains_resource::<crate::game::combat::Battle>()
    {
        return Err(TradeError::Busy);
    }
    if world.resource::<PartyInventory>().hand.held.is_some() {
        return Err(TradeError::Busy);
    }
    let monster = world.get::<Monster>(actor).ok_or(TradeError::Unavailable)?;
    if monster.is_dead() {
        return Err(TradeError::Unavailable);
    }
    let parent = actor_level(world, actor).ok_or(TradeError::Unavailable)?;
    if world.get::<Level>(parent).is_none() {
        return Err(TradeError::Unavailable);
    }
    // Recheck spatial reach from authoritative actor/party poses, not a UI claim.
    let at = world
        .run_system_once(move |q: Query<ActorPose>| q.get(actor).ok().map(|p| p.translation()))
        .ok()
        .flatten()
        .ok_or(TradeError::Unavailable)?;
    let near = world
        .query::<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>()
        .iter(world)
        .any(|(p, _)| p.0.distance(at) <= crate::game::world::items::REACH);
    if !near {
        return Err(TradeError::Unavailable);
    }
    let def = definition(world, actor)?;
    if !available(world, actor) {
        return Err(TradeError::Unavailable);
    }
    let npc = world.get::<Npc>(actor).unwrap();
    let script = world
        .resource::<NpcAssets>()
        .scripts
        .get(&npc.definition)
        .cloned()
        .ok_or(TradeError::MissingContent)?;
    if !world.resource::<Assets<DialogueScript>>().contains(&script) {
        return Err(TradeError::MissingContent);
    }
    let now = world.resource::<crate::game::time::GameClock>().ms;
    let average = average_level(world);
    if def.owns_stock {
        with_items(world, |world, items| {
            world.resource_scope(|world, mut rng: Mut<GameRng>| {
                world
                    .get_mut::<Npc>(actor)
                    .unwrap()
                    .stock
                    .maintain(&def, items, average, now, &mut rng)
            })
        })?;
    }
    let facts_handle = world.resource::<NpcAssets>().facts.clone();
    world.resource_scope(|world, catalogs: Mut<Assets<FactCatalog>>| {
        let catalog = catalogs.get(&facts_handle).ok_or(TradeError::MissingContent)?;
        world.resource_mut::<StoryFacts>().initialize(catalog);
        Ok::<_, TradeError>(())
    })?;
    let greeting = if world.get::<Npc>(actor).unwrap().greeting_pending { 0 } else { 1 };
    Ok(world
        .spawn(Conversation {
            actor,
            script,
            revision: 0,
            current: None,
            wait: ConversationWait::Ready,
            transcript: vec![],
            result: None,
            queue: VecDeque::from([Step::Quote { quote: greeting, subquote: 0, paid: false }]),
        })
        .id())
}
pub fn close(world: &mut World) {
    let sessions: Vec<_> =
        world.query_filtered::<Entity, With<Conversation>>().iter(world).collect();
    for entity in sessions {
        close_one(world, entity)
    }
}
fn close_one(world: &mut World, entity: Entity) {
    if world.get::<Conversation>(entity).is_some() {
        world.despawn(entity);
    }
}

fn select_response(response: &Response, rng: &mut GameRng) -> u16 {
    match *response {
        Response::Fixed(n) => n,
        Response::Random { first, last } => first + rng.below(u32::from(last - first) + 1) as u16,
    }
}
fn block(c: &mut Conversation, quote: u16, entry: Option<usize>, reason: String) {
    c.wait = ConversationWait::Blocked { quote, entry, reason };
}
fn script(world: &World, c: &Conversation) -> Option<DialogueScript> {
    world.resource::<Assets<DialogueScript>>().get(&c.script).cloned()
}
/// Preflight the operations that actually execute, up to the first authored
/// control transfer. Unsupported work cannot be skipped after partial effects.
fn validate_line(world: &World, q: &Quote) -> Result<(), (usize, String)> {
    if q.alert {
        return Err((0, "unported alert presentation".into()));
    }
    if q.text
        .iter()
        .any(|text| ["UNKNOWN", "CLASSIFIED", "SOUND"].iter().any(|s| text.eq_ignore_ascii_case(s)))
    {
        return Err((0, "unported quote sentinel".into()));
    }
    for (i, op) in q.operations.iter().enumerate() {
        match op {
            DialogueOp::Unsupported { opcode, .. } => {
                return Err((i, format!("unported mandatory opcode {opcode}")));
            }
            DialogueOp::SetFact(id, _)
                if !fact_catalog(world)
                    .is_some_and(|catalog| StoryFacts::can_set(*id, catalog)) =>
            {
                return Err((i, format!("unported fact assignment {id:?}")));
            }
            DialogueOp::Conditional { facts, .. } => {
                for (id, _) in facts {
                    fact(world, *id).map_err(|e| (i, e))?;
                }
            }
            _ => {}
        }
        match op {
            DialogueOp::Next(_) | DialogueOp::Options(_) => break,
            DialogueOp::Conditional { facts, .. }
                if facts.iter().all(|(id, v)| fact(world, *id) == Ok(*v)) =>
            {
                break;
            }
            _ => {}
        }
    }
    Ok(())
}
fn run(world: &mut World, entity: Entity, c: &mut Conversation) {
    let Some(script) = script(world, c) else {
        block(c, 0, None, "script unavailable".into());
        return;
    };
    // Bound immediate authored transitions, including malicious empty-quote loops.
    for _ in 0..256 {
        if c.wait != ConversationWait::Ready {
            return;
        }
        let Some(step) = c.queue.pop_front() else { return };
        match step {
            Step::End => {
                c.queue.push_front(Step::End);
                return;
            }
            Step::Options { quote, words } => {
                c.wait = ConversationWait::Options { quote, words };
                c.revision += 1;
                return;
            }
            Step::Price { quote, gold, always } => {
                c.wait = ConversationWait::Price { quote, gold, always };
                c.revision += 1;
                return;
            }
            Step::Quote { quote, subquote, paid } => {
                let Some(q) = script.quotes.get(quote as usize) else {
                    block(c, quote, None, "missing quote".into());
                    return;
                };
                if !paid
                    && subquote == 0
                    && let Some((gold, always)) = q.operations.iter().find_map(|op| {
                        if let DialogueOp::Price { gold, always } = op {
                            Some((*gold, *always))
                        } else {
                            None
                        }
                    })
                    && (always || !world.get::<Npc>(c.actor).unwrap().paid_topics.contains(&quote))
                {
                    let gold =
                        (gold - gold.saturating_mul(party_communication(world)) / 500).max(1);
                    c.queue.push_front(Step::Price { quote, gold, always });
                    c.queue.push_front(Step::Quote { quote: 0x12, subquote: 0, paid: true });
                    continue;
                }
                if q.alert {
                    block(c, quote, None, "alert quote presentation is not ported".into());
                    return;
                }
                if let Err((entry, reason)) = validate_line(world, q) {
                    block(c, quote, Some(entry), reason);
                    return;
                }
                if quote == 0 {
                    world.get_mut::<Npc>(c.actor).unwrap().greeting_pending = false;
                }
                c.current = Some((quote, subquote));
                if subquote + 1 >= q.text.len() {
                    let mut following = None;
                    for op in &q.operations {
                        match op {
                            DialogueOp::Next(r) => {
                                following = Some(Step::Quote {
                                    quote: select_response(r, &mut world.resource_mut::<GameRng>()),
                                    subquote: 0,
                                    paid: false,
                                });
                                break;
                            }
                            DialogueOp::Conditional { facts, response }
                                if facts.iter().all(|(id, v)| fact(world, *id) == Ok(*v)) =>
                            {
                                following = Some(Step::Quote {
                                    quote: select_response(
                                        response,
                                        &mut world.resource_mut::<GameRng>(),
                                    ),
                                    subquote: 0,
                                    paid: false,
                                });
                                break;
                            }
                            DialogueOp::Options(words) => {
                                following = Some(Step::Options { quote, words: words.clone() });
                                break;
                            }
                            DialogueOp::SetFact(id, value) => {
                                let facts_handle = world.resource::<NpcAssets>().facts.clone();
                                world.resource_scope(
                                    |world, catalogs: Mut<Assets<FactCatalog>>| {
                                        world
                                            .resource_mut::<StoryFacts>()
                                            .set(*id, *value, catalogs.get(&facts_handle).unwrap())
                                            .expect("preflighted fact");
                                    },
                                );
                            }
                            DialogueOp::Mention { category, text } => {
                                let mut facts = world.resource_mut::<StoryFacts>();
                                if !facts
                                    .topics
                                    .iter()
                                    .any(|(cat, t)| cat == category && t.eq_ignore_ascii_case(text))
                                {
                                    facts.topics.push((*category, text.clone()));
                                }
                            }
                            DialogueOp::Close => following = Some(Step::End),
                            _ => {}
                        }
                    }
                    if let Some(next) = following {
                        c.queue.push_front(next)
                    }
                } else {
                    c.queue.push_front(Step::Quote { quote, subquote: subquote + 1, paid: true })
                }
                let text = q.text.get(subquote).cloned().unwrap_or_default();
                if text.is_empty()
                    || text.eq_ignore_ascii_case("EMPTY")
                    || text.eq_ignore_ascii_case("BLANK")
                {
                    continue;
                }
                if ["UNKNOWN", "CLASSIFIED", "SOUND"].iter().any(|s| text.eq_ignore_ascii_case(s)) {
                    block(c, quote, None, format!("unported quote sentinel {text}"));
                    return;
                }
                let Ok(def) = definition(world, c.actor) else {
                    block(c, quote, None, "NPC definition unavailable".into());
                    return;
                };
                let suffix = if subquote == 0 {
                    String::new()
                } else {
                    char::from(b'a' + (subquote - 1) as u8).to_string()
                };
                let voice = format!("{}_{quote:03}{suffix}", def.voice_stem());
                let seconds = text.encode_utf16().count() as f32 * 0.06 + 2.0;
                c.transcript.push(text.clone());
                let speech =
                    world.spawn((ConversationSpeech { voice, text }, ChildOf(entity))).id();
                c.wait = ConversationWait::Line {
                    speech,
                    phase: LinePhase::Text { remaining: seconds },
                };
                c.revision += 1;
                return;
            }
        }
    }
    block(c, c.current.map_or(0, |v| v.0), None, "script made no bounded progress".into());
}
fn topic(world: &mut World, c: &mut Conversation, text: &str) {
    if !c.ready() {
        return;
    }
    let Some(script) = script(world, c) else { return };
    let cleaned: String = text
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '\'')
        .collect();
    let words: Vec<_> = cleaned.split_whitespace().collect();
    let matched = script
        .topic(&cleaned)
        .or_else(|| words.windows(2).find_map(|pair| script.topic(&pair.join(" "))))
        .or_else(|| words.iter().find_map(|word| script.topic(word)));
    if let Some(quote) = matched {
        c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false });
        c.revision += 1;
        c.result = None;
    } else {
        c.result = Some("No supported topic matches that input.".into());
    }
}
fn reply(world: &mut World, c: &mut Conversation, text: &str) {
    let Some(script) = script(world, c) else { return };
    if let ConversationWait::Options { quote, words } = c.wait.clone() {
        let matched = words.iter().any(|w| w.eq_ignore_ascii_case(text));
        let entries = &script.quotes[quote as usize].operations;
        let response = entries
            .iter()
            .find_map(|op| match op {
                DialogueOp::Reply { response, role: 3, words }
                    if matched && words.iter().any(|w| w.eq_ignore_ascii_case(text)) =>
                {
                    Some(*response)
                }
                _ => None,
            })
            .or_else(|| {
                entries.iter().find_map(|op| match op {
                    DialogueOp::Reply { response, role, .. }
                        if *role == if matched { 1 } else { 2 } =>
                    {
                        Some(*response)
                    }
                    _ => None,
                })
            });
        if let Some(quote) = response {
            c.wait = ConversationWait::Ready;
            c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false });
            c.revision += 1;
        }
    }
}
fn answer_price(world: &mut World, c: &mut Conversation, accept: bool) {
    let Some(script) = script(world, c) else { return };
    if let ConversationWait::Price { quote, gold, always } = c.wait.clone() {
        if accept {
            if world.resource::<PartyInventory>().gold < gold {
                c.queue.push_back(Step::Quote { quote: 0x14, subquote: 0, paid: true });
            } else {
                // Validate the purchased continuation before accepting payment.
                if let Err((entry, reason)) = validate_line(world, &script.quotes[quote as usize]) {
                    block(c, quote, Some(entry), reason);
                    return;
                }
                world.resource_mut::<PartyInventory>().gold -= gold;
                if !always {
                    world.get_mut::<Npc>(c.actor).unwrap().paid_topics.push(quote);
                }
                c.queue.push_back(Step::Quote { quote, subquote: 0, paid: true });
            }
        } else {
            for op in &script.quotes[quote as usize].operations {
                if let DialogueOp::Decline(q) = op {
                    c.queue.push_back(Step::Quote { quote: *q, subquote: 0, paid: false })
                }
            }
        }
        c.wait = ConversationWait::Ready;
        c.revision += 1;
    }
}

pub fn quote_trade(
    world: &mut World,
    conversation: Entity,
    source: TradeSource,
    quantity: u32,
) -> Result<TradeQuote, TradeError> {
    let c = world.get::<Conversation>(conversation).ok_or(TradeError::StaleSelection)?;
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    if !available(world, c.actor) {
        return Err(TradeError::Unavailable);
    }
    let def = definition(world, c.actor)?;
    let actor = c.actor;
    let revision = c.revision;
    let communication = party_communication(world);
    let gold = with_items(world, |world, catalog| match source {
        TradeSource::Stock(id) => {
            let l = world
                .get::<Npc>(actor)
                .unwrap()
                .stock
                .lots
                .iter()
                .find(|l| l.id == id)
                .ok_or(TradeError::StaleSelection)?;
            let d = catalog.get(l.item).ok_or(TradeError::MissingContent)?;
            if l.units < quantity
                || quantity == 0
                || (d.class == crate::content::items::EquipClass::Ammunition && quantity != l.units)
            {
                return Err(TradeError::InvalidQuantity);
            }
            if l.available_at != 0 || d.merchant_cannot_sell {
                return Err(TradeError::Unavailable);
            }
            trade::price(&def, d, quantity, true, true, communication)
        }
        _ => {
            let item = sale_item(world, source)?;
            trade::validate_sale(&def, item, quantity, catalog, communication)
        }
    })?;
    Ok(TradeQuote { conversation, revision, source, quantity, gold })
}
fn sale_item(
    world: &World,
    source: TradeSource,
) -> Result<&crate::game::inventory::ItemStack, TradeError> {
    match source {
        TradeSource::Pool(id) => {
            world.resource::<PartyInventory>().pool.stacks.iter().find(|i| i.id == id)
        }
        TradeSource::Backpack { member, item }
            if world.resource::<PartyRoster>().0.contains(&member) =>
        {
            world
                .get::<Carried>(member)
                .and_then(|c| c.backpack.iter().flatten().find(|i| i.id == item))
        }
        _ => None,
    }
    .ok_or(TradeError::StaleSelection)
}
fn execute_trade(
    world: &mut World,
    c: &mut Conversation,
    quote: TradeQuote,
) -> Result<(), TradeError> {
    if !c.ready() || c.revision != quote.revision {
        return Err(TradeError::StaleSelection);
    }
    if !available(world, c.actor) || world.contains_resource::<crate::game::combat::Battle>() {
        return Err(TradeError::Unavailable);
    }
    let def = definition(world, c.actor)?;
    let communication = party_communication(world);
    with_items(world, |world, catalog| {
        match quote.source {
            TradeSource::Stock(lot) => {
                world.resource_scope(|world, mut party: Mut<PartyInventory>| {
                    world.resource_scope(|world, mut ids: Mut<ItemIds>| {
                        let mut npc =
                            world.get_mut::<Npc>(c.actor).ok_or(TradeError::Unavailable)?;
                        trade::buy(
                            &mut npc.stock,
                            &mut party,
                            &mut ids,
                            &def,
                            catalog,
                            lot,
                            quote.quantity,
                            quote.gold,
                            communication,
                        )
                    })
                })
            }
            source => {
                let item = sale_item(world, source)?.clone();
                let price =
                    trade::validate_sale(&def, &item, quote.quantity, catalog, communication)?;
                if price != quote.gold {
                    return Err(TradeError::PriceChanged);
                }
                let gold = world
                    .resource::<PartyInventory>()
                    .gold
                    .checked_add(price)
                    .ok_or(TradeError::Overflow)?;
                // Draft normalization before removing the source. The stock
                // template deliberately does not retain sold charges/knowledge.
                let mut stock = world.get::<Npc>(c.actor).unwrap().stock.clone();
                stock.add(item.item, quote.quantity, catalog, false)?;
                let remove = |items: &mut Vec<crate::game::inventory::ItemStack>| {
                    let at = items.iter().position(|i| i.id == item.id).unwrap();
                    if let Some(rest) = items.remove(at).remove_units(quote.quantity as u16) {
                        items.insert(at, rest);
                    }
                };
                match source {
                    TradeSource::Pool(_) => {
                        remove(&mut world.resource_mut::<PartyInventory>().pool.stacks)
                    }
                    TradeSource::Backpack { member, .. } => {
                        let mut carried = world.get_mut::<Carried>(member).unwrap();
                        let at = carried
                            .backpack
                            .iter()
                            .position(|i| i.as_ref().is_some_and(|i| i.id == item.id))
                            .unwrap();
                        carried.backpack[at] = carried.backpack[at]
                            .take()
                            .unwrap()
                            .remove_units(quote.quantity as u16);
                    }
                    _ => unreachable!(),
                }
                world.get_mut::<Npc>(c.actor).unwrap().stock = stock;
                world.resource_mut::<PartyInventory>().gold = gold;
                Ok(())
            }
        }
    })?;
    c.revision += 1;
    Ok(())
}
pub fn update_conversation(world: &mut World) {
    let requests: Vec<_> = world.resource_mut::<Messages<ConversationRequest>>().drain().collect();
    for request in requests {
        if let ConversationRequest::Begin { actor } = request {
            if let Err(e) = begin(world, actor) {
                warn!("conversation refused: {e:?}");
            }
            continue;
        }
        let entity = match &request {
            ConversationRequest::Topic { conversation, .. }
            | ConversationRequest::Reply { conversation, .. }
            | ConversationRequest::PriceAnswer { conversation, .. }
            | ConversationRequest::Advance { conversation, .. }
            | ConversationRequest::Close { conversation } => *conversation,
            ConversationRequest::Trade(q) => q.conversation,
            _ => unreachable!(),
        };
        if matches!(request, ConversationRequest::Close { .. }) {
            close_one(world, entity);
            continue;
        }
        let Some(mut c) =
            world.get_entity_mut(entity).ok().and_then(|mut e| e.take::<Conversation>())
        else {
            continue;
        };
        if !available(world, c.actor)
            || world.contains_resource::<crate::game::combat::Battle>()
            || world.contains_resource::<crate::game::camp::Camp>()
        {
            world.entity_mut(entity).insert(c);
            close_one(world, entity);
            continue;
        }
        match request {
            ConversationRequest::Topic { revision, text, .. } if revision == c.revision => {
                topic(world, &mut c, &text)
            }
            ConversationRequest::Reply { revision, text, .. } if revision == c.revision => {
                reply(world, &mut c, &text)
            }
            ConversationRequest::PriceAnswer { revision, accept, .. } if revision == c.revision => {
                answer_price(world, &mut c, accept)
            }
            ConversationRequest::Advance { speech, .. } => {
                if matches!(c.wait,ConversationWait::Line{speech:s,..} if s==speech) {
                    world.despawn(speech);
                    c.wait = ConversationWait::Ready;
                    c.revision += 1;
                }
            }
            ConversationRequest::Trade(q) => {
                c.result = Some(match execute_trade(world, &mut c, q) {
                    Ok(()) => "Trade complete.".into(),
                    Err(e) => format!("Trade refused: {e:?}"),
                });
            }
            _ => {}
        }
        world.entity_mut(entity).insert(c);
    }
    let playback: Vec<_> = world.resource_mut::<Messages<LinePlayback>>().drain().collect();
    let delta = world.resource::<Time<Real>>().delta_secs();
    let sessions: Vec<_> =
        world.query_filtered::<Entity, With<Conversation>>().iter(world).collect();
    for entity in sessions {
        let Some(mut c) =
            world.get_entity_mut(entity).ok().and_then(|mut e| e.take::<Conversation>())
        else {
            continue;
        };
        if !available(world, c.actor) || world.contains_resource::<crate::game::combat::Battle>() {
            world.entity_mut(entity).insert(c);
            close_one(world, entity);
            continue;
        }
        if let ConversationWait::Line { speech, phase } = &mut c.wait {
            for event in playback.iter().filter(|e| e.speech == *speech) {
                match event.event {
                    PlaybackEvent::Started if matches!(phase, LinePhase::Text { .. }) => {
                        *phase = LinePhase::Audio
                    }
                    PlaybackEvent::Finished if matches!(phase, LinePhase::Audio) => {
                        *phase = LinePhase::Hold { remaining: 0.7 }
                    }
                    PlaybackEvent::Unavailable if !matches!(phase, LinePhase::Hold { .. }) => {
                        let remaining =
                            world.get::<ConversationSpeech>(*speech).map_or(2.0, |line| {
                                line.text.encode_utf16().count() as f32 * 0.06 + 2.0
                            });
                        *phase = LinePhase::Text { remaining };
                    }
                    _ => {}
                }
            }
            let done = match phase {
                LinePhase::Text { remaining } | LinePhase::Hold { remaining } => {
                    *remaining -= delta;
                    *remaining <= 0.0
                }
                LinePhase::Audio => false,
            };
            if done {
                world.despawn(*speech);
                c.wait = ConversationWait::Ready;
                c.revision += 1;
            }
        }
        run(world, entity, &mut c);
        // End can request close while the component is temporarily taken.
        if matches!(c.queue.front(), Some(Step::End)) && c.wait == ConversationWait::Ready {
            world.entity_mut(entity).insert(c);
            close_one(world, entity);
        } else {
            world.entity_mut(entity).insert(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::world::population::ScaledBody;
    fn app(script: DialogueScript) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(16),
            ))
            .add_message::<ConversationRequest>()
            .add_message::<LinePlayback>()
            .init_resource::<StoryFacts>()
            .init_resource::<GameRng>()
            .init_resource::<PartyInventory>()
            .init_resource::<Assets<NpcCatalog>>()
            .init_resource::<Assets<DialogueScript>>()
            .init_resource::<Assets<FactCatalog>>()
            .add_systems(Update, update_conversation);
        let w = app.world_mut();
        let catalog = w
            .resource_mut::<Assets<NpcCatalog>>()
            .add(NpcCatalog(vec![NpcDefinition { source_name: "Speaker".into(), ..default() }]));
        let facts =
            w.resource_mut::<Assets<FactCatalog>>().add(FactCatalog(vec![FactDefinition {
                id: FactId(91),
                symbol: "FACT_BURZ_SAID130".into(),
                description: String::new(),
                alternate_description: String::new(),
            }]));
        let script = w.resource_mut::<Assets<DialogueScript>>().add(script);
        w.insert_resource(NpcAssets { catalog, facts, scripts: default() });
        let level = w.spawn(Level::test(default())).id();
        let actor = w
            .spawn((
                ChildOf(level),
                Npc {
                    definition: NpcId(0),
                    disposition: 80,
                    greeting_pending: true,
                    paid_topics: vec![],
                    dormant: None,
                    stock: default(),
                },
                Monster {
                    location: 1,
                    species: 0,
                    hp: 5,
                    max_hp: 5,
                    stamina: 5,
                    max_stamina: 5,
                    conditions: vec![],
                    body: ScaledBody { scale: 1., radius: 0.5, eye_height: 1., speed: 0. },
                },
            ))
            .id();
        let entity = w
            .spawn(Conversation {
                actor,
                script,
                revision: 0,
                current: None,
                wait: ConversationWait::Ready,
                transcript: vec![],
                result: None,
                queue: VecDeque::from([Step::Quote { quote: 0, subquote: 0, paid: false }]),
            })
            .id();
        (app, entity, actor)
    }
    #[test]
    fn unknown_mandatory_operation_blocks_before_any_effect_or_successful_line() {
        let q = Quote {
            text: vec!["This must not claim success".into()],
            operations: vec![
                DialogueOp::Mention { category: TopicCategory::Person, text: "Someone".into() },
                DialogueOp::Unsupported { opcode: 99, operands: [0; 3], subentries: vec![] },
                DialogueOp::SetFact(FactId(91), 1),
            ],
            alert: false,
        };
        let (mut app, session, _) = app(DialogueScript { quotes: vec![q], ..default() });
        app.update();
        let c = app.world().get::<Conversation>(session).unwrap();
        assert!(matches!(c.wait, ConversationWait::Blocked { entry: Some(1), .. }));
        assert!(c.transcript.is_empty());
        assert!(app.world().resource::<StoryFacts>().topics.is_empty());
        assert!(app.world().resource::<StoryFacts>().values.is_empty());
    }
    #[test]
    fn playback_waits_for_its_own_identity_and_real_hold_and_closure_keeps_npc() {
        let (mut app, session, actor) = app(DialogueScript {
            quotes: vec![
                Quote {
                    text: vec!["First".into()],
                    operations: vec![DialogueOp::Next(Response::Fixed(1))],
                    ..default()
                },
                Quote { text: vec!["Second".into()], ..default() },
            ],
            ..default()
        });
        app.update();
        let speech = match app.world().get::<Conversation>(session).unwrap().wait {
            ConversationWait::Line { speech, .. } => speech,
            _ => panic!("line expected"),
        };
        let unrelated = app.world_mut().spawn_empty().id();
        app.world_mut()
            .write_message(LinePlayback { speech: unrelated, event: PlaybackEvent::Finished });
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((0, 0)));
        app.world_mut().write_message(LinePlayback { speech, event: PlaybackEvent::Started });
        app.world_mut().write_message(LinePlayback { speech, event: PlaybackEvent::Finished });
        app.update();
        assert!(matches!(
            app.world().get::<Conversation>(session).unwrap().wait,
            ConversationWait::Line { phase: LinePhase::Hold { .. }, .. }
        ));
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((1, 0)));
        app.world_mut().write_message(LinePlayback { speech, event: PlaybackEvent::Finished });
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((1, 0)));
        app.world_mut().write_message(ConversationRequest::Close { conversation: session });
        app.update();
        assert!(app.world().get_entity(session).is_err());
        assert!(app.world().get::<Npc>(actor).is_some());
        // A delayed request for a closed session is harmless.
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
    }
    #[test]
    fn information_payment_refusal_and_duplicate_acceptance_preserve_ownership() {
        let mut quotes = vec![Quote::default(); 21];
        quotes[0] = Quote {
            text: vec!["Paid information".into()],
            operations: vec![
                DialogueOp::Price { gold: 25, always: false },
                DialogueOp::SetFact(FactId(91), 1),
                DialogueOp::Keywords(vec!["INFO".into()]),
            ],
            ..default()
        };
        quotes[18].text = vec!["Information has a price".into()];
        quotes[20].text = vec!["Insufficient funds".into()];
        let (mut app, session, actor) = app(DialogueScript { quotes, ..default() });
        fn advance(app: &mut App, session: Entity) {
            let speech = match app.world().get::<Conversation>(session).unwrap().wait {
                ConversationWait::Line { speech, .. } => speech,
                _ => panic!("line expected"),
            };
            app.world_mut()
                .write_message(ConversationRequest::Advance { conversation: session, speech });
            app.update();
        }
        fn ask(app: &mut App, session: Entity) {
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            app.world_mut().write_message(ConversationRequest::Topic {
                conversation: session,
                revision,
                text: "INFO".into(),
            });
            app.update();
            advance(app, session);
        }
        app.update();
        advance(&mut app, session);
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::PriceAnswer {
            conversation: session,
            revision,
            accept: true,
        });
        app.update();
        assert_eq!(app.world().resource::<PartyInventory>().gold, 0);
        assert!(app.world().get::<Npc>(actor).unwrap().paid_topics.is_empty());
        assert!(app.world().resource::<StoryFacts>().values.is_empty());
        advance(&mut app, session);
        ask(&mut app, session);
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::PriceAnswer {
            conversation: session,
            revision,
            accept: false,
        });
        app.update();
        assert!(app.world().get::<Conversation>(session).unwrap().ready());
        assert!(app.world().resource::<StoryFacts>().values.is_empty());
        app.world_mut().resource_mut::<PartyInventory>().gold = 100;
        ask(&mut app, session);
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        for _ in 0..2 {
            app.world_mut().write_message(ConversationRequest::PriceAnswer {
                conversation: session,
                revision,
                accept: true,
            });
        }
        app.update();
        assert_eq!(app.world().resource::<PartyInventory>().gold, 75);
        assert_eq!(app.world().get::<Npc>(actor).unwrap().paid_topics, [0]);
        assert_eq!(app.world().resource::<StoryFacts>().values.get(&FactId(91)), Some(&1));
    }
    #[test]
    fn authored_reply_words_select_both_paths_and_stale_answers_do_not_replay() {
        for (word, response, value) in [("ALPHA", 1, 1), ("BETA", 2, 0)] {
            let script = DialogueScript {
                quotes: vec![
                    Quote {
                        text: vec!["Choose".into()],
                        operations: vec![
                            DialogueOp::Options(vec!["ALPHA".into(), "BETA".into()]),
                            DialogueOp::Reply { response: 1, role: 3, words: vec!["ALPHA".into()] },
                            DialogueOp::Reply { response: 2, role: 3, words: vec!["BETA".into()] },
                        ],
                        ..default()
                    },
                    Quote {
                        text: vec!["First answer".into()],
                        operations: vec![DialogueOp::SetFact(FactId(91), 1)],
                        ..default()
                    },
                    Quote {
                        text: vec!["Second answer".into()],
                        operations: vec![DialogueOp::SetFact(FactId(91), 0)],
                        ..default()
                    },
                ],
                ..default()
            };
            let (mut app, session, _) = app(script);
            app.update();
            let speech = match app.world().get::<Conversation>(session).unwrap().wait {
                ConversationWait::Line { speech, .. } => speech,
                _ => panic!("line expected"),
            };
            app.world_mut()
                .write_message(ConversationRequest::Advance { conversation: session, speech });
            app.update();
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            for _ in 0..2 {
                app.world_mut().write_message(ConversationRequest::Reply {
                    conversation: session,
                    revision,
                    text: word.into(),
                });
            }
            app.update();
            assert_eq!(
                app.world().get::<Conversation>(session).unwrap().current,
                Some((response, 0))
            );
            assert_eq!(app.world().resource::<StoryFacts>().values.get(&FactId(91)), Some(&value));
            assert_eq!(app.world().get::<Conversation>(session).unwrap().transcript.len(), 2);
        }
    }
}
