//! Authored continuation and waits belong to gameplay, not the dialogue view.
use super::{
    Npc, definition,
    trade::{self, TradeError, TradeQuote, TradeSource},
};
use crate::content::factions::Faction;
use crate::content::{
    items::{ItemCatalog, ItemId},
    npcs::*,
};
use crate::game::data::Catalogs;
use crate::game::factions::{Band, Factions};
use crate::game::time::GameClock;
use crate::game::{
    characters::{Character, Condition, Vitals},
    inventory::{Carried, ItemIds, ItemOwner, ItemRef, PartyInventory, make_item},
    party::roster::{PartyRoster, the_party},
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
    pub fn description<'a>(&self, catalog: &'a FactCatalog) -> &'a str {
        let fact = &catalog[self.fact];
        if self.was_true { &fact.alternate_description } else { &fact.description }
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

    /// Forget a notebook entry without changing discovered facts or journal history.
    pub fn forget_topic(&mut self, text: &str) -> bool {
        let text = text.trim();
        let before = self.topics.len();
        self.topics.retain(|(_, known)| !known.eq_ignore_ascii_case(text));
        self.topics.len() != before
    }

    /// A fact's value before anything sets it: a new game starts its
    /// virgin facts true.
    fn initial(fact: &crate::content::npcs::FactDefinition) -> u8 {
        u8::from(matches!(
            fact.symbol.as_str(),
            "FACT_VIRGIN" | "FACT_QUEST_VIRGIN_1" | "FACT_QUEST_VIRGIN_2" | "FACT_QUEST_VIRGIN_3"
        ))
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
            | "FACT_PEACE_ACHIEVED" => {
                Ok(self.values.get(&id).copied().unwrap_or(Self::initial(fact)))
            }
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
        let initial = catalog.get(id).map_or(0, Self::initial);
        let previous = self.values.insert(id, value).unwrap_or(initial);
        if previous != value {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level });
        }
        Ok(())
    }
}
fn fact_catalog(world: &World) -> &FactCatalog {
    &world.resource::<Catalogs>().facts
}
/// A fact by its symbol, for rules that name one.
pub fn fact_named(world: &World, symbol: &str) -> Result<u8, String> {
    let id = fact_catalog(world)
        .0
        .iter()
        .find(|fact| fact.symbol == symbol)
        .ok_or_else(|| format!("missing fact {symbol}"))?
        .id;
    fact(world, id)
}
fn fact(world: &World, id: FactId) -> Result<u8, String> {
    world.resource::<StoryFacts>().get(id, fact_catalog(world))
}
#[derive(Debug)]
enum Step {
    Quote {
        quote: u16,
        subquote: usize,
        paid: bool,
        plan: Option<QuotePlan>,
    },
    /// The party is asked for an answer.
    Wait(ConversationWait),
    Join,
    End,
}
impl Step {
    /// A quote from its first line, asking its price first if it has one.
    fn quote(quote: u16) -> Self {
        Step::Quote { quote, subquote: 0, paid: false, plan: None }
    }
    /// A quote from its first line, said without asking a price.
    fn said(quote: u16) -> Self {
        Step::Quote { quote, subquote: 0, paid: true, plan: None }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationWait {
    Ready,
    Line {
        speech: Entity,
    },
    /// The party answers with one of `words`, choosing it, or typing it
    /// when `typed`.
    Options {
        quote: u16,
        words: Vec<String>,
        typed: bool,
    },
    Price {
        quote: u16,
        gold: u32,
        always: bool,
    },
    /// The party is asked whether to give the NPC all its gold (retail
    /// string 0x7d6), answered as a price is.
    GiveAll {
        gold: u32,
    },
    Blocked {
        quote: u16,
        entry: Option<usize>,
        reason: String,
    },
}
/// Accepted dialogue history, including the party's input. Presentation chooses
/// the retail palette without guessing speakers from the text.
#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptLine {
    Npc(String),
    Question(String),
    Reply(String),
    Reward(QuestReward),
    /// The NPC has nothing to say to the question: retail string 1867, or
    /// 1868 for the `second`, which every NPC takes in turn.
    NoAnswer {
        npc: String,
        second: bool,
    },
}
/// Committed outcomes, not commands. Presentation supplies retail wording.
#[derive(Clone, Debug, PartialEq)]
pub enum QuestReward {
    Gold {
        giver: String,
        amount: u32,
    },
    Experience {
        amount_each: u32,
    },
    /// The NPC handed the party this item.
    Item {
        name: String,
    },
    /// One of an item left this member's pack, or the party's items.
    Taken {
        member: Option<String>,
    },
}
#[derive(Component, Debug)]
pub struct Conversation {
    pub actor: Entity,
    /// Temporary effects belong to the affected speakers. Closing the
    /// conversation clears all of them; handing it over transfers none.
    charms: BTreeMap<Entity, i8>,
    pub current: Option<(u16, usize)>,
    pub wait: ConversationWait,
    pub transcript: Vec<TranscriptLine>,
    /// Keywords discovered during this conversation are green. Reopening
    /// imports the persistent notebook without the transient discovery marks.
    pub learned_topics: BTreeSet<String>,
    /// The NPC is not friendly yet: the party sees retail's topic menu
    /// (charm, small talk, bribe) rather than the dialogue's topics.
    pub menu: bool,
    /// What the party's last request came to; presentation owns its
    /// wording.
    result: Option<Outcome>,
    queue: VecDeque<Step>,
}
/// What the party's last request came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NoTopic,
    Traded,
    TradeRefused(TradeError),
    GiftRefused(TradeError),
}
pub(crate) enum MagicalReply {
    Unnoticed,
    Charmed(i8),
    Quote(u16),
}
impl Conversation {
    pub fn ready(&self) -> bool {
        self.wait == ConversationWait::Ready && self.queue.is_empty()
    }

    pub(crate) fn charm(&self) -> i8 {
        self.charms.get(&self.actor).copied().unwrap_or(0)
    }

    pub(crate) fn magical_reply(&mut self, reply: MagicalReply) {
        self.result = None;
        match reply {
            MagicalReply::Unnoticed => {}
            MagicalReply::Charmed(strength) if strength > self.charm() => {
                self.charms.insert(self.actor, strength);
                self.transcript.push(TranscriptLine::Npc("Charmed.".into()));
            }
            MagicalReply::Charmed(_) => {}
            MagicalReply::Quote(quote) => self.queue.push_back(Step::quote(quote)),
        }
    }

    /// What the last request came to, until the party asks something else.
    pub fn outcome(&self) -> Option<Outcome> {
        self.result
    }

    fn report(&mut self, outcome: Outcome) {
        self.result = Some(outcome);
    }
}
/// A line said: the words and the voice stem presentation may play. How
/// long it shows is presentation's to decide.
#[derive(Component, Debug)]
pub struct ConversationSpeech {
    /// The line's recording, if it has one to wait on.
    pub voice: Option<String>,
    pub text: String,
    /// An alert line: the alert chime and the text, without a recording.
    pub alert: bool,
}
/// A party member remarks on the conversation with this character event
/// (retail `QueueCharacterEvent` for `PickRandomPartySpeaker`); presentation
/// picks who and plays it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartyRemark {
    pub event: u16,
}
/// An explicit edit of the persistent keyword notebook, not a spoken question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotebookEdit {
    Remember,
    Forget,
}

