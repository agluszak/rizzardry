//! Authored continuation and waits belong to gameplay, not the dialogue view.
use super::{
    Npc, NpcAssets,
    trade::{self, TradeError, TradeQuote, TradeSource},
};
use crate::content::{items::ItemCatalog, npcs::*};
use crate::game::{
    characters::{Character, Vitals},
    data::GameData,
    inventory::{Carried, ItemIds, ItemOwner, PartyInventory},
    party::roster::{PartyMember, PartyRoster, the_party},
    rng::GameRng,
    world::{ActorPose, level::Level, population::Monster},
};
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
    /// The keyword notebook outlives a conversation. Learning/discussing a
    /// keyword does not consume it; only the explicit minus button removes it.
    pub fn remember_topic(&mut self, category: TopicCategory, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.topics.iter().any(|(_, known)| known.eq_ignore_ascii_case(text))
        {
            return false;
        }
        self.topics.push((category, text.to_owned()));
        true
    }

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
            "FACT_VIRGIN" | "FACT_IMPORT" | "FACT_BURZ_SAID130" | "FACT_TEMPLAR" => {
                Ok(*self.values.get(&id).unwrap_or(&0))
            }
            name => Err(format!("unported fact predicate {name}")),
        }
    }
    fn can_set(id: FactId, catalog: &FactCatalog) -> bool {
        catalog
            .get(id)
            .is_some_and(|f| matches!(f.symbol.as_str(), "FACT_BURZ_SAID130" | "FACT_TEMPLAR"))
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
/// Accepted dialogue history, including the party's input. Presentation chooses
/// the retail palette without guessing speakers from the text.
#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptLine {
    Npc(String),
    Question(String),
    Reply(String),
}
#[derive(Component, Debug)]
pub struct Conversation {
    pub actor: Entity,
    pub script: Handle<DialogueScript>,
    pub revision: u64,
    pub current: Option<(u16, usize)>,
    pub wait: ConversationWait,
    pub transcript: Vec<TranscriptLine>,
    /// Keywords discovered during this conversation are green. Reopening
    /// imports the persistent notebook without the transient discovery marks.
    pub learned_topics: BTreeSet<String>,
    /// Revision-scoped gameplay outcome; presentation owns its wording.
    result: Option<(u64, Outcome)>,
    queue: VecDeque<Step>,
}
/// What the party's last request came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NoTopic,
    Traded,
    TradeRefused(TradeError),
}
impl Conversation {
    pub fn ready(&self) -> bool {
        self.wait == ConversationWait::Ready && self.queue.is_empty()
    }

    /// What the last request came to, until the conversation moves on.
    pub fn outcome(&self) -> Option<Outcome> {
        self.result.filter(|&(revision, _)| revision == self.revision).map(|(_, o)| o)
    }

