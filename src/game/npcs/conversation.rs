//! Authored continuation and waits belong to gameplay, not the dialogue view.
use super::{
    Npc, NpcAssets,
    trade::{self, TradeError, TradeQuote, TradeSource},
};
use crate::content::factions::Faction;
use crate::content::{
    items::{ItemCatalog, ItemId},
    npcs::*,
};
use crate::game::factions::{Band, Factions};
use crate::game::{
    characters::{Character, Condition, Vitals},
    data::GameData,
    inventory::{Carried, ItemIds, ItemOwner, ItemPool, ItemRef, PartyInventory, make_item},
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

    /// Forget a notebook entry without changing discovered facts or journal history.
    pub fn forget_topic(&mut self, text: &str) -> bool {
        let text = text.trim();
        let before = self.topics.len();
        self.topics.retain(|(_, known)| !known.eq_ignore_ascii_case(text));
        self.topics.len() != before
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
    Options { quote: u16, words: Vec<String>, typed: bool },
    Price { quote: u16, gold: u32, always: bool },
    Join,
    End,
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
    pub script: Handle<DialogueScript>,
    pub revision: u64,
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
        self.revision += 1;
        self.result = None;
        match reply {
            MagicalReply::Unnoticed => {}
            MagicalReply::Charmed(strength) if strength > self.charm() => {
                self.charms.insert(self.actor, strength);
                self.transcript.push(TranscriptLine::Npc("Charmed.".into()));
            }
            MagicalReply::Charmed(_) => {}
            MagicalReply::Quote(quote) => {
                self.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None })
            }
        }
    }

    /// What the last request came to, until the conversation moves on.
    pub fn outcome(&self) -> Option<Outcome> {
        self.result.filter(|&(revision, _)| revision == self.revision).map(|(_, o)| o)
    }

    fn report(&mut self, outcome: Outcome) {
        self.result = Some((self.revision, outcome));
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

#[derive(Message, Clone, Debug)]
pub enum ConversationRequest {
    Begin {
        actor: Entity,
    },
    Topic {
        conversation: Entity,
        revision: u64,
        text: String,
    },
    Reply {
        conversation: Entity,
        revision: u64,
        text: String,
    },
    PriceAnswer {
        conversation: Entity,
        revision: u64,
        accept: bool,
    },
    Advance {
        conversation: Entity,
        speech: Entity,
    },
    Close {
        conversation: Entity,
    },
    Recruit {
        conversation: Entity,
        revision: u64,
    },
    EditNotebook {
        conversation: Entity,
        revision: u64,
        edit: NotebookEdit,
        text: String,
    },
    Trade(TradeQuote),
    /// Hand the NPC an item from a member's pack or the party's items.
    Give {
        conversation: Entity,
        revision: u64,
        item: ItemRef,
    },
    /// Hand the NPC some of the party's gold.
    GiveGold {
        conversation: Entity,
        revision: u64,
        amount: u32,
    },
    /// Try to win over an NPC that is not yet friendly.
    Approach {
        conversation: Entity,
        revision: u64,
        approach: Approach,
    },
}
/// How the party tries to win an NPC over from retail's topic menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    Charm,
    SmallTalk,
}
fn definition(world: &World, actor: Entity) -> Result<&NpcDefinition, TradeError> {
    world.get::<super::ActorId>(actor).ok_or(TradeError::Unavailable)?;
    super::definition(world, actor).ok_or(TradeError::MissingContent)
}
fn with_definition<T>(
    world: &mut World,
    actor: Entity,
    f: impl FnOnce(&mut World, &NpcDefinition) -> Result<T, TradeError>,
) -> Result<T, TradeError> {
    let id = world.get::<super::ActorId>(actor).ok_or(TradeError::Unavailable)?.npc();
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
    GameData::with_items(world, f).ok_or(TradeError::MissingContent)?
}
pub fn party_communication(world: &mut World) -> u32 {
    best_communicator(world).map_or(0, |(_, level)| level)
}
fn average_level(world: &mut World) -> u32 {
    let Some(party) = the_party(world) else { return 1 };
    let Some(roster) = world.get::<PartyRoster>(party) else { return 1 };
    crate::game::party::average_level(
        roster.iter().filter_map(|member| world.get::<Character>(member)).map(|c| c.level()),
    )
}
pub(crate) fn active_body(world: &World, actor: Entity) -> Option<Entity> {
    world.get::<super::NpcBodies>(actor)?.iter().find(|body| {
        world
            .get::<crate::game::world::level::LevelOwned>(*body)
            .is_some_and(|parent| world.get::<Level>(parent.0).is_some())
            && world.get::<Monster>(*body).is_some_and(|m| !m.is_dead())
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
    let Some(npc) = world.get::<Npc>(actor) else { return 0 };
    standing(world, actor, npc.disposition)
}
/// [`disposition`] were the NPC's own `disposition` this.
fn standing(world: &World, actor: Entity, disposition: u8) -> i32 {
    let turned = active_body(world, actor)
        .and_then(|body| world.get::<Monster>(body))
        .is_some_and(|m| m.has(Condition::Turncoat));
    let charm = world
        .try_query::<&Conversation>()
        .and_then(|mut q| q.iter(world).find_map(|c| c.charms.get(&actor).copied()))
        .unwrap_or(0);
    let own = if turned { 100 } else { i32::from(disposition.wrapping_add(charm as u8)) };
    let (Ok(def), Some(factions)) = (definition(world, actor), world.get_resource::<Factions>())
    else {
        return own;
    };
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
fn best_communicator(world: &mut World) -> Option<(Entity, u32)> {
    let party = the_party(world)?;
    let roster = world.get::<PartyRoster>(party)?;
    crate::game::party::best_skill(
        roster.iter().filter_map(|member| {
            Some((member, world.get::<Character>(member)?, world.get::<Vitals>(member)?))
        }),
        crate::content::characters::Skill::Communication,
    )
}
/// Practise `members`' Communication `usage` times.
fn practise_communication(world: &mut World, members: &[Entity], usage: u32) {
    world.resource_scope(|world, mut rng: Mut<GameRng>| {
        for &member in members {
            if let Some(mut c) = world.get_mut::<Character>(member) {
                c.practise(crate::content::characters::Skill::Communication, usage, &mut rng);
            }
        }
    });
}
/// The members awake and well enough to fight (retail `hp_current != 0 &&
/// highest_condition < ASLEEP`).
fn able_members(world: &mut World) -> Vec<Entity> {
    let Some(party) = the_party(world) else { return vec![] };
    let Some(roster) = world.get::<PartyRoster>(party) else { return vec![] };
    roster.iter().filter(|&m| world.get::<Vitals>(m).is_some_and(|v| v.can_fight())).collect()
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
    definition(world, actor).is_ok_and(|def| def.owns_stock)
}
fn entry_quote(def: &NpcDefinition, npc: &Npc) -> u16 {
    if def.merchant || npc.greeting_pending { 0 } else { 1 }
}
/// What the NPC says to open its dialogue; meeting it for the first time
/// teaches the party's members Communication (retail
/// `HandleNpcDialogueDeparture`).
fn greet(world: &mut World, actor: Entity) -> Result<u16, TradeError> {
    let def = definition(world, actor)?;
    let greeting = entry_quote(def, world.get::<Npc>(actor).unwrap());
    if greeting == 0 && !def.monster_bound && !def.voice_script && !def.merchant {
        let members = able_members(world);
        practise_communication(world, &members, 0xf);
    }
    Ok(greeting)
}
pub fn begin(world: &mut World, actor: Entity) -> Result<Entity, TradeError> {
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    if !world.run_system_once(crate::game::freedom::party_free).map_err(|_| TradeError::Busy)?
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
    let eye = crate::game::party::body::PartyBody::eye_in(world)
        .filter(|eye| eye.distance(at) <= crate::game::world::items::REACH)
        .ok_or(TradeError::Unavailable)?;
    if !available(world, actor) {
        return Err(TradeError::Unavailable);
    }
    let id = world.get::<super::ActorId>(actor).ok_or(TradeError::Unavailable)?.npc();
    let script = world
        .resource::<NpcAssets>()
        .scripts
        .get(&id)
        .cloned()
        .ok_or(TradeError::MissingContent)?;
    if !world.resource::<Assets<DialogueScript>>().contains(&script) {
        return Err(TradeError::MissingContent);
    }
    let def = definition(world, actor)?;
    // Retail `BeginNpcDialogueInternal` and `OpenNpcDialoguePanel`: an NPC
    // that speaks through its own script greets as it is; otherwise one
    // hostile to the party only turns it away, a neutral one offers the
    // topic menu, and a merchant always does business.
    let scripted = def.monster_bound || def.voice_script;
    let band = band(world, actor);
    let quote =
        |quote| VecDeque::from([Step::Quote { quote, subquote: 0, paid: false, plan: None }]);
    let conversation = |queue| Conversation {
        actor,
        script: script.clone(),
        revision: 0,
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
    let greeting = if menu { NEUTRAL_GREETING } else { greet(world, actor)? };
    let now = world.resource::<crate::game::time::GameClock>().ms;
    let average = average_level(world);
    let def = definition(world, actor)?;
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
    crate::game::world::face(world, body, eye);
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
    let Some(c) = world.get::<Conversation>(entity) else { return };
    // Closing removes the view, not ownership of an exchange already paid for.
    // Retain every accepted plan until the whole settlement can run. The
    // unavailable-speaker/battle path retries this operation on later updates.
    let accepted = c.queue.iter().any(|step| matches!(step, Step::Quote { plan: Some(_), .. }));
    if accepted && definition(world, c.actor).is_err()
        || c.queue.iter().any(|step| match step {
            Step::Quote { plan: Some(plan), .. } => !executable(world, plan),
            _ => false,
        })
    {
        if !c.charms.is_empty() {
            world.get_mut::<Conversation>(entity).unwrap().charms.clear();
        }
        if world.get::<Closing>(entity).is_none() {
            world.entity_mut(entity).insert(Closing);
        }
        return;
    }
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
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
    let catalog = super::catalog(world)?;
    let fixed = fact_catalog(world)
        .and_then(|facts| facts.0.iter().find(|f| f.symbol == "FACT_RFS81_HAS_BEEN_FIXED"))
        .is_some_and(|f| fact(world, f.id).is_ok_and(|value| value != 0));
    world.iter_entities().find_map(|e| {
        let id = e.get::<super::ActorId>()?.npc();
        let shown = if id == RFS81 && fixed {
            RFS81_FIXED_NAME
        } else {
            catalog.get(id)?.display_name.as_str()
        };
        shown.eq_ignore_ascii_case(name).then(|| e.id())
    })
}
/// Another NPC takes over the conversation and says `quote` of its own
/// script (retail `ProcessNpcQuoteEntry` for a response naming a speaker).
/// Retail finds the NPC by display name among those it knows; when none
/// answers to it, the conversation simply waits for the party.
fn hand_over(world: &mut World, entity: Entity, name: &str, quote: u16) {
    let Some(speaker) = npc_named(world, name) else { return };
    let id = world.get::<super::ActorId>(speaker).unwrap().npc();
    let Some(script) = world.resource::<NpcAssets>().scripts.get(&id).cloned() else {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        block(&mut c, quote, None, format!("{name}'s script unavailable"));
        return;
    };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.actor = speaker;
    c.script = script;
    c.queue.push_front(Step::Quote { quote, subquote: 0, paid: false, plan: None });
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
fn plan_quote(
    world: &World,
    party: Option<Entity>,
    q: &Quote,
) -> Result<QuotePlan, (usize, String)> {
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
                if !catalog.is_some_and(|catalog| StoryFacts::can_set(*id, catalog)) {
                    return Err((i, format!("unported fact assignment {id:?}")));
                }
                writes.insert(*id, *value);
                plan.effects.push(QuoteEffect::SetFact(*id, *value));
            }
            DialogueOp::GiveGold(amount) => {
                let party = party
                    .filter(|&p| world.get::<PartyInventory>(p).is_some())
                    .ok_or_else(|| (i, "party purse unavailable".into()))?;
                plan.effects.push(QuoteEffect::Gold { party, amount: *amount });
            }
            DialogueOp::GiveExperience(amount) => {
                let roster = party
                    .and_then(|p| world.get::<PartyRoster>(p))
                    .ok_or_else(|| (i, "party roster unavailable".into()))?;
                let mut members = Vec::new();
                for member in roster.iter() {
                    let vitals = world
                        .get::<Vitals>(member)
                        .ok_or_else(|| (i, "party member vitals unavailable".into()))?;
                    if world.get::<Character>(member).is_none() {
                        return Err((i, "party member character unavailable".into()));
                    }
                    // CanPartySlotParticipate includes the unconscious. Quest
                    // XP is the authored amount for EACH eligible member.
                    if !vitals.is_dead() {
                        members.push(member);
                    }
                }
                plan.effects.push(QuoteEffect::Experience { members, amount: *amount });
            }
            DialogueOp::AdjustFaction { faction: Some(faction), delta } => {
                if !world.contains_resource::<Factions>() {
                    return Err((i, "faction state unavailable".into()));
                }
                plan.effects.push(QuoteEffect::Faction { faction: *faction, delta: *delta });
            }
            // Retail ignores a faction name it does not know.
            DialogueOp::AdjustFaction { faction: None, .. } => {}
            DialogueOp::GiveItem(item) => {
                if GameData::items(world).and_then(|c| c.get(*item)).is_none() {
                    return Err((i, format!("unknown item {}", item.0)));
                }
                let party = party
                    .filter(|&p| world.get::<PartyInventory>(p).is_some())
                    .ok_or_else(|| (i, "party items unavailable".into()))?;
                plan.effects.push(QuoteEffect::GiveItem { party, item: *item });
            }
            DialogueOp::TakeItem(item) => {
                let party = party
                    .filter(|&p| world.get::<PartyInventory>(p).is_some())
                    .ok_or_else(|| (i, "party items unavailable".into()))?;
                plan.effects.push(QuoteEffect::TakeItem { party, item: *item });
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
/// Whether the plan can commit now: its recipients and content exist.
fn executable(world: &World, plan: &QuotePlan) -> bool {
    plan.effects.iter().all(|effect| match effect {
        QuoteEffect::Gold { party, .. } => world.get::<PartyInventory>(*party).is_some(),
        QuoteEffect::Experience { members, .. } => {
            members.iter().all(|&member| world.get::<Character>(member).is_some())
        }
        QuoteEffect::Faction { .. } => world.contains_resource::<Factions>(),
        QuoteEffect::SetFact(id, _) => {
            fact_catalog(world).is_some_and(|catalog| StoryFacts::can_set(*id, catalog))
        }
        QuoteEffect::Mention(..) => true,
        QuoteEffect::GiveItem { party, item } => {
            world.get::<PartyInventory>(*party).is_some()
                && GameData::items(world).is_some_and(|catalog| catalog.get(*item).is_some())
        }
        QuoteEffect::TakeItem { party, .. } => world.get::<PartyInventory>(*party).is_some(),
        QuoteEffect::Disposition { npc, .. } => world.get::<Npc>(*npc).is_some(),
        QuoteEffect::PartyRemark(_) | QuoteEffect::Resume => true,
    })
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
    let feet = PartyBody::feet_in(world).unwrap_or_default();
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
/// What scripted gifts and removals change: each member's pack, in party
/// order, and the party's items. An answer previews its exchange on a copy
/// and commits it through the same rules.
struct PartyItems {
    packs: Vec<(Entity, Carried)>,
    pool: ItemPool,
}
impl PartyItems {
    fn of(world: &World, party: Entity) -> Option<Self> {
        // A party without members has no roster, only its own items.
        let packs = world
            .get::<PartyRoster>(party)
            .into_iter()
            .flat_map(|roster| roster.iter())
            .filter_map(|member| Some((member, world.get::<Carried>(member)?.clone())))
            .collect();
        Some(Self { packs, pool: world.get::<PartyInventory>(party)?.pool.clone() })
    }
    fn store(self, world: &mut World, party: Entity) {
        for (member, carried) in self.packs {
            if let Some(mut current) = world.get_mut::<Carried>(member) {
                current.set_if_neq(carried);
            }
        }
        let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
        if inventory.pool != self.pool {
            inventory.pool = self.pool;
        }
    }
    /// One of the offered stack leaves its member's pack or the party's items.
    fn hand_in(&mut self, offered: ItemRef) -> bool {
        match offered.owner {
            ItemOwner::Pool => self.pool.consume_units(offered.id, 1).is_ok(),
            ItemOwner::Member(member) => self
                .packs
                .iter_mut()
                .find(|(owner, _)| *owner == member)
                .is_some_and(|(_, pack)| pack.consume_backpack_units(offered.id, 1).is_ok()),
        }
    }
    /// One of the item leaves the first member's pack that holds it, else the
    /// party's items (retail `RemoveNpcScriptItem`). Says whose it was: a
    /// member, or `None` for the party's; nothing when the party has none.
    fn take(&mut self, item: ItemId) -> Option<Option<Entity>> {
        for (member, pack) in &mut self.packs {
            if let Some(id) = pack.backpack.iter().flatten().find(|s| s.item == item).map(|s| s.id)
            {
                pack.consume_backpack_units(id, 1).ok()?;
                return Some(Some(*member));
            }
        }
        let id = self.pool.stacks.iter().find(|s| s.item == item)?.id;
        self.pool.consume_units(id, 1).ok()?;
        Some(None)
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
            Continuation::Options { words, typed } => Step::Options { quote, words, typed },
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
                let facts = world.resource::<NpcAssets>().facts.clone();
                world.resource_scope(|world, catalogs: Mut<Assets<FactCatalog>>| {
                    world
                        .resource_mut::<StoryFacts>()
                        .set(id, value, catalogs.get(&facts).unwrap(), level)
                        .expect("planned fact assignment");
                });
            }
            QuoteEffect::Mention(category, text) => learn_topic(world, entity, category, &text),
            QuoteEffect::Gold { party, amount } => {
                let mut purse = world.get_mut::<PartyInventory>(party).unwrap();
                purse.gold = purse.gold.wrapping_add(amount);
                let actor = world.get::<Conversation>(entity).unwrap().actor;
                let giver = definition(world, actor).unwrap().source_name.clone();
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
                let (name, crucial, rest) = with_items(world, |world, catalog| {
                    let definition = catalog.get(item).ok_or(TradeError::MissingContent)?;
                    let stack = world.resource_scope(|world, mut ids: Mut<ItemIds>| {
                        let mut rng = world.resource_mut::<GameRng>();
                        make_item(definition, &mut ids, &mut rng, true, true)
                    });
                    let rest = world
                        .get_mut::<PartyInventory>(party)
                        .unwrap()
                        .pool
                        .receive(stack, catalog);
                    Ok((definition.name.clone(), definition.crucial, rest))
                })
                .expect("a validated gift has its item");
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
                let Some(mut items) = PartyItems::of(world, party) else { continue };
                let taken = items.take(item);
                items.store(world, party);
                if let Some(from) = taken {
                    let member = from.map(|member| {
                        world.get::<Character>(member).map(|c| c.name.clone()).unwrap_or_default()
                    });
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
                let now = world.resource::<crate::game::time::GameClock>().ms;
                if let Some(news) = world.resource_mut::<Factions>().adjust(faction, delta, now) {
                    world.write_message(news);
                }
            }
        }
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
    let party = the_party(world);
    world.resource_scope(|world, scripts: Mut<Assets<DialogueScript>>| {
        // Asset loading is not a terminal dialogue error. Keep the queued work
        // and retry when its immutable definition is available.
        if let Some(script) = scripts.get(&handle) {
            run_script(world, entity, script, party);
        }
    });
}
fn run_script(world: &mut World, entity: Entity, script: &DialogueScript, party: Option<Entity>) {
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
            Step::Options { quote, words, typed } => {
                let mut c = world.get_mut::<Conversation>(entity).unwrap();
                c.wait = ConversationWait::Options { quote, words, typed };
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
                    let charm = world.get::<Conversation>(entity).unwrap().charm().max(0) as u32;
                    let gold = gold - gold.saturating_mul(charm) / 100;
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
                let plan = match plan.map_or_else(|| plan_quote(world, party, q), Ok) {
                    Ok(plan) => plan,
                    Err((entry, reason)) => {
                        let mut c = world.get_mut::<Conversation>(entity).unwrap();
                        c.queue.push_front(Step::Quote { quote, subquote, paid, plan: None });
                        block(&mut c, quote, Some(entry), reason);
                        return;
                    }
                };
                if !executable(world, &plan) {
                    let mut c = world.get_mut::<Conversation>(entity).unwrap();
                    c.queue.push_front(Step::Quote { quote, subquote, paid, plan: Some(plan) });
                    block(&mut c, quote, None, "accepted reward recipient unavailable".into());
                    return;
                }
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
                let Ok(def) = definition(world, actor) else {
                    block(
                        &mut world.get_mut::<Conversation>(entity).unwrap(),
                        quote,
                        None,
                        "NPC definition unavailable".into(),
                    );
                    return;
                };
                // Retail `SpeakNpcSubquote`: an alert line is shown with the
                // alert chime and no recording; otherwise a few words stand
                // for something other than a line.
                let is = |word: &str| text.eq_ignore_ascii_case(word);
                let voice =
                    if q.alert {
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
                        world.get_mut::<Conversation>(entity).unwrap().queue.push_back(
                            Step::Quote { quote: next, subquote: 0, paid: false, plan: None },
                        );
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
    if !c.ready() || c.menu {
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
    let now = world.resource::<crate::game::time::GameClock>().ms;
    let npc = world.get::<Npc>(actor).unwrap();
    let at = match approach {
        Approach::Charm => npc.charmed_at,
        Approach::SmallTalk => npc.small_talk_at,
    };
    let Ok(def) = definition(world, actor) else { return };
    let scale = i32::from(match approach {
        Approach::Charm => def.charm_scale,
        Approach::SmallTalk => def.talk_scale,
    });
    if !c.menu || !approach_ready(at, now) {
        return;
    }
    let Some((speaker, level)) = best_communicator(world) else { return };
    let mut delta = scale + level as i32 * (scale.abs() / 5) / 100;
    if approach == Approach::Charm && delta > 0 {
        let average = crate::game::party::average_level(super::able_levels(world));
        let own = active_body(world, actor)
            .and_then(|body| world.get::<Monster>(body))
            .and_then(|m| GameData::species(world)?.kind(m.species))
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
        (Band::Friendly, Approach::SmallTalk) => greet(world, actor).ok(),
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
        c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
    }
    if !friendly && answer == Some(HOSTILE_QUOTE) {
        c.queue.push_back(Step::End);
    }
    c.menu = !friendly;
    c.revision += 1;
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
    let refill = definition(world, actor).map_or(0, |d| d.trade_pool);
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
    let greeting = if greeting { greet(world, actor).ok() } else { None };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.menu = false;
    if let Some(quote) = greeting {
        c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
    }
    c.revision += 1;
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
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    let c = world.get::<Conversation>(entity).unwrap();
    if !c.ready() {
        return Err(TradeError::Busy);
    }
    let actor = c.actor;
    let stack = sale_item(world, TradeSource::Party(item), party)?;
    let (offered, count) = (stack.item, stack.count());
    let definition_of = GameData::items(world)
        .and_then(|catalog| catalog.get(offered))
        .ok_or(TradeError::MissingContent)?;
    let (crucial, value) = (definition_of.crucial, stack_value(definition_of, count));
    let def = definition(world, actor)?;
    let (voiced, kind) = (def.voice_script, def.kind);
    let script = script(world, c).ok_or(TradeError::MissingContent)?;
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
    let script = self::script(world, world.get::<Conversation>(entity).unwrap())
        .ok_or(TradeError::MissingContent)?;
    let plan = script
        .quotes
        .get(usize::from(quote))
        .and_then(|line| plan_quote(world, Some(party), line).ok());
    // Retail takes the item at once; the answer's effects follow at its last
    // line, which plays out even if the party leaves the dialogue. A content
    // gap keeps the item and queues the answer to show it.
    if keeps && plan.as_ref().is_some_and(|plan| executable(world, plan)) {
        let mut items = PartyItems::of(world, party).ok_or(TradeError::Unavailable)?;
        if !items.hand_in(item) {
            return Err(TradeError::StaleSelection);
        }
        items.store(world, party);
        if let Some((pool, disposition)) = debit {
            let mut npc = world.get_mut::<Npc>(actor).unwrap();
            (npc.trade_pool, npc.disposition) = (pool, disposition);
        }
    }
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan });
    c.revision += 1;
    won_over(world, entity, true);
    Ok(())
}
/// The party gives the NPC gold (retail `HandleNpcDialogueItem` with trade
/// gold): all of it only once the party confirms, as retail asks.
fn give_gold(world: &mut World, entity: Entity, amount: u32) -> Result<(), TradeError> {
    let party = the_party(world).ok_or(TradeError::Unavailable)?;
    if !world.get::<Conversation>(entity).unwrap().ready() {
        return Err(TradeError::Busy);
    }
    let gold = world.get::<PartyInventory>(party).ok_or(TradeError::Unavailable)?.gold;
    if amount == 0 {
        return Err(TradeError::InvalidQuantity);
    }
    if amount > gold {
        return Err(TradeError::Funds);
    }
    if amount == gold {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.wait = ConversationWait::GiveAll { gold };
        c.revision += 1;
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
    if bribe && definition(world, actor)?.trade_pool == 0 {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.queue.push_back(Step::Quote {
            quote: UNBRIBED_QUOTE,
            subquote: 0,
            paid: true,
            plan: None,
        });
        c.revision += 1;
        return Ok(());
    }
    let mut inventory = world.get_mut::<PartyInventory>(party).ok_or(TradeError::Unavailable)?;
    inventory.gold = inventory.gold.checked_sub(amount).ok_or(TradeError::Funds)?;
    if bribe {
        debit_trade_pool(world, actor, amount);
    }
    let quote =
        if band(world, actor) == Band::Friendly { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE };
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.queue.push_back(Step::Quote { quote, subquote: 0, paid: true, plan: None });
    c.revision += 1;
    won_over(world, entity, !all);
    Ok(())
}
fn reply(world: &mut World, entity: Entity, text: &str) {
    let c = world.get::<Conversation>(entity).unwrap();
    let Some(script) = script(world, c) else { return };
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
            c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
            c.revision += 1;
        }
    }
}
fn answer_price(world: &mut World, entity: Entity, accept: bool) {
    let Some(party) = the_party(world) else { return };
    let c = world.get::<Conversation>(entity).unwrap();
    if let ConversationWait::GiveAll { gold } = c.wait {
        let mut c = world.get_mut::<Conversation>(entity).unwrap();
        c.wait = ConversationWait::Ready;
        c.revision += 1;
        if accept && let Err(refused) = spend_gift(world, entity, party, gold, true) {
            world.get_mut::<Conversation>(entity).unwrap().report(Outcome::GiftRefused(refused));
        }
        return;
    }
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
                let plan = match plan_quote(world, Some(party), line) {
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
                // Nothing is charged for an answer that cannot commit.
                if !executable(world, &plan) {
                    block(
                        &mut world.get_mut::<Conversation>(entity).unwrap(),
                        quote,
                        None,
                        "accepted reward recipient unavailable".into(),
                    );
                    return;
                }
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
    let occupied = crate::game::world::party_occupied(world);
    let c = world.get::<Conversation>(quote.conversation).ok_or(TradeError::StaleSelection)?;
    if !c.ready() || c.revision != quote.revision {
        return Err(TradeError::StaleSelection);
    }
    if !available(world, c.actor) || !trade_allowed(world, c.actor) || occupied {
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
            | ConversationRequest::EditNotebook { conversation, .. }
            | ConversationRequest::Reply { conversation, .. }
            | ConversationRequest::PriceAnswer { conversation, .. }
            | ConversationRequest::Recruit { conversation, .. }
            | ConversationRequest::Advance { conversation, .. }
            | ConversationRequest::Close { conversation }
            | ConversationRequest::Give { conversation, .. }
            | ConversationRequest::GiveGold { conversation, .. }
            | ConversationRequest::Approach { conversation, .. } => *conversation,
            ConversationRequest::Trade(q) => q.conversation,
            _ => unreachable!(),
        };
        // Nothing but its lines' playback reaches a dialogue the party left.
        if world.get::<Closing>(entity).is_some()
            && !matches!(request, ConversationRequest::Advance { .. })
        {
            continue;
        }
        if matches!(request, ConversationRequest::Close { .. }) {
            leave(world, entity);
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
            ConversationRequest::EditNotebook { revision: requested, edit, text, .. }
                if requested == revision =>
            {
                let mut facts = world.resource_mut::<StoryFacts>();
                match edit {
                    NotebookEdit::Remember => {
                        facts.remember_topic(TopicCategory::Misc, &text);
                    }
                    NotebookEdit::Forget => {
                        facts.forget_topic(&text);
                    }
                }
            }
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
            ConversationRequest::Give { revision: requested, item, .. }
                if requested == revision =>
            {
                if let Err(refused) = give(world, entity, item) {
                    world
                        .get_mut::<Conversation>(entity)
                        .unwrap()
                        .report(Outcome::GiftRefused(refused));
                }
            }
            ConversationRequest::GiveGold { revision: requested, amount, .. }
                if requested == revision =>
            {
                if let Err(refused) = give_gold(world, entity, amount) {
                    world
                        .get_mut::<Conversation>(entity)
                        .unwrap()
                        .report(Outcome::GiftRefused(refused));
                }
            }
            ConversationRequest::Approach { revision: requested, approach, .. }
                if requested == revision =>
            {
                approach_npc(world, entity, approach)
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
            .add_message::<crate::game::factions::FactionNews>()
            .add_message::<NpcSpeech>()
            .add_message::<crate::game::inventory::ItemNews>()
            .init_resource::<StoryFacts>()
            .init_resource::<GameRng>()
            .init_resource::<crate::game::time::GameClock>()
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
            .spawn((
                super::super::ActorId::Named(NpcId(0)),
                Npc { disposition: 80, greeting_pending: true, ..default() },
            ))
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
                spell_points: 0,
                body: ScaledBody { scale: 1., radius: 0.5, eye_height: 1., speed: 0. },
            },
        ));
        let entity = w
            .spawn(Conversation {
                actor,
                script,
                revision: 0,
                charms: default(),
                current: None,
                wait: ConversationWait::Ready,
                transcript: vec![],
                learned_topics: default(),
                menu: false,
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
        let answer = |character: bool,
                      companions: usize,
                      gear: Option<crate::content::items::ItemId>| {
            let quotes =
                (0..16).map(|i| Quote { text: vec![format!("quote {i}")], ..default() }).collect();
            let (mut app, session, _) = app(DialogueScript { quotes, ..default() });
            let w = app.world_mut();
            w.init_resource::<Assets<ItemCatalog>>();
            w.init_resource::<Assets<crate::content::items::LootTables>>();
            w.init_resource::<Assets<crate::content::species::MonsterCatalog>>();
            w.init_resource::<Assets<crate::content::spells::SpellCatalog>>();
            let data = GameData {
                items: w.resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog::default()),
                loot: w
                    .resource_mut::<Assets<crate::content::items::LootTables>>()
                    .add(crate::content::items::LootTables::default()),
                species: w
                    .resource_mut::<Assets<crate::content::species::MonsterCatalog>>()
                    .add(crate::content::species::MonsterCatalog::default()),
                spells: w
                    .resource_mut::<Assets<crate::content::spells::SpellCatalog>>()
                    .add(crate::content::spells::SpellCatalog::default()),
            };
            w.insert_resource(data);
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
                        equipment: [
                            gear, None, None, None, None, None, None, None, None, None, None, None,
                        ],
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
        assert_eq!(answer(false, 0, None), (Some((DECLINES_TO_JOIN, 0)), false));
        assert_eq!(answer(true, 2, None), (Some((NO_ROOM_TO_JOIN, 0)), false));
        assert_eq!(answer(true, 1, None), (Some((JOINS, 0)), true));
        // Gear the catalog lacks is missing content: it declines, nothing made.
        assert_eq!(
            answer(true, 1, Some(crate::content::items::ItemId(5))),
            (Some((DECLINES_TO_JOIN, 0)), false)
        );
    }

    #[test]
    fn dialogue_magic_validates_before_payment_charm_is_scoped_and_mindread_advances_responses() {
        use crate::content::{
            characters::{Realm, Skill},
            items::ItemCatalog,
            species::{MonsterCatalog, Species},
            spells::{SpellCatalog, SpellDefinition, SpellId, Spellbook, Usable},
        };
        use crate::game::{
            characters::{Gender, Profession, Race},
            difficulty::Difficulty,
            inventory::Burden,
            magic::{
                LearnedSpells, SpellPoints,
                casting::{Aim, MagicChoice, Refusal, Source},
            },
        };
        let (mut app, session, actor) =
            app(DialogueScript { quotes: vec![Quote::default(); 106], ..default() });
        let world = app.world_mut();
        world.init_resource::<Assets<ItemCatalog>>();
        world.init_resource::<Assets<MonsterCatalog>>();
        world.init_resource::<Assets<SpellCatalog>>();
        world.init_resource::<Difficulty>();
        world.init_resource::<Messages<crate::game::magic::casting::MagicNews>>();
        let items = world.resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog::default());
        let species = world
            .resource_mut::<Assets<MonsterCatalog>>()
            .add(MonsterCatalog(vec![Species::default()]));
        let mut spells = vec![SpellDefinition::default(); 42];
        for id in [SpellId::CHARM, SpellId::MINDREAD] {
            spells[id.0 as usize] = SpellDefinition {
                id,
                realm: Some(Realm::Mental),
                cost: 1,
                usable: Usable::Camped,
                books: Spellbook::Wizardry.into(),
                ..default()
            };
        }
        let spells = world.resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(spells));
        world.insert_resource(GameData { items, species, spells, loot: default() });
        let mut profile = Character::new("Reader", Race::Human, Gender::Male, Profession::Mage);

        profile.profession_levels[Profession::Mage] = 30;
        profile.skill_points[Skill::Wizardry] = 100;
        profile.skill_points[Skill::MentalMagic] = 100;
        let caster = world
            .spawn((
                Vitals::new(&profile),
                profile,
                Carried::default(),
                Burden::default(),
                LearnedSpells([SpellId::CHARM, SpellId::MINDREAD].into()),
                SpellPoints::default(),
            ))
            .id();
        crate::game::magic::test_set_points(world, caster, Realm::Mental, 100);
        crate::game::party::roster::test_join(world, caster);
        let body = active_body(world, actor).unwrap();
        let choice = |spell| MagicChoice {
            source: Source::Spell { spell, power: 1 },
            aim: Aim::Monster(body),
        };
        let rng = world.resource::<GameRng>().clone();
        assert_eq!(
            crate::game::magic::dialogue::cast(world, caster, choice(SpellId::CHARM)),
            Err(Refusal::WrongTime)
        );
        assert_eq!(world.resource::<GameRng>(), &rng);
        assert_eq!(crate::game::magic::test_points(world, caster, Realm::Mental), 100);
        world.get_mut::<Conversation>(session).unwrap().queue.clear();
        // Select a successful save roll without assuming retail's RNG sequence.
        for seed in 0..30 {
            world.insert_resource(GameRng::seeded(seed));
            crate::game::magic::dialogue::cast(world, caster, choice(SpellId::CHARM)).unwrap();
            if world.get::<Conversation>(session).unwrap().charm() > 0 {
                break;
            }
        }
        assert_eq!(world.get::<Conversation>(session).unwrap().charm(), 7);
        assert_eq!(
            world.get::<Npc>(actor).unwrap().disposition,
            80,
            "charm does not overwrite retained opinion"
        );
        for expected in [0x67, 0x68, 0x69, 0x69] {
            for seed in 0..30 {
                world.insert_resource(GameRng::seeded(seed));
                crate::game::magic::dialogue::cast(world, caster, choice(SpellId::MINDREAD))
                    .unwrap();
                let quote = match world
                    .get_mut::<Conversation>(session)
                    .unwrap()
                    .queue
                    .pop_front()
                    .unwrap()
                {
                    Step::Quote { quote, .. } => quote,
                    _ => panic!(),
                };
                let required = ((expected - 0x67) as u8 + 1).min(2);
                if quote == expected
                    && world.get::<Npc>(actor).unwrap().mindread_refusals == required
                {
                    break;
                }
                assert_eq!(quote, 0x69, "a resisted attempt uses the refusal quote");
            }
            assert_eq!(
                world.get::<Npc>(actor).unwrap().mindread_refusals,
                ((expected - 0x67) as u8 + 1).min(2)
            );
        }
        world.despawn(session);
        assert_eq!(world.get::<Npc>(actor).unwrap().disposition, 80);
        assert_eq!(world.get::<Npc>(actor).unwrap().mindread_refusals, 2);
    }

    #[test]
    fn quest_rewards_commit_once_to_living_roster_members_and_the_faction_owner() {
        use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};
        let operations = vec![
            DialogueOp::GiveGold(500),
            DialogueOp::GiveExperience(100),
            DialogueOp::AdjustFaction { faction: Some(Faction::Umpani), delta: -50 },
            DialogueOp::SetFact(FactId(91), 1),
            DialogueOp::Next(Response::Fixed(1)),
        ];
        for blocked in [false, true] {
            let mut ops = operations.clone();
            if blocked {
                ops.insert(
                    3,
                    DialogueOp::Unsupported { opcode: 99, operands: [0; 3], subentries: vec![] },
                );
            }
            let (mut app, session, _) = app(DialogueScript {
                quotes: vec![
                    Quote { text: vec!["Reward".into()], operations: ops, ..default() },
                    Quote { text: vec!["Done".into()], ..default() },
                ],
                ..default()
            });
            app.init_resource::<Factions>();
            let world = app.world_mut();
            let party = the_party(world).unwrap();
            let members: Vec<_> = (0..4)
                .map(|index| {
                    let mut character =
                        Character::new("Member", Race::Human, Gender::Male, Profession::Fighter);
                    if index == 0 {
                        character.experience = u32::MAX - 50;
                    }
                    let mut vitals = Vitals::new(&character);
                    if index == 1 {
                        vitals.conditions.push((Condition::Unconscious, Lasting::Indefinite));
                    }
                    if index == 2 {
                        vitals.hp = 0;
                    }
                    let member = world.spawn((character, vitals)).id();
                    if index != 3 {
                        crate::game::party::roster::test_join(world, member);
                    }
                    member
                })
                .collect();
            app.update();
            let world = app.world();
            if blocked {
                assert!(matches!(
                    world.get::<Conversation>(session).unwrap().wait,
                    ConversationWait::Blocked { .. }
                ));
                assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 0);
                assert_eq!(world.get::<Character>(members[0]).unwrap().experience, u32::MAX - 50);
                assert_eq!(world.get::<Character>(members[1]).unwrap().experience, 0);
                assert_eq!(world.resource::<Factions>().score(Faction::Umpani as i32), Some(80));
                assert!(world.resource::<StoryFacts>().journal.is_empty());
                continue;
            }
            assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 500);
            assert_eq!(world.get::<Character>(members[0]).unwrap().experience, u32::MAX);
            assert_eq!(world.get::<Character>(members[1]).unwrap().experience, 100);
            assert_eq!(world.get::<Character>(members[2]).unwrap().experience, 0);
            assert_eq!(world.get::<Character>(members[3]).unwrap().experience, 0);
            assert_eq!(world.resource::<Factions>().score(Faction::Umpani as i32), Some(30));
            assert_eq!(world.resource::<StoryFacts>().values[&FactId(91)], 1);
            let speech = match world.get::<Conversation>(session).unwrap().wait {
                ConversationWait::Line { speech, .. } => speech,
                _ => panic!("reward line expected"),
            };
            for _ in 0..2 {
                app.world_mut()
                    .write_message(ConversationRequest::Advance { conversation: session, speech });
            }
            app.update();
            let world = app.world();
            assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, 500);
            assert_eq!(world.get::<Character>(members[1]).unwrap().experience, 100);
            assert_eq!(
                world
                    .get::<Conversation>(session)
                    .unwrap()
                    .transcript
                    .iter()
                    .filter(|line| matches!(line, TranscriptLine::Reward(_)))
                    .count(),
                2
            );
            let news = world.resource::<Messages<crate::game::factions::FactionNews>>();
            assert_eq!(
                news.iter_current_update_messages().copied().collect::<Vec<_>>(),
                [crate::game::factions::FactionNews {
                    faction: Faction::Umpani as i32,
                    improved: false
                }],
                "the faction notice goes to the text box once"
            );
        }
    }

    /// Items 0 to 3 in a stackable catalog; item 3 may not be given away.
    fn with_items(app: &mut App) {
        use crate::content::items::{Dice, ItemDefinition, QuantityKind};
        let world = app.world_mut();
        world.init_resource::<Assets<ItemCatalog>>();
        world.init_resource::<ItemIds>();
        let items = (0..4)
            .map(|id| ItemDefinition {
                id: ItemId(id),
                name: format!("Item {id}"),
                quantity: QuantityKind::Stack,
                stack_limit: 20,
                initial_quantity: Dice { base: 5, ..default() },
                crucial: id == 3,
                ..default()
            })
            .collect();
        let items = world.resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog(items));
        world.insert_resource(GameData {
            items,
            loot: default(),
            species: default(),
            spells: default(),
        });
    }

    fn stack(app: &mut App, item: u16, count: u16) -> crate::game::inventory::ItemStack {
        let world = app.world_mut();
        let definition = GameData::items(world).unwrap().get(ItemId(item)).unwrap().clone();
        crate::game::inventory::ItemStack::new(
            &definition,
            &mut world.resource_mut::<ItemIds>(),
            count,
            true,
        )
    }

    /// A member carrying `count` of `item` in their first backpack slot.
    fn carrier(app: &mut App, name: &str, item: u16, count: u16) -> Entity {
        use crate::game::characters::{Gender, Profession, Race};
        let mut carried = Carried::default();
        carried.backpack[0] = Some(stack(app, item, count));
        let character = Character::new(name, Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&character);
        let member = app.world_mut().spawn((character, vitals, carried)).id();
        crate::game::party::roster::test_join(app.world_mut(), member);
        member
    }

    fn finish_line(app: &mut App, session: Entity) {
        if let ConversationWait::Line { speech, .. } =
            app.world().get::<Conversation>(session).unwrap().wait
        {
            app.world_mut()
                .write_message(ConversationRequest::Advance { conversation: session, speech });
            app.update();
        }
    }

    fn rewards(app: &App, session: Entity) -> Vec<QuestReward> {
        app.world()
            .get::<Conversation>(session)
            .unwrap()
            .transcript
            .iter()
            .filter_map(|line| match line {
                TranscriptLine::Reward(reward) => Some(reward.clone()),
                _ => None,
            })
            .collect()
    }

    /// Retail `ProcessNpcQuoteEntry`: a gift arrives whole and identified in
    /// the party's items; a removal takes one from the first member's pack
    /// that holds it before the party's items.
    #[test]
    fn scripted_items_are_given_whole_and_taken_one_at_a_time() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![
                Quote {
                    text: vec!["Take this".into()],
                    operations: vec![
                        DialogueOp::GiveItem(ItemId(1)),
                        DialogueOp::TakeItem(ItemId(2)),
                        DialogueOp::Next(Response::Fixed(1)),
                    ],
                    ..default()
                },
                Quote { text: vec!["Done".into()], ..default() },
            ],
            ..default()
        });
        with_items(&mut app);
        let member = carrier(&mut app, "Ragnar", 2, 3);
        let party = the_party(app.world_mut()).unwrap();
        let pooled = stack(&mut app, 2, 4);
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(pooled);
        app.update();
        let world = app.world();
        let pool = &world.get::<PartyInventory>(party).unwrap().pool.stacks;
        let gift = pool.iter().find(|s| s.item == ItemId(1)).expect("the gift");
        assert_eq!((gift.count(), gift.identified), (5, true), "whole and identified");
        assert_eq!(pool.iter().find(|s| s.item == ItemId(2)).unwrap().count(), 4);
        let pack = world.get::<Carried>(member).unwrap().backpack[0].as_ref().unwrap();
        assert_eq!(pack.count(), 2, "one taken from the member's pack first");
        assert_eq!(
            rewards(&app, session),
            [
                QuestReward::Item { name: "Item 1".into() },
                QuestReward::Taken { member: Some("Ragnar".into()) }
            ]
        );
    }

    /// Retail `HandleNpcDialogueItem` for a friendly NPC: the answering quote
    /// runs; what it asks for is kept and what it only remarks on comes
    /// back; anything else is taken as a gift unless it may not be given.
    #[test]
    fn offered_items_are_answered_and_kept_only_when_wanted() {
        let mut quotes: Vec<_> =
            (0..18).map(|i| Quote { text: vec![format!("Line {i}")], ..default() }).collect();
        quotes[1].operations =
            vec![DialogueOp::ItemQuote { item: ItemId(1), keeps: true }, DialogueOp::GiveGold(50)];
        quotes[2].operations = vec![DialogueOp::ItemQuote { item: ItemId(2), keeps: false }];
        let (mut app, session, _) = app(DialogueScript { quotes, ..default() });
        with_items(&mut app);
        let party = the_party(app.world_mut()).unwrap();
        let mut refs = Vec::new();
        for item in 0..4 {
            let stack = stack(&mut app, item, 2);
            refs.push(ItemRef { owner: ItemOwner::Pool, id: stack.id });
            app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(stack);
        }
        app.update();
        finish_line(&mut app, session);
        let count = |app: &App, item: usize| {
            app.world()
                .get::<PartyInventory>(party)
                .unwrap()
                .pool
                .stacks
                .iter()
                .find(|s| s.id == refs[item].id)
                .map_or(0, |s| s.count())
        };
        for (item, quote, kept) in [(1, 1, 1), (2, 2, 2), (0, 0x10, 1), (3, 0x11, 2)] {
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            app.world_mut().write_message(ConversationRequest::Give {
                conversation: session,
                revision,
                item: refs[item],
            });
            app.update();
            assert_eq!(
                app.world().get::<Conversation>(session).unwrap().current.map(|c| c.0),
                Some(quote)
            );
            assert_eq!(count(&app, item), kept, "item {item}");
            finish_line(&mut app, session);
        }
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 50);
    }

    /// A second NPC, `name`, in the same level, speaking `script`.
    fn second_speaker(app: &mut App, name: &str, script: DialogueScript) -> Entity {
        let world = app.world_mut();
        let handle = world.resource::<NpcAssets>().catalog.clone();
        let id = {
            let mut catalogs = world.resource_mut::<Assets<NpcCatalog>>();
            let mut catalog = catalogs.get_mut(&handle).unwrap();
            catalog.0.push(NpcDefinition {
                source_name: name.into(),
                display_name: name.into(),
                ..default()
            });
            NpcId(catalog.0.len() as u16 - 1)
        };
        let script = world.resource_mut::<Assets<DialogueScript>>().add(script);
        world.resource_mut::<NpcAssets>().scripts.insert(id, script);
        let level = world.query_filtered::<Entity, With<Level>>().single(world).unwrap();
        let actor = world
            .spawn((
                super::super::ActorId::Named(id),
                Npc { disposition: 80, greeting_pending: true, paid_topics: vec![], ..default() },
            ))
            .id();
        world.spawn((
            super::super::NpcBody(actor),
            crate::game::world::level::LevelOwned(level),
            Monster {
                enchantments: default(),
                location: 2,
                species: 0,
                hp: 5,
                max_hp: 5,
                stamina: 5,
                max_stamina: 5,
                conditions: vec![],
                spell_points: 0,
                body: ScaledBody { scale: 1., radius: 0.5, eye_height: 1., speed: 0. },
            },
        ));
        actor
    }

    /// Retail `ProcessNpcQuoteEntry`: a response naming another NPC hands the
    /// conversation to it, which answers with that quote of its own script;
    /// a name no NPC answers to leaves the conversation waiting.
    #[test]
    fn a_response_naming_another_npc_hands_the_conversation_over() {
        for (named, expected) in [("Gadrock", true), ("Nobody", false)] {
            let (mut app, session, first) = app(DialogueScript {
                quotes: vec![Quote {
                    text: vec!["Ask him".into()],
                    operations: vec![DialogueOp::Next(Response::Speaker {
                        npc: named.into(),
                        quote: 2,
                    })],
                    ..default()
                }],
                ..default()
            });
            let other = second_speaker(
                &mut app,
                "GADROCK",
                DialogueScript {
                    quotes: (0..3)
                        .map(|i| Quote { text: vec![format!("Gadrock {i}")], ..default() })
                        .collect(),
                    ..default()
                },
            );
            app.world_mut()
                .get_mut::<Conversation>(session)
                .unwrap()
                .magical_reply(MagicalReply::Charmed(7));
            assert_eq!(disposition(app.world(), first), 87);
            app.update();
            finish_line(&mut app, session);
            app.update();
            let c = app.world().get::<Conversation>(session).unwrap();
            assert_eq!(
                disposition(app.world(), first),
                87,
                "the original speaker retains the effect"
            );
            assert_eq!(disposition(app.world(), other), 80, "handoff cannot transfer Charm");
            assert_eq!(c.charm(), if expected { 0 } else { 7 });
            if expected {
                assert_eq!((c.actor, c.current), (other, Some((2, 0))));
                assert!(matches!(c.wait, ConversationWait::Line { .. }));
            } else {
                assert_eq!(c.actor, first);
                assert!(c.ready());
            }
        }
    }

    fn lines(texts: &[&str], operations: Vec<DialogueOp>) -> Quote {
        Quote { text: texts.iter().map(|t| (*t).into()).collect(), operations, ..default() }
    }

    /// Retail closes after the line and reads none of the quote's later
    /// entries; the speaker turns on the party (`ResumeNpc`).
    #[test]
    fn close_and_resume_ends_after_the_line_without_later_entries() {
        let (mut app, session, actor) = app(DialogueScript {
            quotes: vec![lines(
                &["Off I go"],
                vec![DialogueOp::CloseAndResume, DialogueOp::GiveGold(5)],
            )],
            ..default()
        });
        let party = the_party(app.world_mut()).unwrap();
        app.update();
        finish_line(&mut app, session);
        assert!(app.world().get::<Conversation>(session).is_none());
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 0);
        assert_eq!(app.world().get::<Npc>(actor).unwrap().disposition, 0x19);
    }

    /// A conversation with a neutral NPC at its topic menu.
    fn wary(scales: (i8, i8), trade_pool: u16) -> (App, Entity, Entity) {
        let quotes = (0..0x19).map(|i| lines(&[format!("Line {i}").as_str()], vec![])).collect();
        let (mut app, session, actor) = app(DialogueScript { quotes, ..default() });
        let catalog = app.world().resource::<NpcAssets>().catalog.clone();
        {
            let mut catalogs = app.world_mut().resource_mut::<Assets<NpcCatalog>>();
            let def = &mut catalogs.get_mut(&catalog).unwrap().0[0];
            (def.charm_scale, def.talk_scale, def.trade_pool) = (scales.0, scales.1, trade_pool);
        }
        let mut npc = app.world_mut().get_mut::<Npc>(actor).unwrap();
        npc.disposition = 50;
        npc.trade_pool = trade_pool;
        use crate::game::characters::{Gender, Profession, Race};
        let character = Character::new("Talker", Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&character);
        let member = app.world_mut().spawn((character, vitals)).id();
        crate::game::party::roster::test_join(app.world_mut(), member);
        let mut c = app.world_mut().get_mut::<Conversation>(session).unwrap();
        c.menu = true;
        c.queue = VecDeque::from([Step::Quote {
            quote: NEUTRAL_GREETING,
            subquote: 0,
            paid: false,
            plan: None,
        }]);
        app.update();
        finish_line(&mut app, session);
        (app, session, actor)
    }
    fn approach(app: &mut App, session: Entity, approach: Approach) {
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::Approach {
            conversation: session,
            revision,
            approach,
        });
        app.update();
    }

    /// Retail `SelectNpcDialogueTalk` and `SelectNpcDialogueService`: the
    /// NPC answers by the band its disposition ends in, and each approach
    /// waits half a day.
    #[test]
    fn small_talk_and_charm_move_a_neutral_npc_by_band() {
        let (mut app, session, actor) = wary((5, 30), 0);
        approach(&mut app, session, Approach::Charm);
        assert_eq!(app.world().get::<Npc>(actor).unwrap().disposition, 55);
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((6, 0)));
        finish_line(&mut app, session);
        // A second charm waits.
        approach(&mut app, session, Approach::Charm);
        assert_eq!(app.world().get::<Npc>(actor).unwrap().disposition, 55);
        approach(&mut app, session, Approach::SmallTalk);
        let c = app.world().get::<Conversation>(session).unwrap();
        assert_eq!(app.world().get::<Npc>(actor).unwrap().disposition, 85);
        // Won over, it greets the party as a friend.
        assert!(!c.menu);
        assert_eq!(c.current, Some((0, 0)));

        let (mut app, session, actor) = wary((0, -40), 0);
        approach(&mut app, session, Approach::SmallTalk);
        assert_eq!(app.world().get::<Npc>(actor).unwrap().disposition, 10);
        assert_eq!(
            app.world().get::<Conversation>(session).unwrap().current,
            Some((HOSTILE_QUOTE, 0))
        );
        finish_line(&mut app, session);
        assert!(app.world().get::<Conversation>(session).is_none());
    }

    /// Retail `ConfirmNpcTradePurchase`: gold an NPC not yet friendly takes
    /// draws on its trade pool and wins it over once that runs out; one
    /// without a pool takes nothing.
    #[test]
    fn gold_bribes_draw_on_the_trade_pool() {
        let (mut app, session, actor) = wary((0, 0), 100);
        let party = the_party(app.world_mut()).unwrap();
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 500;
        for (amount, pool, quote) in [(60, 40, UNBRIBED_QUOTE), (60, 100, ACCEPTED_GIFT_QUOTE)] {
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            app.world_mut().write_message(ConversationRequest::GiveGold {
                conversation: session,
                revision,
                amount,
            });
            app.update();
            assert_eq!(app.world().get::<Npc>(actor).unwrap().trade_pool, pool);
            assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((quote, 0)));
            finish_line(&mut app, session);
        }
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 380);
        assert_eq!(app.world().get::<Npc>(actor).unwrap().disposition, 0x4b);
        let c = app.world().get::<Conversation>(session).unwrap();
        assert!(!c.menu);
        assert_eq!(c.current, Some((0, 0)));

        let (mut app, session, _) = wary((0, 0), 0);
        let party = the_party(app.world_mut()).unwrap();
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 500;
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::GiveGold {
            conversation: session,
            revision,
            amount: 60,
        });
        app.update();
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 500);
        assert_eq!(
            app.world().get::<Conversation>(session).unwrap().current,
            Some((UNBRIBED_QUOTE, 0))
        );
    }

    /// Retail `HandleNpcDialogueItem` pairs a bribe's debit with taking the
    /// item: an answer that cannot run keeps the item and leaves the NPC's
    /// trade pool and disposition as they were.
    #[test]
    fn an_item_bribe_is_debited_only_with_the_item() {
        for runs in [false, true] {
            let (mut app, session, actor) = wary((0, 0), 100);
            with_items(&mut app);
            let items = app.world().resource::<GameData>().items.clone();
            app.world_mut().resource_mut::<Assets<ItemCatalog>>().get_mut(&items).unwrap().0[1]
                .value = 400;
            if !runs {
                let script = app.world().get::<Conversation>(session).unwrap().script.clone();
                let mut scripts = app.world_mut().resource_mut::<Assets<DialogueScript>>();
                scripts.get_mut(&script).unwrap().quotes[usize::from(ACCEPTED_GIFT_QUOTE)] =
                    lines(&["Thanks"], vec![DialogueOp::GiveItem(ItemId(9))]);
            }
            let member = carrier(&mut app, "Ragnar", 1, 1);
            let id = app.world().get::<Carried>(member).unwrap().backpack[0].as_ref().unwrap().id;
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            app.world_mut().write_message(ConversationRequest::Give {
                conversation: session,
                revision,
                item: ItemRef { owner: ItemOwner::Member(member), id },
            });
            app.update();
            let npc = app.world().get::<Npc>(actor).unwrap();
            let kept = app.world().get::<Carried>(member).unwrap().backpack[0].is_some();
            let c = app.world().get::<Conversation>(session).unwrap();
            if runs {
                assert!(!kept);
                assert_eq!((npc.trade_pool, npc.disposition), (100, 0x4b));
                assert!(!c.menu);
            } else {
                assert!(kept);
                assert_eq!((npc.trade_pool, npc.disposition), (100, 50));
                assert!(c.menu);
            }
        }
    }

    /// Retail `GetNpcDisposition`: a factioned NPC answers with its faction's
    /// score while the faction is hostile to the party.
    #[test]
    fn a_hostile_faction_decides_its_members_disposition() {
        let (mut app, _, actor) = app(DialogueScript::default());
        let catalog = app.world().resource::<NpcAssets>().catalog.clone();
        app.world_mut().resource_mut::<Assets<NpcCatalog>>().get_mut(&catalog).unwrap().0[0]
            .faction = Faction::Mook as u8;
        app.world_mut().init_resource::<Factions>();
        assert_eq!(band(app.world(), actor), Band::Friendly);
        app.world_mut().resource_mut::<Factions>().adjust(Faction::Mook, -60, 0);
        assert_eq!(disposition(app.world(), actor), 20);
        assert_eq!(band(app.world(), actor), Band::Hostile);
    }

    /// Retail's keyword dialog: the party types the answer, matched as a
    /// chosen one would be (`FindNpcReplyQuote`).
    #[test]
    fn a_keyword_question_is_answered_by_typing() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![
                lines(
                    &["The password?"],
                    vec![
                        DialogueOp::KeywordInput(vec!["SWORDFISH".into()]),
                        DialogueOp::Reply { response: 1, role: 3, words: vec!["SWORDFISH".into()] },
                        DialogueOp::Reply { response: 2, role: 2, words: vec![] },
                    ],
                ),
                lines(&["Enter"], vec![]),
                lines(&["Wrong"], vec![]),
            ],
            ..default()
        });
        app.update();
        finish_line(&mut app, session);
        let c = app.world().get::<Conversation>(session).unwrap();
        assert!(matches!(c.wait, ConversationWait::Options { typed: true, .. }));
        let revision = c.revision;
        app.world_mut().write_message(ConversationRequest::Reply {
            conversation: session,
            revision,
            text: "swordfish".into(),
        });
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((1, 0)));
    }

    /// Retail `ApplyNpcInteraction` mode 4: the named NPC's disposition moves,
    /// clamped to 0..99; a name no NPC answers to changes nothing. A party
    /// remark goes to presentation; leaving the party touches no one who is
    /// not in it.
    #[test]
    fn scripts_move_another_npcs_disposition_and_ask_for_a_party_remark() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![lines(
                &["There"],
                vec![
                    DialogueOp::AdjustDisposition { npc: "gadrock".into(), delta: 30 },
                    DialogueOp::AdjustDisposition { npc: "Nobody".into(), delta: -30 },
                    DialogueOp::LeaveParty { npc: Some("Gadrock".into()) },
                    DialogueOp::LeaveParty { npc: None },
                    DialogueOp::PartyRemark(511),
                ],
            )],
            ..default()
        });
        app.add_message::<PartyRemark>();
        let other = second_speaker(&mut app, "Gadrock", DialogueScript::default());
        app.update();
        let world = app.world();
        assert!(!matches!(
            world.get::<Conversation>(session).unwrap().wait,
            ConversationWait::Blocked { .. }
        ));
        assert_eq!(world.get::<Npc>(other).unwrap().disposition, 99);
        let remarks = world.resource::<Messages<PartyRemark>>();
        assert_eq!(
            remarks.iter_current_update_messages().copied().collect::<Vec<_>>(),
            [PartyRemark { event: 511 }]
        );
    }

    /// Retail `SpeakNpcSubquote`'s stand-ins: a line it does not know answers
    /// with one of two quotes at random, one it will not say with another, a
    /// recording plays without a line, and nothing to say becomes the NPC's
    /// own words where retail has them; an alert is shown with its chime.
    #[test]
    fn quote_stand_ins_and_alerts_speak_as_retail_does() {
        let mut quotes: Vec<_> =
            (0..0x6a).map(|i| lines(&[format!("Line {i}").as_str()], vec![])).collect();
        quotes[1] = lines(&["UNKNOWN"], vec![]);
        quotes[2] = lines(&["classified"], vec![]);
        quotes[3] = lines(&["SOUND"], vec![]);
        quotes[4] = Quote { alert: true, ..lines(&["Beware"], vec![]) };
        quotes[0x67] = lines(&["EMPTY"], vec![]);
        let (mut app, session, _) = app(DialogueScript { quotes, ..default() });
        app.update();
        finish_line(&mut app, session);
        let speech = |app: &App| {
            let c = app.world().get::<Conversation>(session).unwrap();
            let ConversationWait::Line { speech, .. } = c.wait else { panic!("a line") };
            let line = app.world().get::<ConversationSpeech>(speech).unwrap();
            (c.current.unwrap().0, line.voice.clone(), line.text.clone(), line.alert)
        };
        let ask = |app: &mut App, quote: u16| {
            let mut c = app.world_mut().get_mut::<Conversation>(session).unwrap();
            c.queue.push_back(Step::Quote { quote, subquote: 0, paid: false, plan: None });
            app.update();
        };
        ask(&mut app, 1);
        assert!(UNKNOWN_QUOTES.contains(&speech(&app).0));
        finish_line(&mut app, session);
        ask(&mut app, 2);
        assert_eq!(speech(&app).0, CLASSIFIED_QUOTE);
        finish_line(&mut app, session);
        ask(&mut app, 3);
        let (_, voice, text, _) = speech(&app);
        assert!(voice.is_some() && text.is_empty(), "a recording without a line");
        finish_line(&mut app, session);
        ask(&mut app, 4);
        assert_eq!(speech(&app), (4, None, "Beware".into(), true));
        finish_line(&mut app, session);
        ask(&mut app, 0x67);
        assert!(
            app.world()
                .get::<Conversation>(session)
                .unwrap()
                .transcript
                .contains(&TranscriptLine::NoAnswer { npc: "Speaker".into(), second: false })
        );
    }

    /// Fill the party's items to `rows` full, unmergeable stacks of item 0.
    fn fill_pool(app: &mut App, rows: usize) -> Entity {
        let party = the_party(app.world_mut()).unwrap();
        for _ in 0..rows {
            let full = stack(app, 0, 20);
            app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(full);
        }
        party
    }

    fn reward_script(operations: Vec<DialogueOp>, lines: usize) -> DialogueScript {
        DialogueScript {
            quotes: vec![Quote {
                text: (0..lines).map(|i| format!("Line {i}")).collect(),
                operations,
                ..default()
            }],
            ..default()
        }
    }

    /// Retail `AddItemToPartyOrDrop`: a gift with no row left lands at the
    /// party's feet, and one the party may not part with goes to the hand.
    #[test]
    fn gifts_without_room_land_on_the_ground_or_in_hand() {
        let (mut app, session, _) = app(reward_script(
            vec![
                DialogueOp::GiveGold(10),
                DialogueOp::GiveItem(ItemId(1)),
                DialogueOp::GiveItem(ItemId(2)),
                DialogueOp::GiveItem(ItemId(3)),
            ],
            1,
        ));
        with_items(&mut app);
        let party = fill_pool(&mut app, crate::game::inventory::POOL_LIMIT - 1);
        app.update();
        let world = app.world_mut();
        assert!(!matches!(
            world.get::<Conversation>(session).unwrap().wait,
            ConversationWait::Blocked { .. }
        ));
        let inventory = world.get::<PartyInventory>(party).unwrap();
        assert_eq!(inventory.gold, 10);
        assert_eq!(inventory.pool.stacks.len(), crate::game::inventory::POOL_LIMIT);
        assert!(inventory.pool.stacks.iter().any(|s| s.item == ItemId(1)));
        assert_eq!(inventory.hand.held.as_ref().map(|s| s.item), Some(ItemId(3)), "crucial");
        let dropped: Vec<_> = world
            .query::<&crate::game::world::items::WorldItem>()
            .iter(world)
            .map(|lying| lying.appearance())
            .collect();
        assert_eq!(dropped, [ItemId(2)]);
    }

    /// A full set of rows still takes a gift that merges into one of them.
    #[test]
    fn a_gift_merges_into_a_full_pool() {
        let (mut app, session, _) = app(reward_script(vec![DialogueOp::GiveItem(ItemId(1))], 1));
        with_items(&mut app);
        let party = fill_pool(&mut app, crate::game::inventory::POOL_LIMIT - 1);
        let partial = stack(&mut app, 1, 5);
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(partial);
        app.update();
        let world = app.world();
        assert!(!matches!(
            world.get::<Conversation>(session).unwrap().wait,
            ConversationWait::Blocked { .. }
        ));
        let pool = &world.get::<PartyInventory>(party).unwrap().pool.stacks;
        assert_eq!(pool.len(), crate::game::inventory::POOL_LIMIT);
        assert_eq!(pool.iter().find(|s| s.item == ItemId(1)).unwrap().count(), 10);
    }

    /// Content that disappears before an answer's last line holds the whole
    /// answer back: no gift is lost and no other effect commits.
    #[test]
    fn missing_item_content_at_the_last_line_holds_the_whole_answer() {
        let (mut app, session, _) =
            app(reward_script(vec![DialogueOp::GiveGold(10), DialogueOp::GiveItem(ItemId(1))], 2));
        with_items(&mut app);
        let party = the_party(app.world_mut()).unwrap();
        app.update();
        let handle = app.world().resource::<GameData>().items.clone();
        app.world_mut().resource_mut::<Assets<ItemCatalog>>().remove(&handle);
        finish_line(&mut app, session);
        let world = app.world();
        assert!(matches!(
            world.get::<Conversation>(session).unwrap().wait,
            ConversationWait::Blocked { .. }
        ));
        let inventory = world.get::<PartyInventory>(party).unwrap();
        assert_eq!((inventory.gold, inventory.pool.stacks.len()), (0, 0));
    }

    /// Retail keeps its message queue when the dialogue closes: a handed-in
    /// item's answer plays out in the world and commits at its last line.
    #[test]
    fn leaving_during_a_hand_in_answer_lets_it_play_out() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![
                Quote { text: vec!["Hello".into()], ..default() },
                Quote {
                    text: vec!["Thanks".into(), "Here".into()],
                    operations: vec![
                        DialogueOp::ItemQuote { item: ItemId(2), keeps: true },
                        DialogueOp::GiveGold(50),
                        DialogueOp::GiveItem(ItemId(1)),
                    ],
                    ..default()
                },
            ],
            ..default()
        });
        with_items(&mut app);
        let member = carrier(&mut app, "Ragnar", 2, 1);
        let party = the_party(app.world_mut()).unwrap();
        app.update();
        finish_line(&mut app, session);
        let id = app.world().get::<Carried>(member).unwrap().backpack[0].as_ref().unwrap().id;
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::Give {
            conversation: session,
            revision,
            item: ItemRef { owner: ItemOwner::Member(member), id },
        });
        app.update();
        assert!(app.world().get::<Carried>(member).unwrap().backpack[0].is_none(), "taken");
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 0, "not yet");
        app.world_mut().write_message(ConversationRequest::Close { conversation: session });
        app.update();
        assert!(app.world().get::<Closing>(session).is_some());
        finish_line(&mut app, session);
        let world = app.world();
        let said: Vec<_> = world
            .resource::<Messages<NpcSpeech>>()
            .iter_current_update_messages()
            .cloned()
            .collect();
        assert_eq!(said, [NpcSpeech { speaker: "Speaker".into(), text: "Here".into() }]);
        let inventory = world.get::<PartyInventory>(party).unwrap();
        assert_eq!(inventory.gold, 50, "rewarded at the last line");
        assert!(inventory.pool.stacks.iter().any(|s| s.item == ItemId(1)));
        finish_line(&mut app, session);
        assert!(app.world().get::<Conversation>(session).is_none(), "ended with its lines");
    }

    /// Travel or the speaker's absence ends a conversation mid-answer; the
    /// hand-in it accepted still commits its effects.
    #[test]
    fn an_accepted_hand_in_settles_when_the_conversation_is_cut_short() {
        for travel in [true, false] {
            let (mut app, session, actor) = app(DialogueScript {
                quotes: vec![
                    Quote { text: vec!["Hello".into()], ..default() },
                    Quote {
                        text: vec!["Thanks".into(), "Here".into()],
                        operations: vec![
                            DialogueOp::ItemQuote { item: ItemId(2), keeps: true },
                            DialogueOp::GiveGold(50),
                        ],
                        ..default()
                    },
                ],
                ..default()
            });
            with_items(&mut app);
            let member = carrier(&mut app, "Ragnar", 2, 1);
            let party = the_party(app.world_mut()).unwrap();
            app.update();
            finish_line(&mut app, session);
            let id = app.world().get::<Carried>(member).unwrap().backpack[0].as_ref().unwrap().id;
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            app.world_mut().write_message(ConversationRequest::Give {
                conversation: session,
                revision,
                item: ItemRef { owner: ItemOwner::Member(member), id },
            });
            app.update();
            assert!(app.world().get::<Carried>(member).unwrap().backpack[0].is_none());
            if travel {
                app.world_mut().spawn(crate::game::world::level::Destination::new(
                    Level::test(default()),
                    default(),
                    crate::game::world::level::Entry::Travel,
                ));
            } else {
                let body = app.world().get::<super::super::NpcBodies>(actor).unwrap()[0];
                app.world_mut().despawn(body);
                app.update();
            }
            assert!(app.world().get::<Conversation>(session).is_none());
            assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 50);
        }
    }

    #[test]
    fn interrupted_exchange_waits_for_missing_reward_content_without_losing_or_repeating_it() {
        let (mut app, session, actor) = app(DialogueScript {
            quotes: vec![
                Quote { text: vec!["Hello".into()], ..default() },
                Quote {
                    text: vec!["Thanks".into(), "Here".into()],
                    operations: vec![
                        DialogueOp::ItemQuote { item: ItemId(2), keeps: true },
                        DialogueOp::GiveGold(50),
                        DialogueOp::GiveItem(ItemId(1)),
                    ],
                    ..default()
                },
            ],
            ..default()
        });
        with_items(&mut app);
        let member = carrier(&mut app, "Ragnar", 2, 1);
        let party = the_party(app.world_mut()).unwrap();
        app.update();
        finish_line(&mut app, session);
        let id = app.world().get::<Carried>(member).unwrap().backpack[0].as_ref().unwrap().id;
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::Give {
            conversation: session,
            revision,
            item: ItemRef { owner: ItemOwner::Member(member), id },
        });
        app.update();
        assert!(app.world().get::<Carried>(member).unwrap().backpack[0].is_none());
        let handle = app.world().resource::<GameData>().items.clone();
        let catalog =
            app.world_mut().resource_mut::<Assets<ItemCatalog>>().remove(&handle).unwrap();
        let body = app.world().get::<super::super::NpcBodies>(actor).unwrap()[0];
        app.world_mut().despawn(body);
        for _ in 0..3 {
            app.update();
            assert!(app.world().get::<Conversation>(session).is_some());
            assert!(app.world().get::<Closing>(session).is_some());
            assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 0);
        }
        app.world_mut().resource_mut::<Assets<ItemCatalog>>().insert(&handle, catalog).unwrap();
        app.update();
        assert!(app.world().get::<Conversation>(session).is_none());
        let inventory = app.world().get::<PartyInventory>(party).unwrap().clone();
        assert_eq!(inventory.gold, 50);
        assert_eq!(inventory.pool.stacks.iter().filter(|stack| stack.item == ItemId(1)).count(), 1);
        app.update();
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, inventory.gold);
        assert_eq!(
            app.world().get::<PartyInventory>(party).unwrap().pool.stacks,
            inventory.pool.stacks
        );
    }

    /// Removals empty the members' packs before the party's items, and the
    /// room they free counts for a later gift, both in preview and commit.
    #[test]
    fn repeated_removals_exhaust_a_pack_before_freeing_the_pool_for_a_gift() {
        let (mut app, session, _) = app(reward_script(
            vec![
                DialogueOp::TakeItem(ItemId(2)),
                DialogueOp::TakeItem(ItemId(2)),
                DialogueOp::GiveItem(ItemId(1)),
            ],
            1,
        ));
        with_items(&mut app);
        let member = carrier(&mut app, "Ragnar", 2, 1);
        let party = fill_pool(&mut app, crate::game::inventory::POOL_LIMIT - 1);
        let last = stack(&mut app, 2, 1);
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(last);
        app.update();
        let world = app.world();
        assert!(!matches!(
            world.get::<Conversation>(session).unwrap().wait,
            ConversationWait::Blocked { .. }
        ));
        assert!(world.get::<Carried>(member).unwrap().backpack[0].is_none());
        let pool = &world.get::<PartyInventory>(party).unwrap().pool.stacks;
        assert!(!pool.iter().any(|s| s.item == ItemId(2)));
        assert!(pool.iter().any(|s| s.item == ItemId(1)));
        assert_eq!(
            rewards(&app, session),
            [
                QuestReward::Taken { member: Some("Ragnar".into()) },
                QuestReward::Taken { member: None },
                QuestReward::Item { name: "Item 1".into() },
            ]
        );
    }

    /// Gold given to the NPC is spent and thanked for; more than the party
    /// has is refused without spending any.
    #[test]
    fn gold_given_to_an_npc_is_spent_and_thanked_for() {
        let mut quotes: Vec<_> =
            (0..0x11).map(|i| lines(&[format!("Line {i}").as_str()], vec![])).collect();
        quotes[0x10] = lines(&["Thank you"], vec![]);
        let (mut app, session, _) = app(DialogueScript { quotes, ..default() });
        let party = the_party(app.world_mut()).unwrap();
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 100;
        app.update();
        finish_line(&mut app, session);
        for (amount, left, refused) in [(150, 100, true), (40, 60, false)] {
            finish_line(&mut app, session);
            let revision = app.world().get::<Conversation>(session).unwrap().revision;
            app.world_mut().write_message(ConversationRequest::GiveGold {
                conversation: session,
                revision,
                amount,
            });
            app.update();
            let c = app.world().get::<Conversation>(session).unwrap();
            assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, left);
            if refused {
                assert_eq!(c.outcome(), Some(Outcome::GiftRefused(TradeError::Funds)));
            } else {
                assert_eq!(c.current, Some((ACCEPTED_GIFT_QUOTE, 0)));
            }
        }
        // All of it only once the party confirms.
        finish_line(&mut app, session);
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::GiveGold {
            conversation: session,
            revision,
            amount: 60,
        });
        app.update();
        let c = app.world().get::<Conversation>(session).unwrap();
        assert_eq!(c.wait, ConversationWait::GiveAll { gold: 60 });
        let revision = c.revision;
        app.world_mut().write_message(ConversationRequest::PriceAnswer {
            conversation: session,
            revision,
            accept: true,
        });
        app.update();
        assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 0);
    }

    /// The item stays with the party when the answer it would buy cannot run.
    #[test]
    fn an_offered_item_is_kept_when_its_answer_is_a_content_gap() {
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![
                Quote { text: vec!["Hello".into()], ..default() },
                Quote {
                    text: vec!["Thanks".into()],
                    operations: vec![
                        DialogueOp::ItemQuote { item: ItemId(1), keeps: true },
                        DialogueOp::Unsupported {
                            opcode: 13,
                            operands: [0; 3],
                            subentries: vec![],
                        },
                    ],
                    ..default()
                },
            ],
            ..default()
        });
        with_items(&mut app);
        let party = the_party(app.world_mut()).unwrap();
        let offered = stack(&mut app, 1, 1);
        let item = ItemRef { owner: ItemOwner::Pool, id: offered.id };
        app.world_mut().get_mut::<PartyInventory>(party).unwrap().pool.stacks.push(offered);
        app.update();
        finish_line(&mut app, session);
        let revision = app.world().get::<Conversation>(session).unwrap().revision;
        app.world_mut().write_message(ConversationRequest::Give {
            conversation: session,
            revision,
            item,
        });
        app.update();
        let world = app.world();
        assert!(matches!(
            world.get::<Conversation>(session).unwrap().wait,
            ConversationWait::Blocked { .. }
        ));
        assert_eq!(world.get::<PartyInventory>(party).unwrap().pool.stacks.len(), 1);
    }

    #[test]
    fn pending_experience_is_not_redirected_by_roster_changes() {
        use crate::game::characters::{Gender, Profession, Race};
        let (mut app, session, _) = app(DialogueScript {
            quotes: vec![Quote {
                text: vec!["First".into(), "Last".into()],
                operations: vec![DialogueOp::GiveExperience(100)],
                ..default()
            }],
            ..default()
        });
        let character = Character::new("Recipient", Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&character);
        let member = app.world_mut().spawn((character, vitals)).id();
        crate::game::party::roster::test_join(app.world_mut(), member);
        app.update();
        let speech = match app.world().get::<Conversation>(session).unwrap().wait {
            ConversationWait::Line { speech, .. } => speech,
            _ => panic!("first line expected"),
        };
        app.world_mut().entity_mut(member).remove::<PartyMember>();
        let replacement =
            Character::new("Replacement", Race::Human, Gender::Male, Profession::Fighter);
        let vitals = Vitals::new(&replacement);
        let replacement = app.world_mut().spawn((replacement, vitals)).id();
        crate::game::party::roster::test_join(app.world_mut(), replacement);
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        assert_eq!(app.world().get::<Character>(member).unwrap().experience, 100);
        assert_eq!(app.world().get::<Character>(replacement).unwrap().experience, 0);
    }

    #[test]
    fn notebook_edits_are_persistent_idempotent_and_session_bound() {
        let (mut app, session, _) = app(DialogueScript::default());
        app.world_mut().get_mut::<Conversation>(session).unwrap().queue.clear();
        app.world_mut()
            .get_mut::<Conversation>(session)
            .unwrap()
            .learned_topics
            .insert("arnika".into());
        {
            let mut facts = app.world_mut().resource_mut::<StoryFacts>();
            facts.remember_topic(TopicCategory::Place, "Arnika");
            facts.values.insert(FactId(91), 1);
            facts.journal.push(JournalEntry {
                fact: FactId(91),
                was_true: true,
                level: "home".into(),
            });
        }
        let edit = |world: &mut World, conversation, revision, edit, text: &str| {
            world.write_message(ConversationRequest::EditNotebook {
                conversation,
                revision,
                edit,
                text: text.into(),
            });
        };
        for text in [" Ship ", "ship", "   "] {
            edit(app.world_mut(), session, 0, NotebookEdit::Remember, text);
        }
        edit(app.world_mut(), session, 99, NotebookEdit::Forget, "Arnika");
        app.update();
        assert_eq!(
            app.world().resource::<StoryFacts>().topics,
            [(TopicCategory::Place, "Arnika".into()), (TopicCategory::Misc, "Ship".into()),]
        );
        for _ in 0..2 {
            edit(app.world_mut(), session, 0, NotebookEdit::Forget, " ARNIKA ");
        }
        app.update();
        let facts = app.world().resource::<StoryFacts>();
        assert_eq!(facts.topics, [(TopicCategory::Misc, "Ship".into())]);
        assert_eq!(facts.values.get(&FactId(91)), Some(&1));
        assert_eq!(facts.journal.len(), 1);
        assert!(
            app.world().get::<Conversation>(session).unwrap().learned_topics.contains("arnika")
        );
        // A new session sees the same notebook. Delayed edits from the old
        // session cannot change it, even with the same revision number.
        let mut reopened = app.world_mut().entity_mut(session).take::<Conversation>().unwrap();
        app.world_mut().despawn(session);
        reopened.learned_topics.clear();
        let next = app.world_mut().spawn(reopened).id();
        edit(app.world_mut(), session, 0, NotebookEdit::Forget, "ship");
        app.update();
        assert_eq!(app.world().resource::<StoryFacts>().topics.len(), 1);
        edit(app.world_mut(), next, 0, NotebookEdit::Forget, "SHIP");
        app.update();
        assert!(app.world().resource::<StoryFacts>().topics.is_empty());
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
        let body = app.world().get::<super::super::NpcBodies>(actor).unwrap()[0];
        use crate::game::world::{
            hostility::{SetStance, Stance},
            identify::{GroupMember, MonsterGroup},
        };
        app.init_resource::<Assets<crate::content::species::MonsterCatalog>>()
            .init_resource::<Factions>();
        let species = app
            .world_mut()
            .resource_mut::<Assets<crate::content::species::MonsterCatalog>>()
            .add(crate::content::species::MonsterCatalog(vec![default()]));
        app.world_mut().insert_resource(GameData {
            species,
            items: default(),
            loot: default(),
            spells: default(),
        });
        let group = app
            .world_mut()
            .spawn(MonsterGroup {
                set: Some(SetStance { stance: Stance::Hostile, at: 0 }),
                ..default()
            })
            .id();
        app.world_mut().entity_mut(body).insert(GroupMember(group));
        app.update();
        assert!(app.world().get::<Conversation>(session).is_none());
        assert_eq!(app.world().resource::<Lifecycle>().removed, 1);
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn authored_raven_reward_changes_xp_and_rapax_standing_and_urq_gives_gold() {
        use crate::game::characters::{Gender, Profession, Race};
        let files = crate::retail::files::retail_files();
        for (path, quote, expected_gold, expected_xp, expected_rapax) in [
            ("data/npc scripts/npc_guard_raven.nsf", 31, 0, 30000, 20),
            ("data/npc scripts/rpc_urq.nsf", 70, 500, 0, 25),
        ] {
            let script =
                crate::retail::npcs::read_dialogue(&files.read(path).unwrap().unwrap()).unwrap();
            let line_count = script.quotes[quote as usize].text.len();
            let (mut app, session, _) = app(script);
            app.init_resource::<Factions>();
            let world = app.world_mut();
            let party = the_party(world).unwrap();
            let profile =
                Character::new("Recipient", Race::Human, Gender::Male, Profession::Fighter);
            let vitals = Vitals::new(&profile);
            let member = world.spawn((profile, vitals)).id();
            crate::game::party::roster::test_join(world, member);
            world.get_mut::<Conversation>(session).unwrap().queue =
                VecDeque::from([Step::Quote { quote, subquote: 0, paid: true, plan: None }]);
            app.update();
            for _ in 1..line_count {
                let speech = match app.world().get::<Conversation>(session).unwrap().wait {
                    ConversationWait::Line { speech, .. } => speech,
                    _ => panic!("retail reward line expected"),
                };
                app.world_mut()
                    .write_message(ConversationRequest::Advance { conversation: session, speech });
                app.update();
            }
            let world = app.world();
            assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, expected_gold);
            assert_eq!(world.get::<Character>(member).unwrap().experience, expected_xp);
            assert_eq!(
                world.resource::<Factions>().score(Faction::RapaxCommon as i32),
                Some(expected_rapax)
            );
            assert!(!matches!(
                world.get::<Conversation>(session).unwrap().wait,
                ConversationWait::Blocked { .. }
            ));
        }
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
                typed: false,
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
    fn only_its_own_advance_moves_a_line_on_and_closure_keeps_npc() {
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
        // Time alone never moves the script on, nor another line's advance.
        let unrelated = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(ConversationRequest::Advance {
            conversation: session,
            speech: unrelated,
        });
        for _ in 0..600 {
            app.update();
        }
        let c = app.world().get::<Conversation>(session).unwrap();
        assert_eq!(c.current, Some((0, 0)));
        assert_eq!(c.wait, ConversationWait::Line { speech });
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((1, 0)));
        // A repeated advance of the line already passed is stale.
        app.world_mut()
            .write_message(ConversationRequest::Advance { conversation: session, speech });
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((1, 0)));
        assert!(matches!(
            app.world().get::<Conversation>(session).unwrap().wait,
            ConversationWait::Line { speech: next } if next != speech
        ));
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

#[cfg(test)]
mod view_lifecycle_test {
    use super::*;

    #[test]
    fn added_sessions_build_once_and_idle_transcripts_keep_their_labels() {
        crate::presentation::conversation::tests::check_session_views(|actor, script| {
            Conversation {
                actor,
                script,
                revision: 0,
                current: None,
                wait: ConversationWait::Ready,
                transcript: vec![],
                learned_topics: default(),
                menu: false,
                charms: default(),
                result: None,
                queue: default(),
            }
        });
    }
}