/// Start talking to an NPC.
#[derive(Message, Clone, Copy, Debug)]
pub struct BeginConversation {
    pub actor: Entity,
}

/// What the party does in a conversation under way.
#[derive(Message, Clone, Debug)]
pub struct ConversationRequest {
    pub conversation: Entity,
    pub talk: Talk,
}

impl ConversationRequest {
    pub fn new(conversation: Entity, talk: Talk) -> Self {
        Self { conversation, talk }
    }

    /// Settle a quoted trade in the conversation it was quoted in.
    pub fn trade(quote: TradeQuote) -> Self {
        Self::new(quote.conversation, Talk::Trade(quote))
    }
}

#[derive(Clone, Debug)]
pub enum Talk {
    Topic {
        text: String,
    },
    Reply {
        text: String,
    },
    PriceAnswer {
        accept: bool,
    },
    Advance {
        speech: Entity,
    },
    Close,
    Recruit,
    EditNotebook {
        edit: NotebookEdit,
        text: String,
    },
    Trade(TradeQuote),
    /// Hand the NPC an item from a member's pack or the party's items.
    Give {
        item: ItemRef,
    },
    /// Hand the NPC some of the party's gold.
    GiveGold {
        amount: u32,
    },
    /// Try to win over an NPC that is not yet friendly.
    Approach {
        approach: Approach,
    },
}
/// How the party tries to win an NPC over from retail's topic menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    Charm,
    SmallTalk,
}
/// Run `f` with the NPC's definition and the item catalog while the world
/// stays writable; the catalogs are out of the world meanwhile.
fn with_definition<T>(
    world: &mut World,
    actor: Entity,
    f: impl FnOnce(&mut World, &NpcDefinition, &ItemCatalog) -> T,
) -> T {
    let id = world.get::<super::ActorId>(actor).expect("an NPC actor").npc();
    Catalogs::scope(world, |world, catalogs| f(world, &catalogs.npcs[id], &catalogs.items))
}
pub fn party_communication(world: &World) -> u32 {
    best_communicator(world).map_or(0, |(_, level)| level)
}
pub(crate) fn active_body(world: &World, actor: Entity) -> Option<Entity> {
    world.get::<super::NpcBodies>(actor)?.iter().find(|body| {
        world
            .get::<crate::game::world::level::LevelOwned>(*body)
            .is_some_and(|parent| world.get::<Level>(parent.0).is_some())
            && !world.get::<Monster>(*body).unwrap().is_dead()
    })
}
/// What a neutral NPC says first, and its answers to charm, small talk and
/// bribes that leave it unmoved; what a hostile one says to turn the party
/// away; and what one won over by charm says.
const NEUTRAL_GREETING: u16 = 2;
const SMALL_TALK_QUOTE: u16 = 3;
const CHARMED_QUOTE: u16 = 4;
const UNCHARMED_QUOTE: u16 = 6;
const UNBRIBED_QUOTE: u16 = 7;
const HOSTILE_QUOTE: u16 = 0x18;
/// How long the party waits to charm or make small talk with an NPC again
/// (retail `AdvanceNpcTimers`: 0xa8c0 seconds).
const APPROACH_COOLDOWN_MS: u64 = 0xa8c0 * 1000;
/// Whether an approach made at `at` may be tried again at `now`.
pub fn approach_ready(at: Option<u64>, now: u64) -> bool {
    at.is_none_or(|at| now.saturating_sub(at) > APPROACH_COOLDOWN_MS)
}
/// How the NPC stands toward the party now (retail `GetNpcDisposition`):
/// its own disposition, fully friendly while its body is turned. A
/// factioned NPC answers with its faction's score when bound to its monster
/// or when the faction is hostile to the party. Charm is scoped to this
/// speaker in the open conversation; it never overwrites retained opinion.
/// Allied party NPCs are not yet represented.
pub fn disposition(world: &World, actor: Entity) -> i32 {
    let npc = world.get::<Npc>(actor).unwrap();
    standing(world, actor, npc.disposition)
}
/// [`disposition`] were the NPC's own `disposition` this.
fn standing(world: &World, actor: Entity, disposition: u8) -> i32 {
    let turned = active_body(world, actor)
        .and_then(|body| world.get::<Monster>(body))
        .is_some_and(|m| m.conditions.has(Condition::Turncoat));
    let charm = world
        .try_query::<&Conversation>()
        .and_then(|mut q| q.iter(world).find_map(|c| c.charms.get(&actor).copied()))
        .unwrap_or(0);
    let own = if turned { 100 } else { i32::from(disposition.wrapping_add(charm as u8)) };
    let def = definition(world, actor);
    let factions = world.resource::<Factions>();
    let faction = i32::from(def.faction);
    if def.faction != 0 && (def.monster_bound || factions.band(faction) == Band::Hostile) {
        return factions.score(faction).map_or(own, i32::from);
    }
    own
}
/// Retail `GetNpcDispositionBand`: hostile under 0x21, friendly from 0x42.
pub fn band(world: &World, actor: Entity) -> Band {
    band_of(disposition(world, actor))
}
fn band_of(disposition: i32) -> Band {
    match disposition {
        d if d < 0x21 => Band::Hostile,
        d if d < 0x42 => Band::Neutral,
        _ => Band::Friendly,
    }
}
/// Retail `AdjustNpcDisposition`, clamped to 0..99.
fn shift_disposition(npc: &mut Npc, delta: i8) {
    npc.disposition = (i16::from(npc.disposition) + i16::from(delta)).clamp(0, 99) as u8;
}
/// Retail `SetNpcDispositionBand` to hostile.
fn turn_hostile(world: &mut World, actor: Entity) {
    if band(world, actor) != Band::Hostile {
        world.get_mut::<Npc>(actor).unwrap().disposition = 0x19;
    }
}
/// Retail `ResumeNpc` as a dialogue closes on the party: an NPC that is not
/// already hostile becomes so. Retail's NPC then fights with its group; NPCs
/// do not fight here yet, so it only turns the party away from now on.
fn resume(world: &mut World, actor: Entity) {
    if disposition(world, actor) > 0x20 {
        world.get_mut::<Npc>(actor).unwrap().disposition = 0x19;
    }
}
/// The member with the best Communication, the first on a tie, and that
/// level (retail `GetBestPartySkillLevel`).
fn best_communicator(world: &World) -> Option<(Entity, u32)> {
    crate::game::party::Lineup::in_world(world)
        .best(crate::content::characters::Skill::Communication)
}
/// Practise `members`' Communication `usage` times.
fn practise_communication(world: &mut World, members: &[Entity], usage: u32) {
    world.resource_scope(|world, mut rng: Mut<GameRng>| {
        for &member in members {
            world.get_mut::<Character>(member).unwrap().practise(
                crate::content::characters::Skill::Communication,
                usage,
                &mut rng,
            );
        }
    });
}
/// The script's quote asking the price of a paid topic.
const PRICE_QUOTE: u16 = 0x12;
/// Which of retail's two answers for having nothing to say comes next; retail
/// alternates them across every NPC (`g_empty_quote_text_index`).
#[derive(Resource, Debug, Default)]
pub struct NoAnswerTurn(bool);
/// The quotes that may answer with nothing to say, the quotes a line that
/// does not know answers with at random, and the one for a line that will
/// not say (retail `SpeakNpcSubquote`).
const NO_ANSWER_QUOTES: std::ops::RangeInclusive<u16> = 0x67..=0x69;
const UNKNOWN_QUOTES: [u16; 2] = [0x1c, 0x1d];
const CLASSIFIED_QUOTE: u16 = 0x1e;
/// The script's quote for a party that cannot pay.
const TOO_POOR_QUOTE: u16 = 0x14;
fn available(world: &World, actor: Entity) -> bool {
    active_body(world, actor).is_some_and(|body| {
        crate::game::world::hostility::monster_stance(world, body)
            != crate::game::world::hostility::Stance::Hostile
    }) && world.get::<Npc>(actor).is_some()
}
/// Shared gameplay permission, also used to derive the trade controls.
pub fn trade_allowed(world: &World, actor: Entity) -> bool {
    definition(world, actor).owns_stock
}
fn entry_quote(def: &NpcDefinition, npc: &Npc) -> u16 {
    if def.merchant || npc.greeting_pending { 0 } else { 1 }
}
/// What the NPC says to open its dialogue; meeting it for the first time
/// teaches the party's members Communication (retail
/// `HandleNpcDialogueDeparture`).
fn greet(world: &mut World, actor: Entity) -> u16 {
    let def = definition(world, actor);
    let greeting = entry_quote(def, world.get::<Npc>(actor).unwrap());
    if greeting == 0 && !def.monster_bound && !def.voice_script && !def.merchant {
        let members: Vec<_> =
            crate::game::party::Lineup::in_world(world).able().map(|(m, _)| m).collect();
        practise_communication(world, &members, 0xf);
    }
    greeting
}
pub fn begin(world: &mut World, actor: Entity) -> Result<Entity, TradeError> {
    let party = the_party(world);
    if !crate::game::freedom::free_in(world) || crate::game::combat::in_battle(world) {
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
    let party_at = crate::game::party::body::PartyBody::centre(
        crate::game::party::body::PartyBody::eye_in(world),
    );
    if party_at.distance(at) > crate::game::world::items::REACH {
        return Err(TradeError::Unavailable);
    }
    if !available(world, actor) {
        return Err(TradeError::Unavailable);
    }
    let def = definition(world, actor);
    // Retail `BeginNpcDialogueInternal` and `OpenNpcDialoguePanel`: an NPC
    // that speaks through its own script greets as it is; otherwise one
    // hostile to the party only turns it away, a neutral one offers the
    // topic menu, and a merchant always does business.
    let scripted = def.monster_bound || def.voice_script;
    let band = band(world, actor);
    let quote = |quote| VecDeque::from([Step::quote(quote)]);
    let conversation = |queue| Conversation {
        actor,
        charms: default(),
        current: None,
        wait: ConversationWait::Ready,
        transcript: vec![],
        learned_topics: default(),
        menu: false,
        result: None,
        queue,
    };
    if !scripted && !def.merchant && band == Band::Hostile {
        return Ok(world.spawn((conversation(quote(HOSTILE_QUOTE)), Closing)).id());
    }
    let menu = !scripted && !def.merchant && band == Band::Neutral;
    let greeting = if menu { NEUTRAL_GREETING } else { greet(world, actor) };
    let now = world.resource::<GameClock>().ms;
    let average = crate::game::party::Lineup::in_world(world).average_level();
    let def = definition(world, actor);
    if def.owns_stock {
        let stock = &world.get::<Npc>(actor).unwrap().stock;
        let gated = stock.restock_at.is_some()
            && now.saturating_sub(stock.maintenance_at) >= crate::game::time::DAY_MS
            && def.stock.iter().any(|r| !r.persistent && r.item == super::trade::TEMPLAR_GATED);
        let templar = gated && fact_named(world, "FACT_TEMPLAR").expect("the Templar fact") != 0;
        with_definition(world, actor, |world, def, items| {
            world.resource_scope(|world, mut rng: Mut<GameRng>| {
                world
                    .get_mut::<Npc>(actor)
                    .unwrap()
                    .stock
                    .maintain(def, items, average, now, templar, &mut rng)
            })
        });
    }
    // OpenNpcDialoguePanel / MonsterForwardReferencePosition: the accepted
    // speaker faces the party. Physics rotation owns a moving body's facing.
    crate::game::world::face(world, body, party_at);
    Ok(world.spawn(Conversation { menu, ..conversation(quote(greeting)) }).id())
}
/// Travel ends the conversation; the speaker stays.
pub(crate) fn end_conversations(
    _: On<Add<crate::game::world::level::Destination>>,
    conversations: Query<Entity, With<Conversation>>,
    mut commands: Commands,
) {
    for conversation in &conversations {
        commands.queue(move |world: &mut World| close_one(world, conversation));
    }
}
/// Travel, battle or the speaker's absence ends a conversation at once.
/// Retail's message queue would still play the answers already under way,
/// a paid answer or an accepted hand-in among them, so their effects commit
/// as it ends; only their lines are lost.
fn close_one(world: &mut World, entity: Entity) {
    let Some(mut c) = world.get_mut::<Conversation>(entity) else { return };
    let plans: Vec<_> = c
        .queue
        .drain(..)
        .filter_map(|step| match step {
            Step::Quote { plan: Some(plan), .. } => Some(plan),
            _ => None,
        })
        .collect();
    for plan in plans {
        commit_effects(world, entity, plan.effects);
    }
    world.despawn(entity);
}
/// The party has left the dialogue while its lines were still queued.
/// Retail keeps its message queue when the dialogue closes, so they play out
/// in the world and commit their effects; the conversation ends with them,
/// or at the first question the party is no longer there to answer.
#[derive(Component, Debug)]
pub struct Closing;
/// What the speaker of a dialogue the party has left says, for the text box
/// (retail `SpeakNpcSubquote` outside dialogue mode: "Name: text").
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct NpcSpeech {
    pub speaker: String,
    pub text: String,
}
/// The party leaves the dialogue: at once, or once its queued lines have
/// played out.
fn leave(world: &mut World, entity: Entity) {
    let Some(c) = world.get::<Conversation>(entity) else { return };
    let pending = matches!(c.wait, ConversationWait::Line { .. })
        || c.queue.iter().any(|step| matches!(step, Step::Quote { .. }));
    if pending {
        world.entity_mut(entity).insert(Closing);
    } else {
        world.despawn(entity);
    }
}

/// The quote a response leads to in this script; a speaker change leads out
/// of it.
fn select_response(response: &Response, rng: &mut GameRng) -> Option<u16> {
    match *response {
        Response::Fixed(n) => Some(n),
        Response::Random { first, last } => {
            Some(first + rng.below(u32::from(last - first) + 1) as u16)
        }
        Response::Speaker { .. } => None,
    }
}
/// RFS-81 answers to another name once the party has fixed it (retail
/// `GetNpcDisplayName`).
const RFS81: NpcId = NpcId(0x20);
const RFS81_FIXED_NAME: &str = "RFS81B";
/// The NPC a script names by its display name (retail `FindNpcStateByName`).
fn npc_named(world: &World, name: &str) -> Option<Entity> {
    let catalog = super::catalog(world);
    let fixed = fact_named(world, "FACT_RFS81_HAS_BEEN_FIXED").is_ok_and(|value| value != 0);
    world.iter_entities().find_map(|e| {
        let id = e.get::<super::ActorId>()?.npc();
        let shown =
            if id == RFS81 && fixed { RFS81_FIXED_NAME } else { catalog[id].display_name.as_str() };
        shown.eq_ignore_ascii_case(name).then(|| e.id())
    })
}
/// Another NPC takes over the conversation and says `quote` of its own
/// script (retail `ProcessNpcQuoteEntry` for a response naming a speaker).
/// Retail finds the NPC by display name among those it knows; when none
/// answers to it, the conversation simply waits for the party.
fn hand_over(world: &mut World, entity: Entity, name: &str, quote: u16) {
    let Some(speaker) = npc_named(world, name) else { return };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.actor = speaker;
    c.queue.push_front(Step::quote(quote));
}
fn block(c: &mut Conversation, quote: u16, entry: Option<usize>, reason: String) {
    c.wait = ConversationWait::Blocked { quote, entry, reason };
}
/// The speaker's script, which loaded before the conversation began.
pub fn script<'w>(world: &'w World, c: &Conversation) -> &'w DialogueScript {
    world.resource::<Assets<DialogueScript>>().get(super::script(world, c.actor)).unwrap()
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
    Gold { party: Entity, amount: u32 },
    Experience { members: Vec<Entity>, amount: u32 },
    Faction { faction: Faction, delta: i8 },
    GiveItem { party: Entity, item: ItemId },
    TakeItem { party: Entity, item: ItemId },
    Disposition { npc: Entity, delta: i8 },
    PartyRemark(u16),
    // The speaker turns on the party as the dialogue closes.
    Resume,
}
#[derive(Debug)]
enum Continuation {
    Quote(Response),
    Options { words: Vec<String>, typed: bool },
    End,
}
fn plan_quote(world: &World, party: Entity, q: &Quote) -> Result<QuotePlan, (usize, String)> {
    let story = world.resource::<StoryFacts>();
    let catalog = fact_catalog(world);
    let mut writes = BTreeMap::new();
    let mut plan = QuotePlan { effects: Vec::new(), following: None };
    for (i, op) in q.operations.iter().enumerate() {
        match op {
            DialogueOp::Unsupported { opcode, .. } => {
                return Err((i, format!("unported mandatory opcode {opcode}")));
            }
            DialogueOp::SetFact(id, value) => {
                if !StoryFacts::can_set(*id, catalog) {
                    return Err((i, format!("unported fact assignment {id:?}")));
                }
                writes.insert(*id, *value);
                plan.effects.push(QuoteEffect::SetFact(*id, *value));
            }
            DialogueOp::GiveGold(amount) => {
                plan.effects.push(QuoteEffect::Gold { party, amount: *amount });
            }
            DialogueOp::GiveExperience(amount) => {
                // CanPartySlotParticipate includes the unconscious. Quest
                // XP is the authored amount for EACH eligible member.
                let members = world
                    .get::<PartyRoster>(party)
                    .unwrap()
                    .iter()
                    .filter(|&member| !world.get::<Vitals>(member).unwrap().is_dead())
                    .collect();
                plan.effects.push(QuoteEffect::Experience { members, amount: *amount });
            }
            DialogueOp::AdjustFaction { faction: Some(faction), delta } => {
                plan.effects.push(QuoteEffect::Faction { faction: *faction, delta: *delta });
            }
            // Retail ignores a faction name it does not know.
            DialogueOp::AdjustFaction { faction: None, .. } => {}
            DialogueOp::GiveItem(item) => {
                plan.effects.push(QuoteEffect::GiveItem { party, item: *item });
            }
            DialogueOp::TakeItem(item) => {
                plan.effects.push(QuoteEffect::TakeItem { party, item: *item });
            }
            DialogueOp::Conditional { facts, response } => {
                let mut matched = true;
                for (id, expected) in facts {
                    let actual = match writes.get(id) {
                        Some(value) => *value,
                        None => story.get(*id, catalog).map_err(|e| (i, e))?,
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
                plan.following = Some(Continuation::Options { words: words.clone(), typed: false });
                break;
            }
            DialogueOp::KeywordInput(words) => {
                plan.following = Some(Continuation::Options { words: words.clone(), typed: true });
                break;
            }
            // Retail closes without reading the quote's later entries.
            DialogueOp::CloseAndResume => {
                plan.effects.push(QuoteEffect::Resume);
                plan.following = Some(Continuation::End);
                break;
            }
            // Retail `DismissNpcFromParty` acts only on an NPC in the party,
            // and none can join it yet (#117). For any other NPC the action
            // only locks and releases the world cursor.
            DialogueOp::LeaveParty { .. } => {}
            // An NPC no one answers to is left alone, as in retail.
            DialogueOp::AdjustDisposition { npc, delta } => {
                if let Some(npc) = npc_named(world, npc) {
                    plan.effects.push(QuoteEffect::Disposition { npc, delta: *delta });
                }
            }
            DialogueOp::PartyRemark(event) => plan.effects.push(QuoteEffect::PartyRemark(*event)),
            // Retail `W8Monster::SetScriptLabel` moves the NPC's monster
            // script, whose interpreter is not ported.
            DialogueOp::MonsterScriptLabel(Some(label)) => {
                return Err((i, format!("unported monster script label {label}")));
            }
            DialogueOp::MonsterScriptLabel(None) => {}
            DialogueOp::Mention { category, text } => {
                plan.effects.push(QuoteEffect::Mention(*category, text.clone()))
            }
            DialogueOp::Close => plan.following = Some(Continuation::End),
            _ => {}
        }
    }
    Ok(plan)
}
/// Retail `AddItemToPartyOrDrop`: a gift the party's items have no room for
/// lands at the party's feet, unless the party may not part with it; that
/// one goes to the hand with retail's notice (`DropUnstoredCharacterItem`).
/// Retail then puts back whatever the hand held, so it keeps only the first
/// such gift.
fn drop_gift(
    world: &mut World,
    party: Entity,
    gift: crate::game::inventory::ItemStack,
    crucial: bool,
) {
    use crate::game::{party::body::PartyBody, world::items};
    let level = world.query_filtered::<Entity, With<Level>>().single(world).ok();
    let feet = PartyBody::feet_in(world);
    match level {
        Some(level) if !crucial => {
            items::lay(&mut world.commands(), level, feet, items::WorldItem::loose(gift, false));
            world.flush();
        }
        _ => {
            let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
            if inventory.hand.held.is_none() {
                inventory.hand.held = Some(gift);
            }
            world.write_message(crate::game::inventory::ItemNews::CannotDrop);
        }
    }
}
/// Commit a removal whose ownership and quantity were resolved in this operation.
fn consume_item(world: &mut World, party: Entity, source: ItemRef, units: u16) {
    match source.owner {
        ItemOwner::Pool => {
            world.get_mut::<PartyInventory>(party).unwrap().pool.consume_units(source.id, units)
        }
        ItemOwner::Member(member) => {
            world.get_mut::<Carried>(member).unwrap().consume_backpack_units(source.id, units)
        }
    }
}
fn commit_quote(world: &mut World, entity: Entity, quote: u16, plan: QuotePlan) {
    commit_effects(world, entity, plan.effects);
    if let Some(following) = plan.following {
        let next = match following {
            Continuation::Quote(Response::Speaker { npc, quote }) => {
                return hand_over(world, entity, &npc, quote);
            }
            Continuation::Quote(response) => Step::Quote {
                quote: select_response(&response, &mut world.resource_mut::<GameRng>())
                    .expect("a response within this script"),
                subquote: 0,
                paid: false,
                plan: None,
            },
            Continuation::Options { words, typed } => {
                Step::Wait(ConversationWait::Options { quote, words, typed })
            }
            Continuation::End => Step::End,
        };
        world.get_mut::<Conversation>(entity).unwrap().queue.push_front(next);
    }
}
fn commit_effects(world: &mut World, entity: Entity, effects: Vec<QuoteEffect>) {
    for effect in effects {
        match effect {
            QuoteEffect::SetFact(id, value) => {
                let level = world
                    .query::<&Level>()
                    .single(world)
                    .map_or_else(|_| String::new(), |level| level.path.clone());
                Catalogs::scope(world, |world, catalogs| {
                    world
                        .resource_mut::<StoryFacts>()
                        .set(id, value, &catalogs.facts, level)
                        .expect("planned fact assignment");
                });
            }
            QuoteEffect::Mention(category, text) => learn_topic(world, entity, category, &text),
            QuoteEffect::Gold { party, amount } => {
                let mut purse = world.get_mut::<PartyInventory>(party).unwrap();
                purse.gold = purse.gold.wrapping_add(amount);
                let actor = world.get::<Conversation>(entity).unwrap().actor;
                let giver = definition(world, actor).source_name.clone();
                world
                    .get_mut::<Conversation>(entity)
                    .unwrap()
                    .transcript
                    .push(TranscriptLine::Reward(QuestReward::Gold { giver, amount }));
            }
            QuoteEffect::Experience { members, amount } => {
                for member in members {
                    world.get_mut::<Character>(member).unwrap().award_experience(amount);
                }
                world
                    .get_mut::<Conversation>(entity)
                    .unwrap()
                    .transcript
                    .push(TranscriptLine::Reward(QuestReward::Experience { amount_each: amount }));
            }
            // `executable` checked the content just before.
            QuoteEffect::GiveItem { party, item } => {
                let (name, crucial, rest) = Catalogs::scope(world, |world, c| {
                    let catalog = &c.items;
                    let definition = &catalog[item];
                    let stack = world.resource_scope(|world, mut ids: Mut<ItemIds>| {
                        let mut rng = world.resource_mut::<GameRng>();
                        make_item(definition, &mut ids, &mut rng, true, true)
                    });
                    let rest = world
                        .get_mut::<PartyInventory>(party)
                        .unwrap()
                        .pool
                        .receive(stack, catalog);
                    (definition.name.clone(), definition.crucial, rest)
                });
                if let Some(rest) = rest {
                    drop_gift(world, party, rest, crucial);
                }
                world
                    .get_mut::<Conversation>(entity)
                    .unwrap()
                    .transcript
                    .push(TranscriptLine::Reward(QuestReward::Item { name }));
            }
            QuoteEffect::TakeItem { party, item } => {
                let source = world
                    .get::<PartyRoster>(party)
                    .unwrap()
                    .iter()
                    .find_map(|member| {
                        let pack = world.get::<Carried>(member).unwrap();
                        let stack = pack.backpack.iter().flatten().find(|s| s.item == item)?;
                        Some(ItemRef { owner: ItemOwner::Member(member), id: stack.id })
                    })
                    .or_else(|| {
                        world
                            .get::<PartyInventory>(party)
                            .unwrap()
                            .pool
                            .stacks
                            .iter()
                            .find(|s| s.item == item)
                            .map(|s| ItemRef { owner: ItemOwner::Pool, id: s.id })
                    });
                if let Some(source) = source {
                    consume_item(world, party, source, 1);
                    let member = match source.owner {
                        ItemOwner::Member(member) => {
                            Some(world.get::<Character>(member).unwrap().name.clone())
                        }
                        ItemOwner::Pool => None,
                    };
                    world
                        .get_mut::<Conversation>(entity)
                        .unwrap()
                        .transcript
                        .push(TranscriptLine::Reward(QuestReward::Taken { member }));
                }
            }
            QuoteEffect::Disposition { npc, delta } => {
                shift_disposition(&mut world.get_mut::<Npc>(npc).unwrap(), delta)
            }
            QuoteEffect::Resume => {
                let actor = world.get::<Conversation>(entity).unwrap().actor;
                resume(world, actor);
            }
            QuoteEffect::PartyRemark(event) => {
                world.write_message(PartyRemark { event });
            }
            QuoteEffect::Faction { faction, delta } => {
                // Retail announces it in the text box, not the dialogue.
                let now = world.resource::<GameClock>().ms;
                if let Some(news) = world.resource_mut::<Factions>().adjust(faction, delta, now) {
                    world.write_message(news);
                }
            }
        }
    }
}
fn run(world: &mut World, entity: Entity) {
    let c = world.get::<Conversation>(entity).unwrap();
    if c.wait != ConversationWait::Ready || c.queue.is_empty() {
        return;
    }
    let handle = super::script(world, c.actor);
    let party = the_party(world);
    world.resource_scope(|world, scripts: Mut<Assets<DialogueScript>>| {
        run_script(world, entity, scripts.get(handle).unwrap(), party);
    });
}
fn run_script(world: &mut World, entity: Entity, script: &DialogueScript, party: Entity) {
    let actor = world.get::<Conversation>(entity).unwrap().actor;
    // Bound immediate authored transitions, including malicious empty-quote loops.
    for _ in 0..256 {
        let c = world.get::<Conversation>(entity).unwrap();
        // A new speaker's script is fetched on the next run.
        if c.wait != ConversationWait::Ready || c.actor != actor {
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
            Step::Wait(wait) => {
                let mut c = world.get_mut::<Conversation>(entity).unwrap();
                c.wait = wait;
                return;
            }
            Step::Quote { quote, subquote, paid, plan } => {
                let q = &script.quotes[quote as usize];
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
                    let charm = world.get::<Conversation>(entity).unwrap().charm().max(0) as u32;
                    let gold = gold - gold.saturating_mul(charm) / 100;
                    let gold =
                        (gold - gold.saturating_mul(party_communication(world)) / 500).max(1);
                    let mut c = world.get_mut::<Conversation>(entity).unwrap();
                    c.queue.push_front(Step::Wait(ConversationWait::Price { quote, gold, always }));
                    c.queue.push_front(Step::said(PRICE_QUOTE));
                    continue;
                }
                let plan = match plan.map_or_else(|| plan_quote(world, party, q), Ok) {
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
                    if first {
                        let name = definition(world, actor).source_name.clone();
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
                let def = definition(world, actor);
                // Retail `SpeakNpcSubquote`: an alert line is shown with the
                // alert chime and no recording; otherwise a few words stand
                // for something other than a line.
                let is = |word: &str| text.eq_ignore_ascii_case(word);
                let voice = if q.alert {
                    None
                } else if text.is_empty() {
                    continue;
                } else if is("EMPTY") || is("BLANK") {
                    // The answer to a question it has nothing to say to.
                    if (NO_ANSWER_QUOTES).contains(&quote) {
                        let npc = def.source_name.clone();
                        let mut turn = world.get_resource_or_init::<NoAnswerTurn>();
                        let second = turn.0;
                        turn.0 = !second;
                        let mut c = world.get_mut::<Conversation>(entity).unwrap();
                        c.transcript.push(TranscriptLine::NoAnswer { npc, second });
                    }
                    continue;
                } else if is("UNKNOWN") || is("CLASSIFIED") {
                    let next = if is("CLASSIFIED") {
                        CLASSIFIED_QUOTE
                    } else if world.resource_mut::<GameRng>().below(100) < 50 {
                        UNKNOWN_QUOTES[0]
                    } else {
                        UNKNOWN_QUOTES[1]
                    };
                    world
                        .get_mut::<Conversation>(entity)
                        .unwrap()
                        .queue
                        .push_back(Step::quote(next));
                    continue;
                } else {
                    let suffix = if subquote == 0 {
                        String::new()
                    } else {
                        char::from(b'a' + (subquote - 1) as u8).to_string()
                    };
                    Some(format!("{}_{quote:03}{suffix}", def.voice_stem()))
                };
                // "SOUND" plays its recording without showing a line.
                let text = if is("SOUND") && !q.alert { String::new() } else { text };
                let speaker = def.source_name.clone();
                let speech = world
                    .spawn((
                        ConversationSpeech { voice, text: text.clone(), alert: q.alert },
                        ChildOf(entity),
                    ))
                    .id();
                if world.get::<Closing>(entity).is_some() && !text.is_empty() {
                    world.write_message(NpcSpeech { speaker, text: text.clone() });
                }
                // The line being spoken is always the transcript's last; one
                // with no text is not shown.
                let mut c = world.get_mut::<Conversation>(entity).unwrap();
                c.transcript.push(TranscriptLine::Npc(text));
                c.wait = ConversationWait::Line { speech };
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
    c.queue.push_back(Step::quote(quote));
    if answer.is_ok() {
        c.queue.extend([Step::Join, Step::End]);
    }
}
fn topic(world: &mut World, entity: Entity, text: &str) {
    let c = world.get::<Conversation>(entity).unwrap();
    if !c.ready() || c.menu {
        return;
    }
    let script = script(world, c);
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
        c.queue.push_back(Step::quote(quote));
    } else {
        world.get_mut::<Conversation>(entity).unwrap().report(Outcome::NoTopic);
    }
}
/// The party charms the NPC or makes small talk with it from the topic menu
/// (retail `SelectNpcDialogueService`, `SelectNpcDialogueTalk` and
/// `ApplyNpcInteraction` 1 and 0). Its disposition moves by its record's
/// scale, more with the party's best Communication, which practises; a
/// charm that warms it moves it further the higher the party's level is
/// over its own. The band it ends in answers: won over, it turns to its
/// dialogue; turned hostile, it sends the party away.
fn approach_npc(world: &mut World, entity: Entity, approach: Approach) {
    let c = world.get::<Conversation>(entity).unwrap();
    let actor = c.actor;
    let now = world.resource::<GameClock>().ms;
    let npc = world.get::<Npc>(actor).unwrap();
    let at = match approach {
        Approach::Charm => npc.charmed_at,
        Approach::SmallTalk => npc.small_talk_at,
    };
    let def = definition(world, actor);
    let scale = i32::from(match approach {
        Approach::Charm => def.charm_scale,
        Approach::SmallTalk => def.talk_scale,
    });
    if !c.ready() || !c.menu || !approach_ready(at, now) {
        return;
    }
    let Some((speaker, level)) = best_communicator(world) else { return };
    let mut delta = scale + level as i32 * (scale.abs() / 5) / 100;
    if approach == Approach::Charm && delta > 0 {
        let average = crate::game::party::average_level(
            crate::game::party::Lineup::in_world(world).able().map(|(_, c)| c.level()),
        );
        let own = active_body(world, actor)
            .and_then(|body| world.get::<Monster>(body))
            .map(|m| &world.resource::<Catalogs>().species[m.species])
            .map_or(1, |kind| u32::from(kind.level).max(1));
        delta = (average * delta as u32 / own) as i32;
    }
    let mut npc = world.get_mut::<Npc>(actor).unwrap();
    // Retail narrows the change to a signed byte.
    shift_disposition(&mut npc, delta as i8);
    let usage = match approach {
        Approach::Charm => {
            npc.charmed_at = Some(now);
            5
        }
        Approach::SmallTalk => {
            npc.small_talk_at = Some(now);
            8
        }
    };
    practise_communication(world, &[speaker], usage);
    let answer = match (band(world, actor), approach) {
        (Band::Friendly, Approach::Charm) => Some(CHARMED_QUOTE),
        (Band::Friendly, Approach::SmallTalk) => Some(greet(world, actor)),
        (Band::Neutral, Approach::Charm) => Some(UNCHARMED_QUOTE),
        (Band::Neutral, Approach::SmallTalk) => Some(SMALL_TALK_QUOTE),
        (Band::Hostile, _) => {
            turn_hostile(world, actor);
            Some(HOSTILE_QUOTE)
        }
    };
    let friendly = band(world, actor) == Band::Friendly;
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    if let Some(quote) = answer {
        c.queue.push_back(Step::quote(quote));
    }
    if !friendly && answer == Some(HOSTILE_QUOTE) {
        c.queue.push_back(Step::End);
    }
    c.menu = !friendly;
}
/// Retail `GetItemStackValue`: ammunition by the bundle of 25, a stack of
/// things by each, anything else as one.
fn stack_value(item: &crate::content::items::ItemDefinition, count: u16) -> u32 {
    let count = u32::from(count);
    if item.class == crate::content::items::EquipClass::Ammunition {
        (item.value * count).div_ceil(crate::content::items::AMMUNITION_BUNDLE)
    } else if item.quantity == crate::content::items::QuantityKind::Stack && count > 1 {
        item.value * count
    } else {
        item.value
    }
}
/// Retail `WillNpcTradeForItem`: one kind of NPC takes no bribes in kind;
/// the rest take one item always, and others worth 300 gold.
const NO_TRADE_KIND: i32 = 20;
const ALWAYS_TAKEN: ItemId = ItemId(0x29f);
const LEAST_BRIBE: u32 = 300;
/// Retail `DebitNpcTradePool`: a bribe, Communication-weighted, draws on
/// what the NPC will take. Once that runs out an NPC not yet friendly warms
/// to the party, and the pool fills again.
fn debit_trade_pool(world: &mut World, actor: Entity, price: u32) {
    let (pool, disposition) = debited(world, actor, price);
    let mut npc = world.get_mut::<Npc>(actor).unwrap();
    (npc.trade_pool, npc.disposition) = (pool, disposition);
}
/// The NPC's trade pool and own disposition once a bribe of `price` is
/// debited, without debiting it.
fn debited(world: &mut World, actor: Entity, price: u32) -> (u16, u8) {
    let level = party_communication(world);
    let adjusted = price.wrapping_add(level * ((price & 0xffff) / 5) / 100);
    let refill = definition(world, actor).trade_pool;
    let cold = band(world, actor) != Band::Friendly;
    let npc = world.get::<Npc>(actor).unwrap();
    match npc.trade_pool.saturating_sub(adjusted as u16) {
        0 if cold => (refill, 0x4b),
        0 => (refill, npc.disposition),
        pool => (pool, npc.disposition),
    }
}
/// A bribe that wins the NPC over from the topic menu turns it to its
/// dialogue; it greets the party unless it took all the party's gold
/// (retail `HandleNpcDialogueItemChoice`, `OnNpcTradeDialogClosed`).
pub(crate) fn won_over(world: &mut World, entity: Entity, greeting: bool) {
    let actor = world.get::<Conversation>(entity).unwrap().actor;
    if !world.get::<Conversation>(entity).unwrap().menu || band(world, actor) != Band::Friendly {
        return;
    }
    let greeting = if greeting { Some(greet(world, actor)) } else { None };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.menu = false;
    if let Some(quote) = greeting {
        c.queue.push_back(Step::quote(quote));
    }
}
/// What a friendly NPC says to an item it has no quote for: it takes it
/// gladly, or turns down one the party may not give away.
const ACCEPTED_GIFT_QUOTE: u16 = 0x10;
const REFUSED_GIFT_QUOTE: u16 = 0x11;
/// The party offers the NPC an item from a member's pack or the party's items
/// (retail `HandleNpcDialogueItem`). The quote answering it runs; the NPC keeps what it asked for, and any other item it
/// is given unless the party may not part with it. A voice-scripted NPC
/// ignores an item it has nothing to say about. The item is taken only when
/// its answer's effects are valid; a content gap keeps it.
fn give(world: &mut World, entity: Entity, item: ItemRef) -> Result<(), TradeError> {
    let party = the_party(world);
    let c = world.get::<Conversation>(entity).unwrap();
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    let actor = c.actor;
    let stack = sale_item(world, item, party)?;
    let (offered, count) = (stack.item, stack.count());
    let definition_of = &world.resource::<Catalogs>().items[offered];
    let (crucial, value) = (definition_of.crucial, stack_value(definition_of, count));
    let def = definition(world, actor);
    let (voiced, kind) = (def.voice_script, def.kind);
    let script = script(world, c);
    let answer = script.item_quote(offered);
    // Retail `HandleNpcDialogueItem`: an NPC not yet friendly takes an item
    // as a bribe, valued by the whole stack offered, and keeps it whatever
    // it says.
    let bribe = !voiced && band(world, actor) != Band::Friendly;
    // A bribe is debited only with the item it buys.
    let mut debit = None;
    let (quote, keeps) = match answer {
        _ if bribe && crucial => (REFUSED_GIFT_QUOTE, false),
        _ if bribe
            && (kind == NO_TRADE_KIND || (offered != ALWAYS_TAKEN && value < LEAST_BRIBE)) =>
        {
            (UNBRIBED_QUOTE, false)
        }
        Some((quote, _)) if bribe => (quote, true),
        None if bribe => {
            let after = debited(world, actor, value);
            debit = Some(after);
            let warmed = band_of(standing(world, actor, after.1)) == Band::Friendly;
            (if warmed { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE }, true)
        }
        Some(answer) => answer,
        None if voiced => return Ok(()),
        None if crucial => (REFUSED_GIFT_QUOTE, false),
        None => (ACCEPTED_GIFT_QUOTE, true),
    };
    let script = self::script(world, world.get::<Conversation>(entity).unwrap());
    let plan =
        script.quotes.get(usize::from(quote)).and_then(|line| plan_quote(world, party, line).ok());
    // Retail takes the item at once; the answer's effects follow at its last
    // line, which plays out even if the party leaves the dialogue. A content
    // gap keeps the item and queues the answer to show it.
    if keeps && plan.is_some() {
        consume_item(world, party, item, 1);
        if let Some((pool, disposition)) = debit {
            let mut npc = world.get_mut::<Npc>(actor).unwrap();
            (npc.trade_pool, npc.disposition) = (pool, disposition);
        }
    }
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan });
    won_over(world, entity, true);
    Ok(())
}
/// The party gives the NPC gold (retail `HandleNpcDialogueItem` with trade
/// gold): all of it only once the party confirms, as retail asks.
fn give_gold(world: &mut World, entity: Entity, amount: u32) -> Result<(), TradeError> {
    let party = the_party(world);
    if !world.get::<Conversation>(entity).unwrap().ready() {
        return Err(TradeError::Busy);
    }
    let gold = world.get::<PartyInventory>(party).unwrap().gold;
    if amount == 0 {
        return Err(TradeError::InvalidQuantity);
    }
    if amount > gold {
        return Err(TradeError::Funds);
    }
    if amount == gold {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.wait = ConversationWait::GiveAll { gold };
        return Ok(());
    }
    spend_gift(world, entity, party, amount, false)
}
/// The NPC takes the gold (retail `ConfirmNpcTradePurchase`): a friendly
/// one thanks the party for it; one not yet friendly takes it as a bribe,
/// unless it takes no bribes, and says whether that won it over.
fn spend_gift(
    world: &mut World,
    entity: Entity,
    party: Entity,
    amount: u32,
    all: bool,
) -> Result<(), TradeError> {
    let actor = world.get::<Conversation>(entity).unwrap().actor;
    let bribe = band(world, actor) != Band::Friendly;
    if bribe && definition(world, actor).trade_pool == 0 {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.queue.push_back(Step::said(UNBRIBED_QUOTE));
        return Ok(());
    }
    let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
    inventory.gold = inventory.gold.checked_sub(amount).ok_or(TradeError::Funds)?;
    if bribe {
        debit_trade_pool(world, actor, amount);
    }
    let quote =
        if band(world, actor) == Band::Friendly { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.queue.push_back(Step::said(quote));
    won_over(world, entity, !all);
    Ok(())
}
fn reply(world: &mut World, entity: Entity, text: &str) {
    let c = world.get::<Conversation>(entity).unwrap();
    let script = script(world, c);
    if let ConversationWait::Options { quote, words, .. } = &c.wait {
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
            c.queue.push_back(Step::quote(quote));
        }
    }
}
fn answer_price(world: &mut World, entity: Entity, accept: bool) {
    let party = the_party(world);
    let c = world.get::<Conversation>(entity).unwrap();
    if let ConversationWait::GiveAll { gold } = c.wait {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.wait = ConversationWait::Ready;
        if accept && let Err(refused) = spend_gift(world, entity, party, gold, true) {
            world.get_mut::<Conversation>(entity).unwrap().report(Outcome::GiftRefused(refused));
        }
        return;
    }
    let ConversationWait::Price { quote, gold, always } = c.wait else { return };
    let actor = c.actor;
    let handle = super::script(world, actor);
    world.resource_scope(|world, scripts: Mut<Assets<DialogueScript>>| {
        let Some(line) = scripts.get(handle).unwrap().quotes.get(usize::from(quote)) else {
            return;
        };
        if accept {
            let (next, plan) = if world.get::<PartyInventory>(party).unwrap().gold < gold {
                (TOO_POOR_QUOTE, None)
            } else {
                let plan = match plan_quote(world, party, line) {
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
                    c.queue.push_back(Step::quote(*quote));
                }
            }
        }
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.wait = ConversationWait::Ready;
    });
}

pub fn quote_trade(
    world: &mut World,
    conversation: Entity,
    source: TradeSource,
    quantity: u32,
) -> Result<TradeQuote, TradeError> {
    if crate::game::world::party_occupied(world) {
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
    let party = the_party(world);
    let communication = party_communication(world);
    let def = definition(world, actor);
    let catalog = &world.resource::<Catalogs>().items;
    let trade = world.get::<Npc>(actor).unwrap().stock.trades;
    let gold = match source {
        TradeSource::Stock(lot) => {
            let stock = &world.get::<Npc>(actor).unwrap().stock;
            trade::purchase_price(
                stock.lots.iter().find(|l| l.id == lot).ok_or(TradeError::StaleSelection)?,
                def,
                catalog,
                quantity,
                communication,
            )
        }
        TradeSource::Party(source) => {
            let item = sale_item(world, source, party)?;
            trade::validate_sale(def, item, quantity, catalog, communication)
        }
    }?;
    Ok(TradeQuote { conversation, trade, source, quantity, gold })
}
fn sale_item(
    world: &World,
    source: ItemRef,
    party: Entity,
) -> Result<&crate::game::inventory::ItemStack, TradeError> {
    match source.owner {
        ItemOwner::Pool => world
            .get::<PartyInventory>(party)
            .unwrap()
            .pool
            .stacks
            .iter()
            .find(|i| i.id == source.id),
        ItemOwner::Member(member) => {
            world.get::<Carried>(member).unwrap().backpack_stack(source.id)
        }
    }
    .ok_or(TradeError::StaleSelection)
}
fn execute_trade(world: &mut World, quote: TradeQuote) -> Result<(), TradeError> {
    let occupied = crate::game::world::party_occupied(world);
    let c = world.get::<Conversation>(quote.conversation).ok_or(TradeError::StaleSelection)?;
    if !c.ready() {
        return Err(TradeError::StaleSelection);
    }
    if !available(world, c.actor) || !trade_allowed(world, c.actor) || occupied {
        return Err(TradeError::Unavailable);
    }
    let actor = c.actor;
    if world.get::<Npc>(actor).unwrap().stock.trades != quote.trade {
        return Err(TradeError::StaleSelection);
    }
    let party = the_party(world);
    let communication = party_communication(world);
    with_definition(world, actor, |world, def, catalog| match quote.source {
        TradeSource::Stock(lot) => world.resource_scope(|world, mut ids: Mut<ItemIds>| {
            let [mut party_entity, mut actor] = world.get_entity_mut([party, actor]).unwrap();
            let mut inventory = party_entity.get_mut::<PartyInventory>().unwrap();
            let mut npc = actor.get_mut::<Npc>().unwrap();
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
            let item = sale_item(world, source, party)?;
            let units = u16::try_from(quote.quantity).map_err(|_| TradeError::InvalidQuantity)?;
            let gold = world.get::<PartyInventory>(party).unwrap().gold + quote.gold;
            if trade::validate_sale(def, item, quote.quantity, catalog, communication)?
                != quote.gold
            {
                return Err(TradeError::PriceChanged);
            }
            let item = item.item;
            world.get_mut::<Npc>(actor).unwrap().stock.add(item, quote.quantity, catalog, false);
            consume_item(world, party, source, units);
            world.get_mut::<PartyInventory>(party).unwrap().gold = gold;
            Ok(())
        }
    })?;
    world.get_mut::<Npc>(actor).unwrap().stock.trades += 1;
    Ok(())
}
pub fn update_conversation(world: &mut World) {
    if crate::game::world::level::entering(world) {
        // An accepted entry has ended the dialogue (`end_conversations`);
        // no queued payment, trade or authored effect may alter the
        // departing campaign.
        world.resource_mut::<Messages<ConversationRequest>>().clear();
        return;
    }
    let begins: Vec<_> = world.resource_mut::<Messages<BeginConversation>>().drain().collect();
    for BeginConversation { actor } in begins {
        if let Err(e) = begin(world, actor) {
            warn!("conversation refused: {e:?}");
        }
    }
    let requests: Vec<_> = world.resource_mut::<Messages<ConversationRequest>>().drain().collect();
    for ConversationRequest { conversation: entity, talk } in requests {
        // Nothing but its lines' playback reaches a dialogue the party left.
        if world.get::<Closing>(entity).is_some() && !matches!(talk, Talk::Advance { .. }) {
            continue;
        }
        // Leaving needs nothing of the NPC; anything else ends the dialogue
        // when the NPC, a fight or a camp no longer allows it.
        if !matches!(talk, Talk::Close) {
            let in_battle = crate::game::combat::in_battle(world);
            let Some(c) = world.get::<Conversation>(entity) else { continue };
            if !available(world, c.actor)
                || in_battle
                || world.contains_resource::<crate::game::camp::Camp>()
            {
                close_one(world, entity);
                continue;
            }
            world.get_mut::<Conversation>(entity).unwrap().result = None;
        }
        let outcome = match talk {
            Talk::Close => {
                leave(world, entity);
                None
            }
            Talk::EditNotebook { edit, text } => {
                let mut facts = world.resource_mut::<StoryFacts>();
                match edit {
                    NotebookEdit::Remember => facts.remember_topic(TopicCategory::Misc, &text),
                    NotebookEdit::Forget => facts.forget_topic(&text),
                };
                None
            }
            Talk::Topic { text } => {
                topic(world, entity, &text);
                None
            }
            Talk::Reply { text } => {
                reply(world, entity, &text);
                None
            }
            Talk::PriceAnswer { accept } => {
                answer_price(world, entity, accept);
                None
            }
            Talk::Recruit => {
                ask_to_join(world, entity);
                None
            }
            Talk::Advance { speech } => {
                if matches!(world.get::<Conversation>(entity).unwrap().wait, ConversationWait::Line { speech: s, .. } if s == speech)
                {
                    world.despawn(speech);
                    world.get_mut::<Conversation>(entity).unwrap().wait = ConversationWait::Ready;
                }
                None
            }
            Talk::Give { item } => give(world, entity, item).err().map(Outcome::GiftRefused),
            Talk::GiveGold { amount } => {
                give_gold(world, entity, amount).err().map(Outcome::GiftRefused)
            }
            Talk::Approach { approach } => {
                approach_npc(world, entity, approach);
                None
            }
            Talk::Trade(q) => Some(match execute_trade(world, q) {
                Ok(()) => Outcome::Traded,
                Err(e) => Outcome::TradeRefused(e),
            }),
        };
        if let Some(outcome) = outcome {
            world.get_mut::<Conversation>(entity).unwrap().report(outcome);
        }
    }
    let sessions: Vec<_> =
        world.query_filtered::<Entity, With<Conversation>>().iter(world).collect();
    for entity in sessions {
        let in_battle = crate::game::combat::in_battle(world);
        let Some(c) = world.get::<Conversation>(entity) else { continue };
        if !available(world, c.actor) || in_battle {
            close_one(world, entity);
            continue;
        }
        run(world, entity);
        let c = world.get::<Conversation>(entity).unwrap();
        let left = world.get::<Closing>(entity).is_some()
            && match c.wait {
                ConversationWait::Ready => c.queue.is_empty(),
                ConversationWait::Line { .. } => false,
                _ => true,
            };
        if left || (matches!(c.queue.front(), Some(Step::End)) && c.wait == ConversationWait::Ready)
        {
            close_one(world, entity);
        }
    }
}