    fn report(&mut self, outcome: Outcome) {
        self.result = Some((self.revision, outcome));
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
fn definition(world: &World, actor: Entity) -> Result<&NpcDefinition, TradeError> {
    let npc = world.get::<Npc>(actor).ok_or(TradeError::Unavailable)?;
    let assets = world.resource::<NpcAssets>();
    world
        .resource::<Assets<NpcCatalog>>()
        .get(&assets.catalog)
        .and_then(|c| c.get(npc.definition))
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
pub fn party_communication(world: &mut World) -> u32 {
    let Some(party) = the_party(world) else { return 0 };
    let Some(roster) = world.get::<PartyRoster>(party) else { return 0 };
    roster
        .iter()
        .filter_map(|e| Some((world.get::<Character>(e)?, world.get::<Vitals>(e)?)))
        .filter(|(_, v)| v.can_act())
        .map(|(c, _)| c.skill(crate::content::characters::Skill::Communication))
        .max()
        .unwrap_or(0)
}
fn average_level(world: &mut World) -> u32 {
    let Some(party) = the_party(world) else { return 1 };
    let Some(roster) = world.get::<PartyRoster>(party) else { return 1 };
    crate::game::party::average_level(
        roster.iter().filter_map(|member| world.get::<Character>(member)).map(|c| c.level),
    )
}
fn active_body(world: &World, actor: Entity) -> Option<Entity> {
    world.get::<super::NpcBodies>(actor)?.iter().find(|body| {
        world
            .get::<crate::game::world::level::LevelOwned>(*body)
            .is_some_and(|parent| world.get::<Level>(parent.0).is_some())
            && world.get::<Monster>(*body).is_some_and(|m| !m.is_dead())
    })
}
/// The lowest disposition at which an NPC will talk.
const WILLING_DISPOSITION: u8 = 0x42;
/// The script's quote asking the price of a paid topic.
const PRICE_QUOTE: u16 = 0x12;
/// The script's quote for a party that cannot pay.
const TOO_POOR_QUOTE: u16 = 0x14;
fn available(world: &World, actor: Entity) -> bool {
    active_body(world, actor)
        .is_some_and(|body| world.get::<crate::game::world::population::Provoked>(body).is_none())
        && world.get::<Npc>(actor).is_some_and(|n| n.disposition >= WILLING_DISPOSITION)
}
/// Shared gameplay permission, also used to derive the trade controls.
pub fn trade_allowed(world: &World, actor: Entity) -> bool {
    definition(world, actor).is_ok_and(|def| def.owns_stock)
}
fn entry_quote(def: &NpcDefinition, npc: &Npc) -> u16 {
    if def.merchant || npc.greeting_pending { 0 } else { 1 }
}
fn conditions_match(world: &World, facts: &[(FactId, u8)]) -> Result<bool, String> {
    for (id, expected) in facts {
        if fact(world, *id)? != *expected {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn begin(world: &mut World, actor: Entity) -> Result<Entity, TradeError> {
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    if !world.run_system_once(crate::game::screen::party_free).map_err(|_| TradeError::Busy)?
        || crate::game::combat::in_battle(world)
    {
        return Err(TradeError::Busy);
    }
    if world.get::<PartyInventory>(party).unwrap().hand.held.is_some() {
        return Err(TradeError::Busy);
    }
    let body = active_body(world, actor).ok_or(TradeError::Unavailable)?;
    // Recheck spatial reach from authoritative actor/party poses, not a UI claim.
    let at = world
        .run_system_once(move |q: Query<ActorPose>| q.get(body).ok().map(|p| p.translation()))
        .ok()
        .flatten()
        .ok_or(TradeError::Unavailable)?;
    let party_at = world
        .query::<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>()
        .iter(world)
        .find(|(p, _)| p.0.distance(at) <= crate::game::world::items::REACH)
        .map(|(p, _)| p.0)
        .ok_or(TradeError::Unavailable)?;
    let def = definition(world, actor)?.clone();
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
        let stock = &world.get::<Npc>(actor).unwrap().stock;
        let gated = stock.initialized
            && now.saturating_sub(stock.maintenance_at) >= crate::game::time::DAY_MS
            && def.stock.iter().any(|r| !r.persistent && r.item == super::trade::TEMPLAR_GATED);
        let templar = if gated {
            let catalog = fact_catalog(world).ok_or(TradeError::MissingContent)?;
            let id = catalog
                .0
                .iter()
                .find(|f| f.symbol == "FACT_TEMPLAR")
                .ok_or(TradeError::MissingContent)?
                .id;
            fact(world, id).map_err(|_| TradeError::MissingContent)? != 0
        } else {
            false
        };
        with_items(world, |world, items| {
            world.resource_scope(|world, mut rng: Mut<GameRng>| {
                world
                    .get_mut::<Npc>(actor)
                    .unwrap()
                    .stock
                    .maintain(&def, items, average, now, templar, &mut rng)
            })
        })?;
    }
    let facts_handle = world.resource::<NpcAssets>().facts.clone();
    world.resource_scope(|world, catalogs: Mut<Assets<FactCatalog>>| {
        let catalog = catalogs.get(&facts_handle).ok_or(TradeError::MissingContent)?;
        world.resource_mut::<StoryFacts>().initialize(catalog);
        Ok::<_, TradeError>(())
    })?;
    // OpenNpcDialoguePanel / MonsterForwardReferencePosition: the accepted
    // speaker faces the party. Physics rotation owns a moving body's facing.
    {
        let delta = (party_at - at).with_y(0.);
        if delta.length_squared() > 0.0001 {
            let facing = Quat::from_rotation_y((-delta.x).atan2(-delta.z));
            if let Some(mut rotation) = world.get_mut::<avian3d::prelude::Rotation>(body) {
                rotation.0 = facing;
            }
            // Set the corresponding engine transform in the same accepted
            // teleport, so Avian input synchronization cannot restore its old yaw.
            if let Some(mut transform) = world.get_mut::<Transform>(body) {
                transform.rotation = facing;
            }
        }
    }
    let greeting = entry_quote(&def, world.get::<Npc>(actor).unwrap());
    Ok(world
        .spawn(Conversation {
            actor,
            script,
            revision: 0,
            current: None,
            wait: ConversationWait::Ready,
            transcript: vec![],
            learned_topics: default(),
            result: None,
            queue: VecDeque::from([Step::Quote { quote: greeting, subquote: 0, paid: false }]),
        })
        .id())
}
/// Travel ends the conversation; the speaker stays.
pub(crate) fn end_conversations(
    _: On<Add<crate::game::world::level::Destination>>,
    conversations: Query<Entity, With<Conversation>>,
    mut commands: Commands,
) {
    for conversation in &conversations {
        commands.entity(conversation).despawn();
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
fn script<'w>(world: &'w World, c: &Conversation) -> Option<&'w DialogueScript> {
    world.resource::<Assets<DialogueScript>>().get(&c.script)
}
/// A waited-on quote; the wait was set from this same script.
fn quote(world: &World, c: &Conversation, quote: u16) -> Option<Quote> {
    script(world, c)?.quotes.get(usize::from(quote)).cloned()
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
            DialogueOp::Conditional { facts, .. }
                if conditions_match(world, facts).map_err(|e| (i, e))? =>
            {
                break;
            }
            _ => {}
        }
        if matches!(op, DialogueOp::Next(_) | DialogueOp::Options(_)) {
            break;
        }
    }
    Ok(())
}
fn run(world: &mut World, entity: Entity, c: &mut Conversation) {
    if c.wait != ConversationWait::Ready || c.queue.is_empty() {
        return;
    }
    // Steps change the world, so the script is held apart from it.
    let Some(script) = script(world, c).cloned() else {
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
                    c.queue.push_front(Step::Quote { quote: PRICE_QUOTE, subquote: 0, paid: true });
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
                    let first = world.get::<Npc>(c.actor).unwrap().greeting_pending;
                    if first && let Ok(definition) = definition(world, c.actor) {
                        let name = definition.source_name.clone();
                        learn_topic(world, c, TopicCategory::Person, &name);
                    }
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
                                if conditions_match(world, facts)
                                    .expect("preflighted conditions") =>
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
                                learn_topic(world, c, *category, text);
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
                c.transcript.push(TranscriptLine::Npc(text.clone()));
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
fn learn_topic(world: &mut World, c: &mut Conversation, category: TopicCategory, text: &str) {
    if world.resource_mut::<StoryFacts>().remember_topic(category, text) {
        c.learned_topics.insert(text.trim().to_ascii_lowercase());
    }
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
        // The accepted question belongs in the notebook even when its
        // answer first requires payment or another continuation.
        world.resource_mut::<StoryFacts>().remember_topic(TopicCategory::Misc, &cleaned);
        c.transcript.push(TranscriptLine::Question(text.to_owned()));
        c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false });
        c.revision += 1;
    } else {
        c.report(Outcome::NoTopic);
    }
}
fn reply(world: &mut World, c: &mut Conversation, text: &str) {
    let Some(script) = script(world, c) else { return };
    if let ConversationWait::Options { quote, words } = c.wait.clone() {
        let matched = words.iter().any(|w| w.eq_ignore_ascii_case(text));
        let Some(entries) = script.quotes.get(usize::from(quote)).map(|q| &q.operations) else {
            return;
        };
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
            c.transcript.push(TranscriptLine::Reply(text.to_owned()));
            c.wait = ConversationWait::Ready;
            c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false });
            c.revision += 1;
        }
    }
}
fn answer_price(world: &mut World, c: &mut Conversation, accept: bool) {
    let Some(party) = the_party(world) else { return };
    if let ConversationWait::Price { quote: id, gold, always } = c.wait.clone() {
        let Some(line) = quote(world, c, id) else { return };
        let quote = id;
        if accept {
            if world.get::<PartyInventory>(party).unwrap().gold < gold {
                c.queue.push_back(Step::Quote { quote: TOO_POOR_QUOTE, subquote: 0, paid: true });
            } else {
                // Validate the purchased continuation before accepting payment.
                if let Err((entry, reason)) = validate_line(world, &line) {
                    block(c, quote, Some(entry), reason);
                    return;
                }
                world.get_mut::<PartyInventory>(party).unwrap().gold -= gold;
                if !always {
                    world.get_mut::<Npc>(c.actor).unwrap().paid_topics.push(quote);
                }
                c.queue.push_back(Step::Quote { quote, subquote: 0, paid: true });
            }
        } else {
            for op in &line.operations {
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
    if crate::game::world::level::entering(world) {
        return Err(TradeError::Busy);
    }
    let c = world.get::<Conversation>(conversation).ok_or(TradeError::StaleSelection)?;
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    if !available(world, c.actor) || !trade_allowed(world, c.actor) {
        return Err(TradeError::Unavailable);
    }
    let def = definition(world, c.actor)?.clone();
    let actor = c.actor;
    let revision = c.revision;
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    let communication = party_communication(world);
    let gold = with_items(world, |world, catalog| match source {
        TradeSource::Stock(lot) => {
            let stock = &world.get::<Npc>(actor).ok_or(TradeError::Unavailable)?.stock;
            trade::purchase_price(stock, &def, catalog, lot, quantity, communication)
        }
        _ => {
            let item = sale_item(world, source, party)?;
            trade::validate_sale(&def, item, quantity, catalog, communication)
        }
    })?;
    Ok(TradeQuote { conversation, revision, source, quantity, gold })
}
fn sale_item(
    world: &World,
    source: TradeSource,
    party: Entity,
) -> Result<&crate::game::inventory::ItemStack, TradeError> {
    let TradeSource::Party(source) = source else { return Err(TradeError::StaleSelection) };
    match source.owner {
        ItemOwner::Pool => world
            .get::<PartyInventory>(party)
            .unwrap()
            .pool
            .stacks
            .iter()
            .find(|i| i.id == source.id),
        ItemOwner::Member(member)
            if world.get::<PartyMember>(member).is_some_and(|m| m.0 == party) =>
        {
            world.get::<Carried>(member).and_then(|c| c.backpack_stack(source.id))
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
    if !available(world, c.actor)
        || !trade_allowed(world, c.actor)
        || crate::game::combat::in_battle(world)
    {
        return Err(TradeError::Unavailable);
    }
    let def = definition(world, c.actor)?.clone();
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    let communication = party_communication(world);
    with_items(world, |world, catalog| match quote.source {
        TradeSource::Stock(lot) => world.resource_scope(|world, mut ids: Mut<ItemIds>| {
            let [mut party_entity, mut actor] =
                world.get_entity_mut([party, c.actor]).map_err(|_| TradeError::Unavailable)?;
            let mut inventory =
                party_entity.get_mut::<PartyInventory>().ok_or(TradeError::Unavailable)?;
            let mut npc = actor.get_mut::<Npc>().ok_or(TradeError::Unavailable)?;
            trade::buy(
                &mut npc.stock,
                &mut inventory,
                &mut ids,
                &def,
                catalog,
                lot,
                quote.quantity,
                quote.gold,
                communication,
            )
        }),
        TradeSource::Party(source) => {
            let item = sale_item(world, TradeSource::Party(source), party)?;
            let units = u16::try_from(quote.quantity).map_err(|_| TradeError::InvalidQuantity)?;
            let gold = world
                .get::<PartyInventory>(party)
                .ok_or(TradeError::Unavailable)?
                .gold
                .checked_add(quote.gold)
                .ok_or(TradeError::Overflow)?;
            let merchant = &world.get::<Npc>(c.actor).ok_or(TradeError::Unavailable)?.stock;
            let stock = trade::sell(
                merchant,
                &def,
                item,
                quote.quantity,
                quote.gold,
                catalog,
                communication,
            )?;
            match source.owner {
                ItemOwner::Pool => world
                    .get_mut::<PartyInventory>(party)
                    .unwrap()
                    .pool
                    .consume_units(source.id, units),
                ItemOwner::Member(member) => world
                    .get_mut::<Carried>(member)
                    .unwrap()
                    .consume_backpack_units(source.id, units),
            }
            .map_err(|_| TradeError::StaleSelection)?;
            world.get_mut::<Npc>(c.actor).unwrap().stock = stock;
            world.get_mut::<PartyInventory>(party).unwrap().gold = gold;
            Ok(())
        }
    })?;
    c.revision += 1;
    Ok(())
}
pub fn update_conversation(world: &mut World) {
    if crate::game::world::level::entering(world) {
        // An accepted entry has ended the dialogue (`end_conversations`);
        // no queued payment, trade or authored effect may alter the
        // departing campaign.
        world.resource_mut::<Messages<ConversationRequest>>().clear();
        world.resource_mut::<Messages<LinePlayback>>().clear();
        return;
    }
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
            || crate::game::combat::in_battle(world)
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
                let outcome = match execute_trade(world, &mut c, q) {
                    Ok(()) => Outcome::Traded,
                    Err(e) => Outcome::TradeRefused(e),
                };
                c.report(outcome);
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
        if !available(world, c.actor) || crate::game::combat::in_battle(world) {
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
    use crate::game::inventory::ItemRef;
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
            .init_resource::<Assets<NpcCatalog>>()
            .init_resource::<Assets<DialogueScript>>()
            .init_resource::<Assets<FactCatalog>>()
            .add_observer(end_conversations)
            .add_systems(Update, update_conversation);
        let w = app.world_mut();
        crate::game::party::roster::test_party(w, &[]);
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
            .spawn((Npc {
                definition: NpcId(0),
                disposition: 80,
                greeting_pending: true,
                paid_topics: vec![],
                stock: default(),
            },))
            .id();
        w.spawn((
            super::super::NpcBody(actor),
            crate::game::world::level::LevelOwned(level),
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
        ));
        let entity = w
            .spawn(Conversation {
                actor,
                script,
                revision: 0,
                current: None,
                wait: ConversationWait::Ready,
                transcript: vec![],
                learned_topics: default(),
                result: None,
                queue: VecDeque::from([Step::Quote { quote: 0, subquote: 0, paid: false }]),
            })
            .id();
        (app, entity, actor)
    }
    #[test]
    fn discussed_topics_survive_the_session_and_discoveries_are_marked_only_in_it() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![
                Quote::default(),
                Quote {
                    text: vec!["The ship came from Arnika.".into()],
                    operations: vec![
                        DialogueOp::Keywords(vec!["SHIP".into()]),
                        DialogueOp::Mention {
                            category: TopicCategory::Place,
                            text: "Arnika".into(),
                        },
                    ],
                    ..default()
                },
            ],
            ..default()
        });
        app.update();
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        for text in ["SHIP", "ship"] {
            app.world_mut().write_message(ConversationRequest::Topic {
                conversation: session,
                revision,
                text: text.into(),
            });
        }
        app.update();
        let facts = app.world().resource::<StoryFacts>();
        assert_eq!(facts.topics.iter().filter(|(_, t)| t.eq_ignore_ascii_case("ship")).count(), 1);
        let session_state = app.world().get::<Conversation>(session).unwrap();
        assert!(session_state.learned_topics.contains("arnika"));
        assert!(!session_state.learned_topics.contains("ship"));
        app.world_mut().despawn(session);
        assert!(app.world().resource::<StoryFacts>().topics.iter().any(|(_, t)| t == "SHIP"));
        assert!(
            !app.world_mut()
                .resource_mut::<StoryFacts>()
                .remember_topic(TopicCategory::Misc, "ship")
        );
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
        use crate::game::world::level::{Destination, Entry};
        let entering = app
            .world_mut()
            .spawn(Destination::new(Level::test(default()), default(), Entry::Travel))
            .id();
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        assert!(app.world().get_entity(session).is_err());
        assert!(app.world().get::<Npc>(actor).is_some());
        app.world_mut().despawn(entering);
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
        let party = the_party(app.world_mut()).unwrap();
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
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 0);
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
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 100;
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
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 75);
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
            let history = &app.world().get::<Conversation>(session).unwrap().transcript;
            assert_eq!(history.len(), 3);
            assert_eq!(history[1], TranscriptLine::Reply(word.into()));
        }
    }
    #[test]
    fn conditional_preflight_short_circuits_before_unreachable_semantics() {
        for (expected, blocked) in [(1, false), (0, true)] {
            let (mut app, session, _) = app(DialogueScript {
                quotes: vec![Quote {
                    text: vec!["Continue normally".into()],
                    operations: vec![DialogueOp::Conditional {
                        facts: vec![(FactId(91), expected), (FactId(999), 1)],
                        response: Response::Fixed(0),
                    }],
                    ..default()
                }],
                ..default()
            });
            app.update();
            let c = app.world().get::<Conversation>(session).unwrap();
            assert_eq!(matches!(c.wait, ConversationWait::Blocked { .. }), blocked);
            assert_eq!(c.transcript.is_empty(), blocked);
        }
    }
    #[test]
    fn an_npc_without_stock_ownership_cannot_quote_or_settle_a_trade() {
        let (mut app, session, actor) = app(DialogueScript::default());
        let world = app.world_mut();
        let party = the_party(world).unwrap();
        let mut c = world.entity_mut(session).take::<Conversation>().unwrap();
        c.queue.clear();
        world.entity_mut(session).insert(c);
        for source in [
            TradeSource::Stock(trade::StockLotId(1)),
            TradeSource::Party(ItemRef {
                owner: ItemOwner::Pool,
                id: crate::game::inventory::StackId(1),
            }),
        ] {
            assert!(!trade_allowed(world, actor));
            assert_eq!(quote_trade(world, session, source, 1), Err(TradeError::Unavailable));
            let mut c = world.entity_mut(session).take::<Conversation>().unwrap();
            let quote = TradeQuote {
                conversation: session,
                revision: c.revision,
                source,
                quantity: 1,
                gold: 1,
            };
            assert_eq!(execute_trade(world, &mut c, quote), Err(TradeError::Unavailable));
            world.entity_mut(session).insert(c);
        }
        assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 0);
        assert!(world.get::<Npc>(actor).unwrap().stock.lots.is_empty());
    }
    #[test]
    fn party_item_references_only_sell_backpack_or_pool_possessions() {
        use crate::content::items::ItemDefinition;
        use crate::game::inventory::{EquipSlot, Equipped, ItemStack};
        let (mut app, _, _) = app(DialogueScript::default());
        let world = app.world_mut();
        let party = the_party(world).unwrap();
        let mut ids = ItemIds::default();
        let item = ItemStack::new(&ItemDefinition::default(), &mut ids, 1, true);
        let mut carried = Carried::default();
        carried.equipment[EquipSlot::Primary] = Some(Equipped::new(item.clone()));
        let member = world.spawn(carried).id();
        world.entity_mut(member).insert(PartyMember(party));
        let source = TradeSource::Party(ItemRef { owner: ItemOwner::Member(member), id: item.id });
        assert!(matches!(sale_item(world, source, party), Err(TradeError::StaleSelection)));
        world.get_mut::<Carried>(member).unwrap().equipment[EquipSlot::Primary] = None;
        world.get_mut::<Carried>(member).unwrap().backpack[0] = Some(item.clone());
        assert_eq!(sale_item(world, source, party).unwrap().id, item.id);
        world.entity_mut(member).remove::<PartyMember>();
        assert!(matches!(sale_item(world, source, party), Err(TradeError::StaleSelection)));
    }
}
