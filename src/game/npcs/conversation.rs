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
    pub journal: Vec<JournalEntry>,
    initialized: bool,
}
/// A fact transition, retaining the level it happened on and whether the
/// fact became true, which picks its description (retail `W8JournalEntry`'s
/// `alternate_text`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub fact: FactId,
    pub was_true: bool,
    pub level: String,
}
impl JournalEntry {
    pub fn description<'a>(&self, catalog: &'a FactCatalog) -> Option<&'a str> {
        let fact = catalog.get(self.fact)?;
        Some(if self.was_true { &fact.alternate_description } else { &fact.description })
    }
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
            // Plain stored facts (retail `EvaluateFact` reads them unchanged).
            "FACT_VIRGIN"
            | "FACT_IMPORT"
            | "FACT_BURZ_SAID130"
            | "FACT_TEMPLAR"
            | "FACT_UMISSION_SCUBA_DONE"
            | "FACT_SEXUS_PAID"
            | "FACT_TRYNNIE_MADRAS_WILL_JOIN"
            | "FACT_PEACE_ACHIEVED" => Ok(*self.values.get(&id).unwrap_or(&0)),
            name => Err(format!("unported fact predicate {name}")),
        }
    }
    fn can_set(id: FactId, catalog: &FactCatalog) -> bool {
        catalog
            .get(id)
            .is_some_and(|f| matches!(f.symbol.as_str(), "FACT_BURZ_SAID130" | "FACT_TEMPLAR"))
    }
    fn set(
        &mut self,
        id: FactId,
        value: u8,
        catalog: &FactCatalog,
        level: String,
    ) -> Result<(), String> {
        if !Self::can_set(id, catalog) {
            return Err(format!("unported fact assignment {id:?}"));
        }
        let previous = self.values.insert(id, value).unwrap_or(0);
        if previous != value {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level });
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
/// A fact by its symbol, for rules that name one.
pub fn fact_named(world: &World, symbol: &str) -> Result<u8, String> {
    let id = fact_catalog(world)
        .ok_or_else(|| "fact database unavailable".to_string())?
        .0
        .iter()
        .find(|fact| fact.symbol == symbol)
        .ok_or_else(|| format!("missing fact {symbol}"))?
        .id;
    fact(world, id)
}
fn fact(world: &World, id: FactId) -> Result<u8, String> {
    world
        .resource::<StoryFacts>()
        .get(id, fact_catalog(world).ok_or_else(|| "fact database unavailable".to_string())?)
}
#[derive(Debug)]
enum Step {
    Quote { quote: u16, subquote: usize, paid: bool, plan: Option<QuotePlan> },
    Options { quote: u16, words: Vec<String> },
    Price { quote: u16, gold: u32, always: bool },
    Join,
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
    Recruit { conversation: Entity, revision: u64 },
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
fn with_definition<T>(
    world: &mut World,
    actor: Entity,
    f: impl FnOnce(&mut World, &NpcDefinition) -> Result<T, TradeError>,
) -> Result<T, TradeError> {
    let id = world.get::<Npc>(actor).ok_or(TradeError::Unavailable)?.definition;
    let handle = world.resource::<NpcAssets>().catalog.clone();
    world.resource_scope(|world, catalogs: Mut<Assets<NpcCatalog>>| {
        let def =
            catalogs.get(&handle).and_then(|c| c.get(id)).ok_or(TradeError::MissingContent)?;
        f(world, def)
    })
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
    let def = definition(world, actor)?;
    let greeting = entry_quote(def, world.get::<Npc>(actor).unwrap());
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
        with_definition(world, actor, |world, def| {
            with_items(world, |world, items| {
                world.resource_scope(|world, mut rng: Mut<GameRng>| {
                    world
                        .get_mut::<Npc>(actor)
                        .unwrap()
                        .stock
                        .maintain(def, items, average, now, templar, &mut rng)
                })
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
            queue: VecDeque::from([Step::Quote {
                quote: greeting,
                subquote: 0,
                paid: false,
                plan: None,
            }]),
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
/// A quote's supported effects and selected control transfer. Planning reads
/// through preceding assignments, but never changes campaign state or draws RNG.
#[derive(Debug)]
struct QuotePlan {
    effects: Vec<QuoteEffect>,
    following: Option<Continuation>,
}
#[derive(Debug)]
enum QuoteEffect {
    SetFact(FactId, u8),
    Mention(TopicCategory, String),
}
#[derive(Debug)]
enum Continuation {
    Quote(Response),
    Options(Vec<String>),
    End,
}
fn plan_quote(
    story: &StoryFacts,
    catalog: Option<&FactCatalog>,
    q: &Quote,
) -> Result<QuotePlan, (usize, String)> {
    if q.alert {
        return Err((0, "unported alert presentation".into()));
    }
    if q.text
        .iter()
        .any(|text| ["UNKNOWN", "CLASSIFIED", "SOUND"].iter().any(|s| text.eq_ignore_ascii_case(s)))
    {
        return Err((0, "unported quote sentinel".into()));
    }
    let mut writes = BTreeMap::new();
    let mut plan = QuotePlan { effects: Vec::new(), following: None };
    for (i, op) in q.operations.iter().enumerate() {
        match op {
            DialogueOp::Unsupported { opcode, .. } => {
                return Err((i, format!("unported mandatory opcode {opcode}")));
            }
            DialogueOp::SetFact(id, value) => {
                if !catalog.is_some_and(|catalog| StoryFacts::can_set(*id, catalog)) {
                    return Err((i, format!("unported fact assignment {id:?}")));
                }
                writes.insert(*id, *value);
                plan.effects.push(QuoteEffect::SetFact(*id, *value));
            }
            DialogueOp::Conditional { facts, response } => {
                let mut matched = true;
                for (id, expected) in facts {
                    let actual = match writes.get(id) {
                        Some(value) => *value,
                        None => story
                            .get(
                                *id,
                                catalog
                                    .ok_or_else(|| (i, "fact database unavailable".to_string()))?,
                            )
                            .map_err(|e| (i, e))?,
                    };
                    if actual != *expected {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    plan.following = Some(Continuation::Quote(response.clone()));
                    break;
                }
            }
            DialogueOp::Next(response) => {
                plan.following = Some(Continuation::Quote(response.clone()));
                break;
            }
            DialogueOp::Options(words) => {
                plan.following = Some(Continuation::Options(words.clone()));
                break;
            }
            DialogueOp::Mention { category, text } => {
                plan.effects.push(QuoteEffect::Mention(*category, text.clone()))
            }
            DialogueOp::Close => plan.following = Some(Continuation::End),
            _ => {}
        }
    }
    Ok(plan)
}
fn commit_quote(world: &mut World, entity: Entity, quote: u16, plan: QuotePlan) {
    for effect in plan.effects {
        match effect {
            QuoteEffect::SetFact(id, value) => {
                let level = world
                    .query::<&Level>()
                    .single(world)
                    .map_or_else(|_| String::new(), |level| level.path.clone());
                let facts = world.resource::<NpcAssets>().facts.clone();
                world.resource_scope(|world, catalogs: Mut<Assets<FactCatalog>>| {
                    world
                        .resource_mut::<StoryFacts>()
                        .set(id, value, catalogs.get(&facts).unwrap(), level)
                        .expect("planned fact assignment");
                });
            }
            QuoteEffect::Mention(category, text) => learn_topic(world, entity, category, &text),
        }
    }
    if let Some(following) = plan.following {
        let next = match following {
            Continuation::Quote(response) => Step::Quote {
                quote: select_response(&response, &mut world.resource_mut::<GameRng>()),
                subquote: 0,
                paid: false,
                plan: None,
            },
            Continuation::Options(words) => Step::Options { quote, words },
            Continuation::End => Step::End,
        };
        world.get_mut::<Conversation>(entity).unwrap().queue.push_front(next);
    }
}
fn run(world: &mut World, entity: Entity) {
    let c = world.get::<Conversation>(entity).unwrap();
    if c.wait != ConversationWait::Ready
        || c.queue.is_empty()
        || definition(world, c.actor).is_err()
    {
        return;
    }
    let handle = c.script.clone();
    world.resource_scope(|world, scripts: Mut<Assets<DialogueScript>>| {
        // Asset loading is not a terminal dialogue error. Keep the queued work
        // and retry when its immutable definition is available.
        if let Some(script) = scripts.get(&handle) {
            run_script(world, entity, script);
        }
    });
}
fn run_script(world: &mut World, entity: Entity, script: &DialogueScript) {
    let actor = world.get::<Conversation>(entity).unwrap().actor;
    // Bound immediate authored transitions, including malicious empty-quote loops.
    for _ in 0..256 {
        if world.get::<Conversation>(entity).unwrap().wait != ConversationWait::Ready {
            return;
        }
        let Some(step) = world.get_mut::<Conversation>(entity).unwrap().queue.pop_front() else {
            return;
        };
        match step {
            Step::End => {
                world.get_mut::<Conversation>(entity).unwrap().queue.push_front(Step::End);
                return;
            }
            // The NPC joins the party once its acceptance is said.
            Step::Join => {
                if let Err(refusal) = super::recruit::join(world, actor) {
                    warn!("NPC did not join after all: {refusal:?}");
                }
            }
            Step::Options { quote, words } => {
                let mut c = world.get_mut::<Conversation>(entity).unwrap();
                c.wait = ConversationWait::Options { quote, words };
                c.revision += 1;
                return;
            }
            Step::Price { quote, gold, always } => {
                let mut c = world.get_mut::<Conversation>(entity).unwrap();
                c.wait = ConversationWait::Price { quote, gold, always };
                c.revision += 1;
                return;
            }
            Step::Quote { quote, subquote, paid, plan } => {
                let Some(q) = script.quotes.get(quote as usize) else {
                    block(
                        &mut world.get_mut::<Conversation>(entity).unwrap(),
                        quote,
                        None,
                        "missing quote".into(),
                    );
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
                    && (always || !world.get::<Npc>(actor).unwrap().paid_topics.contains(&quote))
                {
                    let gold =
                        (gold - gold.saturating_mul(party_communication(world)) / 500).max(1);
                    let mut c = world.get_mut::<Conversation>(entity).unwrap();
                    c.queue.push_front(Step::Price { quote, gold, always });
                    c.queue.push_front(Step::Quote {
                        quote: PRICE_QUOTE,
                        subquote: 0,
                        paid: true,
                        plan: None,
                    });
                    continue;
                }
                let plan = match plan.map_or_else(
                    || plan_quote(world.resource::<StoryFacts>(), fact_catalog(world), q),
                    Ok,
                ) {
                    Ok(plan) => plan,
                    Err((entry, reason)) => {
                        let mut c = world.get_mut::<Conversation>(entity).unwrap();
                        c.queue.push_front(Step::Quote { quote, subquote, paid, plan: None });
                        block(&mut c, quote, Some(entry), reason);
                        return;
                    }
                };
                if quote == 0 {
                    let first = world.get::<Npc>(actor).unwrap().greeting_pending;
                    if first && let Ok(definition) = definition(world, actor) {
                        let name = definition.source_name.clone();
                        learn_topic(world, entity, TopicCategory::Person, &name);
                    }
                    world.get_mut::<Npc>(actor).unwrap().greeting_pending = false;
                }
                world.get_mut::<Conversation>(entity).unwrap().current = Some((quote, subquote));
                if subquote + 1 >= q.text.len() {
                    commit_quote(world, entity, quote, plan);
                } else {
                    world.get_mut::<Conversation>(entity).unwrap().queue.push_front(Step::Quote {
                        quote,
                        subquote: subquote + 1,
                        paid: true,
                        plan: Some(plan),
                    })
                }
                let text = q.text.get(subquote).cloned().unwrap_or_default();
                if text.is_empty()
                    || text.eq_ignore_ascii_case("EMPTY")
                    || text.eq_ignore_ascii_case("BLANK")
                {
                    continue;
                }
                let Ok(def) = definition(world, actor) else {
                    block(
                        &mut world.get_mut::<Conversation>(entity).unwrap(),
                        quote,
                        None,
                        "NPC definition unavailable".into(),
                    );
                    return;
                };
                let suffix = if subquote == 0 {
                    String::new()
                } else {
                    char::from(b'a' + (subquote - 1) as u8).to_string()
                };
                let voice = format!("{}_{quote:03}{suffix}", def.voice_stem());
                let seconds = text.encode_utf16().count() as f32 * 0.06 + 2.0;
                let line = TranscriptLine::Npc(text.clone());
                let speech =
                    world.spawn((ConversationSpeech { voice, text }, ChildOf(entity))).id();
                let mut c = world.get_mut::<Conversation>(entity).unwrap();
                c.transcript.push(line);
                c.wait = ConversationWait::Line {
                    speech,
                    phase: LinePhase::Text { remaining: seconds },
                };
                c.revision += 1;
                return;
            }
        }
    }
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    let quote = c.current.map_or(0, |v| v.0);
    block(&mut c, quote, None, "script made no bounded progress".into());
}
fn learn_topic(world: &mut World, entity: Entity, category: TopicCategory, text: &str) {
    if world.resource_mut::<StoryFacts>().remember_topic(category, text) {
        world
            .get_mut::<Conversation>(entity)
            .unwrap()
            .learned_topics
            .insert(text.trim().to_ascii_lowercase());
    }
}
/// The script's quotes answering a request to join (retail
/// `RequestNpcJoinParty`): it will not, the party has no room, it will.
const DECLINES_TO_JOIN: u16 = 0xb;
const NO_ROOM_TO_JOIN: u16 = 0xc;
const JOINS: u16 = 0xd;
/// The party asks the NPC to join. It answers from its script, and joining,
/// leaves the conversation.
fn ask_to_join(world: &mut World, entity: Entity) {
    let c = world.get::<Conversation>(entity).unwrap();
    if !c.ready() {
        return;
    }
    let actor = c.actor;
    let answer = super::recruit::joinable(world, actor);
    let quote = match answer {
        Ok(()) => JOINS,
        Err(super::recruit::JoinRefusal::NoRoom) => NO_ROOM_TO_JOIN,
        Err(super::recruit::JoinRefusal::Declines) => DECLINES_TO_JOIN,
    };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
    if answer.is_ok() {
        c.queue.extend([Step::Join, Step::End]);
    }
    c.revision += 1;
}
fn topic(world: &mut World, entity: Entity, text: &str) {
    let c = world.get::<Conversation>(entity).unwrap();
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
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.transcript.push(TranscriptLine::Question(text.to_owned()));
        c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
        c.revision += 1;
    } else {
        world.get_mut::<Conversation>(entity).unwrap().report(Outcome::NoTopic);
    }
}
fn reply(world: &mut World, entity: Entity, text: &str) {
    let c = world.get::<Conversation>(entity).unwrap();
    let Some(script) = script(world, c) else { return };
    if let ConversationWait::Options { quote, words } = &c.wait {
        let matched = words.iter().any(|w| w.eq_ignore_ascii_case(text));
        let Some(entries) = script.quotes.get(usize::from(*quote)).map(|q| &q.operations) else {
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
            let mut c = world.get_mut::<Conversation>(entity).unwrap();
            c.transcript.push(TranscriptLine::Reply(text.to_owned()));
            c.wait = ConversationWait::Ready;
            c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
            c.revision += 1;
        }
    }
}
fn answer_price(world: &mut World, entity: Entity, accept: bool) {
    let Some(party) = the_party(world) else { return };
    let c = world.get::<Conversation>(entity).unwrap();
    let ConversationWait::Price { quote, gold, always } = c.wait else { return };
    let actor = c.actor;
    if definition(world, actor).is_err() {
        return;
    }
    let handle = c.script.clone();
    world.resource_scope(|world, scripts: Mut<Assets<DialogueScript>>| {
        let Some(line) = scripts.get(&handle).and_then(|s| s.quotes.get(usize::from(quote))) else {
            return;
        };
        if accept {
            let (next, plan) = if world.get::<PartyInventory>(party).unwrap().gold < gold {
                (TOO_POOR_QUOTE, None)
            } else {
                let plan =
                    match plan_quote(world.resource::<StoryFacts>(), fact_catalog(world), line) {
                        Ok(plan) => plan,
                        Err((entry, reason)) => {
                            block(
                                &mut world.get_mut::<Conversation>(entity).unwrap(),
                                quote,
                                Some(entry),
                                reason,
                            );
                            return;
                        }
                    };
                world.get_mut::<PartyInventory>(party).unwrap().gold -= gold;
                if !always {
                    world.get_mut::<Npc>(actor).unwrap().paid_topics.push(quote);
                }
                (quote, Some(plan))
            };
            world.get_mut::<Conversation>(entity).unwrap().queue.push_back(Step::Quote {
                quote: next,
                subquote: 0,
                paid: true,
                plan,
            });
        } else {
            let mut c = world.get_mut::<Conversation>(entity).unwrap();
            for op in &line.operations {
                if let DialogueOp::Decline(quote) = op {
                    c.queue.push_back(Step::Quote {
                        quote: *quote,
                        subquote: 0,
                        paid: false,
                        plan: None,
                    });
                }
            }
        }
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.wait = ConversationWait::Ready;
        c.revision += 1;
    });
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
    let actor = c.actor;
    let revision = c.revision;
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    let communication = party_communication(world);
    let def = definition(world, actor)?;
    let catalog = GameData::items(world).ok_or(TradeError::MissingContent)?;
    let gold = match source {
        TradeSource::Stock(lot) => {
            let stock = &world.get::<Npc>(actor).ok_or(TradeError::Unavailable)?.stock;
            trade::purchase_price(stock, def, catalog, lot, quantity, communication)
        }
        _ => {
            let item = sale_item(world, source, party)?;
            trade::validate_sale(def, item, quantity, catalog, communication)
        }
    }?;
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
fn execute_trade(world: &mut World, quote: TradeQuote) -> Result<(), TradeError> {
    let in_battle = crate::game::combat::in_battle(world);
    let c = world.get::<Conversation>(quote.conversation).ok_or(TradeError::StaleSelection)?;
    if !c.ready() || c.revision != quote.revision {
        return Err(TradeError::StaleSelection);
    }
    if !available(world, c.actor) || !trade_allowed(world, c.actor) || in_battle {
        return Err(TradeError::Unavailable);
    }
    let actor = c.actor;
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    let communication = party_communication(world);
    with_definition(world, actor, |world, def| {
        with_items(world, |world, catalog| match quote.source {
            TradeSource::Stock(lot) => world.resource_scope(|world, mut ids: Mut<ItemIds>| {
                let [mut party_entity, mut actor] =
                    world.get_entity_mut([party, actor]).map_err(|_| TradeError::Unavailable)?;
                let mut inventory =
                    party_entity.get_mut::<PartyInventory>().ok_or(TradeError::Unavailable)?;
                let mut npc = actor.get_mut::<Npc>().ok_or(TradeError::Unavailable)?;
                trade::buy(
                    &mut npc.stock,
                    &mut inventory,
                    &mut ids,
                    def,
                    catalog,
                    lot,
                    quote.quantity,
                    quote.gold,
                    communication,
                )
            }),
            TradeSource::Party(source) => {
                let item = sale_item(world, TradeSource::Party(source), party)?;
                let units =
                    u16::try_from(quote.quantity).map_err(|_| TradeError::InvalidQuantity)?;
                let gold = world
                    .get::<PartyInventory>(party)
                    .ok_or(TradeError::Unavailable)?
                    .gold
                    .checked_add(quote.gold)
                    .ok_or(TradeError::Overflow)?;
                let merchant = &world.get::<Npc>(actor).ok_or(TradeError::Unavailable)?.stock;
                let stock = trade::sell(
                    merchant,
                    def,
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
                world.get_mut::<Npc>(actor).unwrap().stock = stock;
                world.get_mut::<PartyInventory>(party).unwrap().gold = gold;
                Ok(())
            }
        })
    })?;
    world.get_mut::<Conversation>(quote.conversation).unwrap().revision += 1;
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
            | ConversationRequest::Recruit { conversation, .. }
            | ConversationRequest::Advance { conversation, .. }
            | ConversationRequest::Close { conversation } => *conversation,
            ConversationRequest::Trade(q) => q.conversation,
            _ => unreachable!(),
        };
        if matches!(request, ConversationRequest::Close { .. }) {
            close_one(world, entity);
            continue;
        }
        let in_battle = crate::game::combat::in_battle(world);
        let Some(c) = world.get::<Conversation>(entity) else { continue };
        if !available(world, c.actor)
            || in_battle
            || world.contains_resource::<crate::game::camp::Camp>()
        {
            close_one(world, entity);
            continue;
        }
        let revision = c.revision;
        match request {
            ConversationRequest::Topic { revision: requested, text, .. }
                if requested == revision =>
            {
                topic(world, entity, &text)
            }
            ConversationRequest::Reply { revision: requested, text, .. }
                if requested == revision =>
            {
                reply(world, entity, &text)
            }
            ConversationRequest::PriceAnswer { revision: requested, accept, .. }
                if requested == revision =>
            {
                answer_price(world, entity, accept)
            }
            ConversationRequest::Recruit { revision: requested, .. } if requested == revision => {
                ask_to_join(world, entity)
            }
            ConversationRequest::Advance { speech, .. } => {
                if matches!(world.get::<Conversation>(entity).unwrap().wait,ConversationWait::Line{speech:s,..} if s==speech)
                {
                    world.despawn(speech);
                    let mut c = world.get_mut::<Conversation>(entity).unwrap();
                    c.wait = ConversationWait::Ready;
                    c.revision += 1;
                }
            }
            ConversationRequest::Trade(q) => {
                let outcome = match execute_trade(world, q) {
                    Ok(()) => Outcome::Traded,
                    Err(e) => Outcome::TradeRefused(e),
                };
                world.get_mut::<Conversation>(entity).unwrap().report(outcome);
            }
            _ => {}
        }
    }
    let playback: Vec<_> = world.resource_mut::<Messages<LinePlayback>>().drain().collect();
    let delta = world.resource::<Time<Real>>().delta_secs();
    let sessions: Vec<_> =
        world.query_filtered::<Entity, With<Conversation>>().iter(world).collect();
    for entity in sessions {
        let in_battle = crate::game::combat::in_battle(world);
        let Some(c) = world.get::<Conversation>(entity) else { continue };
        if !available(world, c.actor) || in_battle {
            close_one(world, entity);
            continue;
        }
        // No mutable access for an audio wait without a playback event.
        if let ConversationWait::Line { speech, phase } = &c.wait
            && (!matches!(phase, LinePhase::Audio) || playback.iter().any(|e| e.speech == *speech))
        {
            let fallback = world
                .get::<ConversationSpeech>(*speech)
                .map_or(2.0, |line| line.text.encode_utf16().count() as f32 * 0.06 + 2.0);
            let finished = advance_playback(
                &mut world.get_mut::<Conversation>(entity).unwrap(),
                &playback,
                delta,
                fallback,
            );
            if let Some(speech) = finished {
                world.despawn(speech);
            }
        }
        run(world, entity);
        let c = world.get::<Conversation>(entity).unwrap();
        if matches!(c.queue.front(), Some(Step::End)) && c.wait == ConversationWait::Ready {
            close_one(world, entity);
        }
    }
}
fn advance_playback(
    c: &mut Conversation,
    playback: &[LinePlayback],
    delta: f32,
    fallback: f32,
) -> Option<Entity> {
    let ConversationWait::Line { speech, phase } = &mut c.wait else { return None };
    for event in playback.iter().filter(|e| e.speech == *speech) {
        match event.event {
            PlaybackEvent::Started if matches!(phase, LinePhase::Text { .. }) => {
                *phase = LinePhase::Audio
            }
            PlaybackEvent::Finished if matches!(phase, LinePhase::Audio) => {
                *phase = LinePhase::Hold { remaining: 0.7 }
            }
            PlaybackEvent::Unavailable if !matches!(phase, LinePhase::Hold { .. }) => {
                *phase = LinePhase::Text { remaining: fallback }
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
        let speech = *speech;
        c.wait = ConversationWait::Ready;
        c.revision += 1;
        Some(speech)
    } else {
        None
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
                highlight_when_true: false,
                visibility: crate::content::npcs::JournalVisibility::AllDifficulties,
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
                ..default()
            },))
            .id();
        w.spawn((
            super::super::NpcBody(actor),
            crate::game::world::level::LevelOwned(level),
            Monster {
                enchantments: default(),
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
                queue: VecDeque::from([Step::Quote {
                    quote: 0,
                    subquote: 0,
                    paid: false,
                    plan: None,
                }]),
            })
            .id();
        (app, entity, actor)
    }
    /// Asked to join, an NPC answers from its script: one with no character
    /// to bring declines, a party whose NPC places are taken has no room, and
    /// a willing one accepts and then joins.
    #[test]
    fn a_request_to_join_is_answered_from_the_script() {
        let answer = |character: bool, companions: usize| {
            let quotes =
                (0..16).map(|i| Quote { text: vec![format!("quote {i}")], ..default() }).collect();
            let (mut app, session, _) = app(DialogueScript { quotes, ..default() });
            let w = app.world_mut();
            if character {
                let handle = w.resource::<NpcAssets>().catalog.clone();
                w.resource_mut::<Assets<NpcCatalog>>().get_mut(&handle).unwrap().0[0].character =
                    Some(crate::content::npcs::NpcCharacter {
                        name: "Speaker".into(),
                        profession: crate::game::characters::Profession::Fighter,
                        race: crate::game::characters::Race::Human,
                        gender: crate::game::characters::Gender::Male,
                        portrait: 0,
                        level: 1,
                        attributes: default(),
                        skills: default(),
                        spells: vec![],
                        equipment: [None; 12],
                        backpack: [None; 8],
                        wanted: vec![],
                    });
            }
            for _ in 0..companions {
                let companion = w.spawn(Npc::default()).id();
                crate::game::party::roster::test_join(w, companion);
            }
            let mut c = w.get_mut::<Conversation>(session).unwrap();
            c.queue.clear();
            let revision = c.revision;
            w.write_message(ConversationRequest::Recruit { conversation: session, revision });
            app.update();
            let c = app.world().get::<Conversation>(session).unwrap();
            (c.current, c.queue.iter().any(|step| matches!(step, Step::Join)))
        };
        assert_eq!(answer(false, 0), (Some((DECLINES_TO_JOIN, 0)), false));
        assert_eq!(answer(true, 2), (Some((NO_ROOM_TO_JOIN, 0)), false));
        assert_eq!(answer(true, 1), (Some((JOINS, 0)), true));
    }

    #[test]
    fn fact_writes_select_the_validated_branch_before_payment_or_effects() {
        for (initial, assigned) in [(0, 1), (1, 0)] {
            for branch_is_taken in [false, true] {
                let expected = if branch_is_taken { assigned } else { initial };
                let (mut app, session, actor) = app(DialogueScript {
                    quotes: vec![Quote {
                        text: vec!["Purchased answer".into()],
                        operations: vec![
                            DialogueOp::SetFact(FactId(91), assigned),
                            DialogueOp::Conditional {
                                facts: vec![(FactId(91), expected)],
                                response: Response::Random { first: 1, last: 2 },
                            },
                            DialogueOp::Unsupported {
                                opcode: 99,
                                operands: [0; 3],
                                subentries: vec![],
                            },
                        ],
                        ..default()
                    }],
                    ..default()
                });
                let world = app.world_mut();
                let party = the_party(world).unwrap();
                world.get_mut::<PartyInventory>(party).unwrap().gold = 100;
                world.resource_mut::<StoryFacts>().values.insert(FactId(91), initial);
                {
                    let mut c = world.get_mut::<Conversation>(session).unwrap();
                    c.queue.clear();
                    c.wait = ConversationWait::Price { quote: 0, gold: 25, always: false };
                }
                let rng = world.resource::<GameRng>().clone();
                world.write_message(ConversationRequest::PriceAnswer {
                    conversation: session,
                    revision: 0,
                    accept: true,
                });
                app.update();
                let world = app.world();
                let c = world.get::<Conversation>(session).unwrap();
                if branch_is_taken {
                    assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 75);
                    assert_eq!(world.get::<Npc>(actor).unwrap().paid_topics, [0]);
                    assert_eq!(world.resource::<StoryFacts>().values[&FactId(91)], assigned);
                    assert_eq!(world.resource::<StoryFacts>().journal.len(), 1);
                    assert!(matches!(c.queue.front(), Some(Step::Quote { quote: 1 | 2, .. })));
                    assert_eq!(c.transcript.len(), 1);
                } else {
                    assert!(matches!(c.wait, ConversationWait::Blocked { entry: Some(2), .. }));
                    assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 100);
                    assert!(world.get::<Npc>(actor).unwrap().paid_topics.is_empty());
                    assert!(world.get::<Npc>(actor).unwrap().greeting_pending);
                    assert_eq!(world.resource::<StoryFacts>().values[&FactId(91)], initial);
                    assert!(world.resource::<StoryFacts>().journal.is_empty());
                    assert_eq!(world.resource::<GameRng>(), &rng);
                    assert!(c.queue.is_empty());
                    assert!(c.transcript.is_empty());
                    assert_eq!(c.current, None);
                }
            }
        }
    }

    #[test]
    fn repeated_assignments_keep_order_and_accepted_path_across_subquotes() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![Quote {
                text: vec!["First line".into(), "Last line".into()],
                operations: vec![
                    DialogueOp::SetFact(FactId(91), 1),
                    DialogueOp::Conditional {
                        facts: vec![(FactId(91), 0)],
                        response: Response::Fixed(2),
                    },
                    DialogueOp::SetFact(FactId(91), 0),
                    DialogueOp::Conditional {
                        facts: vec![(FactId(91), 0)],
                        response: Response::Fixed(1),
                    },
                    DialogueOp::Unsupported { opcode: 99, operands: [0; 3], subentries: vec![] },
                ],
                ..default()
            }],
            ..default()
        });
        app.update();
        assert!(app.world().resource::<StoryFacts>().journal.is_empty());
        let speech = match app.world().get::<Conversation>(session).unwrap().wait {
            ConversationWait::Line { speech, .. } => speech,
            _ => panic!("first line expected"),
        };
        app.world_mut().resource_mut::<StoryFacts>().values.insert(FactId(91), 0);
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        let world = app.world();
        assert_eq!(
            world.resource::<StoryFacts>().journal.iter().map(|j| j.was_true).collect::<Vec<_>>(),
            [true, false]
        );
        let c = world.get::<Conversation>(session).unwrap();
        assert_eq!(c.current, Some((0, 1)));
        assert!(matches!(c.queue.front(), Some(Step::Quote { quote: 1, .. })));
    }

    #[test]
    fn accepted_payment_keeps_its_path_until_the_last_subquote() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![Quote {
                text: vec!["First".into(), "Last".into()],
                operations: vec![
                    DialogueOp::Conditional {
                        facts: vec![(FactId(91), 0)],
                        response: Response::Fixed(1),
                    },
                    DialogueOp::Unsupported { opcode: 99, operands: [0; 3], subentries: vec![] },
                ],
                ..default()
            }],
            ..default()
        });
        let world = app.world_mut();
        let party = the_party(world).unwrap();
        world.get_mut::<PartyInventory>(party).unwrap().gold = 100;
        {
            let mut c = world.get_mut::<Conversation>(session).unwrap();
            c.queue.clear();
            c.wait = ConversationWait::Price { quote: 0, gold: 25, always: false };
        }
        answer_price(world, session, true);
        // Another conversation can change global facts before this accepted
        // answer runs, and again while its speech is playing.
        world.resource_mut::<StoryFacts>().values.insert(FactId(91), 1);
        app.update();
        let speech = match app.world().get::<Conversation>(session).unwrap().wait {
            ConversationWait::Line { speech, .. } => speech,
            _ => panic!("accepted answer must still play"),
        };
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        let world = app.world();
        let c = world.get::<Conversation>(session).unwrap();
        assert_eq!(c.current, Some((0, 1)));
        assert!(matches!(c.queue.front(), Some(Step::Quote { quote: 1, .. })));
        assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 75);
    }

    #[test]
    fn idle_and_delayed_assets_do_not_replace_or_dirty_conversations() {
        #[derive(Resource, Default)]
        struct Lifecycle {
            added: usize,
            removed: usize,
            changed: usize,
        }
        let (mut app, session, actor) =
            app(DialogueScript { quotes: vec![Quote::default()], ..default() });
        app.init_resource::<Lifecycle>()
            .add_observer(|_: On<Add<Conversation>>, mut counts: ResMut<Lifecycle>| {
                counts.added += 1
            })
            .add_observer(|_: On<Remove<Conversation>>, mut counts: ResMut<Lifecycle>| {
                counts.removed += 1
            })
            .add_systems(
                Update,
                (|changed: Query<(), Changed<Conversation>>, mut counts: ResMut<Lifecycle>| {
                    counts.changed += changed.iter().count()
                })
                .after(update_conversation),
            );
        let handle = app.world().get::<Conversation>(session).unwrap().script.clone();
        let script =
            app.world_mut().resource_mut::<Assets<DialogueScript>>().remove(handle.id()).unwrap();
        app.update();
        let changed = app.world().resource::<Lifecycle>().changed;
        app.update();
        assert_eq!(app.world().resource::<Lifecycle>().changed, changed);
        assert_eq!(app.world().get::<Conversation>(session).unwrap().queue.len(), 1);
        app.world_mut()
            .resource_mut::<Assets<DialogueScript>>()
            .insert(handle.id(), script)
            .unwrap();
        app.update();
        assert!(app.world().get::<Conversation>(session).unwrap().ready());
        let changed = app.world().resource::<Lifecycle>().changed;
        app.update();
        let counts = app.world().resource::<Lifecycle>();
        assert_eq!(counts.changed, changed);
        assert_eq!((counts.added, counts.removed), (0, 0));
        app.world_mut().get_mut::<Npc>(actor).unwrap().disposition = 0;
        app.update();
        assert!(app.world().get::<Conversation>(session).is_none());
        assert_eq!(app.world().resource::<Lifecycle>().removed, 1);
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn burz_retail_arnika_reply_commits_its_authored_fact() {
        let files = crate::retail::files::retail_files();
        let script = crate::retail::npcs::read_dialogue(
            &files.read("data/npc scripts/npc_burz.nsf").unwrap().unwrap(),
        )
        .unwrap();
        let facts = crate::retail::npcs::read_facts(
            &files.read("data/databases/fact.dbs").unwrap().unwrap(),
        )
        .unwrap();
        let said_arnika = facts.0.iter().find(|f| f.symbol == "FACT_BURZ_SAID130").unwrap().id;
        // Retail quote 129 asks whether the party has visited Arnika;
        // answering NO selects quote 130 and records that advice was given.
        const ASKED_ABOUT_ARNIKA: u16 = 129;
        const ARNIKA_ADVICE: u16 = 130;
        let expected = script.quotes[usize::from(ARNIKA_ADVICE)].text[0].clone();
        let (mut app, session, _) = app(script);
        let handle = app.world_mut().resource_mut::<Assets<FactCatalog>>().add(facts);
        app.world_mut().resource_mut::<NpcAssets>().facts = handle;
        {
            let mut c = app.world_mut().get_mut::<Conversation>(session).unwrap();
            c.queue.clear();
            c.wait = ConversationWait::Options {
                quote: ASKED_ABOUT_ARNIKA,
                words: vec!["YES".into(), "NO".into()],
            };
        }
        app.world_mut().write_message(ConversationRequest::Reply {
            conversation: session,
            revision: 0,
            text: "NO".into(),
        });
        app.update();
        let world = app.world();
        assert_eq!(world.resource::<StoryFacts>().values[&said_arnika], 1);
        assert_eq!(world.resource::<StoryFacts>().journal.len(), 1);
        let c = world.get::<Conversation>(session).unwrap();
        assert_eq!(c.current, Some((ARNIKA_ADVICE, 0)));
        assert!(
            matches!(c.transcript.last(), Some(TranscriptLine::Npc(text)) if text == &expected)
        );
    }

    #[test]
    fn journal_keeps_transitions_and_selects_retail_text_without_rewriting_history() {
        let catalog = FactCatalog(vec![FactDefinition {
            id: FactId(91),
            symbol: "FACT_BURZ_SAID130".into(),
            description: "Unfinished".into(),
            alternate_description: "Finished".into(),
            visibility: crate::content::npcs::JournalVisibility::NoviceOnly,
            highlight_when_true: true,
        }]);
        let mut facts = StoryFacts::default();
        facts.set(FactId(91), 1, &catalog, "first".into()).unwrap();
        facts.set(FactId(91), 1, &catalog, "second".into()).unwrap();
        facts.set(FactId(91), 0, &catalog, "second".into()).unwrap();
        assert_eq!(facts.journal.len(), 2);
        assert_eq!(facts.journal[0].description(&catalog), Some("Finished"));
        assert_eq!(facts.journal[0].level, "first");
        assert_eq!(facts.journal[1].description(&catalog), Some("Unfinished"));
        assert_eq!(facts.journal[1].level, "second");
        assert_eq!(facts.get(FactId(91), &catalog), Ok(0));
        assert!(facts.set(FactId(999), 1, &catalog, "third".into()).is_err());
        assert_eq!(facts.journal.len(), 2);
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
        world.get_mut::<Conversation>(session).unwrap().queue.clear();
        for source in [
            TradeSource::Stock(trade::StockLotId(1)),
            TradeSource::Party(ItemRef {
                owner: ItemOwner::Pool,
                id: crate::game::inventory::StackId(1),
            }),
        ] {
            assert!(!trade_allowed(world, actor));
            assert_eq!(quote_trade(world, session, source, 1), Err(TradeError::Unavailable));
            let c = world.get::<Conversation>(session).unwrap();
            let quote = TradeQuote {
                conversation: session,
                revision: c.revision,
                source,
                quantity: 1,
                gold: 1,
            };
            assert_eq!(execute_trade(world, quote), Err(TradeError::Unavailable));
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
