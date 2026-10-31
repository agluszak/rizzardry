//! Authored continuation and waits belong to gameplay, not the dialogue view.
use super::{
    Npc,
    trade::{self, TradeError, TradeQuote, TradeSource},
};
use crate::content::factions::Faction;
use crate::content::{items::ItemId, npcs::*};
use crate::game::data::Catalogs;
use crate::game::factions::{Band, Factions};
use crate::game::time::GameClock;
use crate::game::{
    characters::{Character, Condition, Vitals},
    inventory::{Carried, ItemIds, ItemOwner, ItemRef, PartyInventory, make_item},
    party::{
        body::PartyBody,
        roster::{Party, PartyMember, PartyRoster},
    },
    rng::GameRng,
    world::{
        identify::{GroupMember, MonsterGroup},
        level::{Destination, Level, LevelOwned},
        population::Monster,
    },
};
use avian3d::prelude::Position;
use bevy::prelude::*;
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
    Close,
    Recruit,
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
/// Retail `GetNpcDispositionBand`: hostile under 0x21, friendly from 0x42.
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
fn entry_quote(def: &NpcDefinition, npc: &Npc) -> u16 {
    if def.merchant || npc.greeting_pending { 0 } else { 1 }
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
    Gold(u32),
    Experience { members: Vec<Entity>, amount: u32 },
    Faction { faction: Faction, delta: i8 },
    GiveItem(ItemId),
    TakeItem(ItemId),
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
/// The script's quotes answering a request to join (retail
/// `RequestNpcJoinParty`): it will not, the party has no room, it will.
const DECLINES_TO_JOIN: u16 = 0xb;
const NO_ROOM_TO_JOIN: u16 = 0xc;
const JOINS: u16 = 0xd;
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
/// An NPC is won over by charm or a bribe.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WonOver {
    pub conversation: Entity,
    pub greeting: bool,
}
/// What a friendly NPC says to an item it has no quote for: it takes it
/// gladly, or turns down one the party may not give away.
const ACCEPTED_GIFT_QUOTE: u16 = 0x10;
const REFUSED_GIFT_QUOTE: u16 = 0x11;

/// What the dialogue interpreter reads and changes: the conversations and
/// their speakers, the speakers' bodies, the party and its members, the
/// campaign's facts and factions, and the scripts.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Dialogue<'w, 's> {
    conversations: Query<'w, 's, (Entity, &'static mut Conversation, Has<Closing>)>,
    npcs: Query<
        'w,
        's,
        (
            Entity,
            &'static mut Npc,
            &'static super::ActorId,
            Option<&'static super::NpcBodies>,
            Has<PartyMember>,
        ),
    >,
    bodies: Query<
        'w,
        's,
        (
            &'static Monster,
            &'static LevelOwned,
            Option<&'static GroupMember>,
            Has<crate::game::combat::Combatant>,
            &'static mut Transform,
            Option<&'static Position>,
            Option<&'static mut avian3d::prelude::Rotation>,
        ),
    >,
    groups: Query<'w, 's, &'static MonsterGroup>,
    levels: Query<'w, 's, (Entity, &'static Level)>,
    party: Single<'w, 's, (&'static mut PartyInventory, &'static PartyRoster), With<Party>>,
    eye: Single<'w, 's, &'static Position, With<PartyBody>>,
    members: Query<'w, 's, (&'static mut Character, &'static Vitals, &'static mut Carried)>,
    story: ResMut<'w, StoryFacts>,
    content: If<Res<'w, Catalogs>>,
    factions: ResMut<'w, Factions>,
    rng: ResMut<'w, GameRng>,
    ids: ResMut<'w, ItemIds>,
    clock: Res<'w, GameClock>,
    turn: ResMut<'w, NoAnswerTurn>,
    scripts: Res<'w, Assets<DialogueScript>>,
    assets: Res<'w, super::NpcAssets>,
    freedom: crate::game::freedom::Freedom<'w, 's>,
    entering: Query<'w, 's, (), With<Destination>>,
    battles: Query<'w, 's, (), With<crate::game::combat::Battle>>,
    camp: Option<Res<'w, crate::game::camp::Camp>>,
    commands: Commands<'w, 's>,
    remarks: MessageWriter<'w, PartyRemark>,
    speech: MessageWriter<'w, NpcSpeech>,
    faction_news: MessageWriter<'w, crate::game::factions::FactionNews>,
    item_news: MessageWriter<'w, crate::game::inventory::ItemNews>,
    joins: MessageWriter<'w, super::recruit::JoinParty>,
    /// Conversations the party has left or that ended in this run, whose
    /// removal is not yet applied.
    left: Local<'s, Vec<Entity>>,
}

impl Dialogue<'_, '_> {
    fn conversation(&self, entity: Entity) -> &Conversation {
        self.conversations.get(entity).unwrap().1
    }

    fn conversation_mut(&mut self, entity: Entity) -> Mut<'_, Conversation> {
        self.conversations.get_mut(entity).unwrap().1
    }

    fn npc(&self, actor: Entity) -> &Npc {
        self.npcs.get(actor).unwrap().1
    }

    fn npc_mut(&mut self, actor: Entity) -> Mut<'_, Npc> {
        self.npcs.get_mut(actor).unwrap().1
    }

    fn definition(&self, actor: Entity) -> &NpcDefinition {
        &self.content.npcs[self.npcs.get(actor).unwrap().2.npc()]
    }

    /// The speaker's script, which loaded before the conversation began.
    fn script(&self, actor: Entity) -> &DialogueScript {
        let id = self.npcs.get(actor).unwrap().2.npc();
        self.scripts.get(&self.assets.scripts[&id]).unwrap()
    }

    fn lineup(&self) -> crate::game::party::Lineup<'_> {
        crate::game::party::Lineup::of(self.party.1, |member| {
            let (character, vitals, _) = self.members.get(member).unwrap();
            (character, vitals)
        })
    }

    /// The member with the best Communication, the first on a tie, and that
    /// level (retail `GetBestPartySkillLevel`).
    fn best_communicator(&self) -> Option<(Entity, u32)> {
        self.lineup().best(crate::content::characters::Skill::Communication)
    }

    fn communication(&self) -> u32 {
        self.best_communicator().map_or(0, |(_, level)| level)
    }

    /// A fact by its symbol, for rules that name one.
    pub fn fact_named(&self, symbol: &str) -> Result<u8, String> {
        let facts = &self.content.facts;
        let fact = facts.0.iter().find(|fact| fact.symbol == symbol);
        let id = fact.ok_or_else(|| format!("missing fact {symbol}"))?.id;
        self.story.get(id, facts)
    }

    /// The NPC's body in the played level, alive.
    fn active_body(&self, actor: Entity) -> Option<Entity> {
        let (.., bodies, _) = self.npcs.get(actor).ok()?;
        bodies?.iter().find(|&body| {
            self.bodies.get(body).is_ok_and(|(monster, owner, ..)| {
                self.levels.contains(owner.0) && !monster.is_dead()
            })
        })
    }

    /// How the NPC stands toward the party now (retail `GetNpcDisposition`):
    /// its own disposition, fully friendly while its body is turned. A
    /// factioned NPC answers with its faction's score when bound to its
    /// monster or when the faction is hostile to the party. Charm is scoped
    /// to this speaker in the open conversation; it never overwrites
    /// retained opinion. Allied party NPCs are not yet represented.
    fn disposition(&self, actor: Entity) -> i32 {
        self.standing(actor, self.npc(actor).disposition)
    }

    /// [`Self::disposition`] were the NPC's own `disposition` this.
    fn standing(&self, actor: Entity, disposition: u8) -> i32 {
        let turned = self.active_body(actor).is_some_and(|body| {
            self.bodies.get(body).unwrap().0.conditions.has(Condition::Turncoat)
        });
        let charm = self
            .conversations
            .iter()
            .find_map(|(_, c, _)| c.charms.get(&actor).copied())
            .unwrap_or(0);
        let own = if turned { 100 } else { i32::from(disposition.wrapping_add(charm as u8)) };
        let def = self.definition(actor);
        let faction = i32::from(def.faction);
        if def.faction != 0 && (def.monster_bound || self.factions.band(faction) == Band::Hostile) {
            return self.factions.score(faction).map_or(own, i32::from);
        }
        own
    }

    fn band(&self, actor: Entity) -> Band {
        band_of(self.disposition(actor))
    }

    /// Retail `SetNpcDispositionBand` to hostile.
    fn turn_hostile(&mut self, actor: Entity) {
        if self.band(actor) != Band::Hostile {
            self.npc_mut(actor).disposition = 0x19;
        }
    }

    /// Retail `ResumeNpc` as a dialogue closes on the party: an NPC that is
    /// not already hostile becomes so. Retail's NPC then fights with its
    /// group; NPCs do not fight here yet, so it only turns the party away
    /// from now on.
    fn resume(&mut self, actor: Entity) {
        if self.disposition(actor) > 0x20 {
            self.npc_mut(actor).disposition = 0x19;
        }
    }

    /// Practise `members`' Communication `usage` times.
    fn practise_communication(&mut self, members: &[Entity], usage: u32) {
        for &member in members {
            self.members.get_mut(member).unwrap().0.practise(
                crate::content::characters::Skill::Communication,
                usage,
                &mut self.rng,
            );
        }
    }

    /// The NPC stands in the played level, alive and not hostile.
    fn available(&self, actor: Entity) -> bool {
        self.active_body(actor).is_some_and(|body| {
            let (monster, _, group, fighting, ..) = self.bodies.get(body).unwrap();
            let set = group.and_then(|group| self.groups.get(group.0).ok()).and_then(|g| g.set);
            let species = &self.content.species[monster.species];
            crate::game::world::hostility::monster_own(
                monster,
                set,
                species,
                &self.factions,
                self.clock.ms,
                fighting,
            ) != crate::game::world::hostility::Stance::Hostile
        })
    }

    /// The conversation goes on: its speaker is still there to talk and no
    /// fight has begun.
    fn still_open(&self, entity: Entity) -> bool {
        self.conversations.get(entity).is_ok_and(|(_, c, _)| self.available(c.actor))
            && self.battles.is_empty()
    }

    /// What the NPC says to open its dialogue; meeting it for the first
    /// time teaches the party's members Communication (retail
    /// `HandleNpcDialogueDeparture`).
    fn greet(&mut self, actor: Entity) -> u16 {
        let def = self.definition(actor);
        let greeting = entry_quote(def, self.npc(actor));
        if greeting == 0 && !def.monster_bound && !def.voice_script && !def.merchant {
            let members: Vec<_> = self.lineup().able().map(|(m, _)| m).collect();
            self.practise_communication(&members, 0xf);
        }
        greeting
    }

    /// The party begins talking to the NPC, if it is free, the NPC is
    /// within reach and willing to talk at all.
    pub fn begin(&mut self, actor: Entity) -> Result<Entity, TradeError> {
        if !self.freedom.free() || !self.battles.is_empty() || self.party.0.hand.held.is_some() {
            return Err(TradeError::Busy);
        }
        let body = self.active_body(actor).ok_or(TradeError::Unavailable)?;
        // Recheck spatial reach from authoritative actor/party poses, not a UI claim.
        let (_, _, _, _, transform, position, _) = self.bodies.get(body).unwrap();
        let at = position.map_or(transform.translation, |position| position.0);
        let party_at = PartyBody::centre(PartyBody::eye(&self.eye));
        if party_at.distance(at) > crate::game::world::items::REACH || !self.available(actor) {
            return Err(TradeError::Unavailable);
        }
        let def = self.definition(actor);
        // Retail `BeginNpcDialogueInternal` and `OpenNpcDialoguePanel`: an NPC
        // that speaks through its own script greets as it is; otherwise one
        // hostile to the party only turns it away, a neutral one offers the
        // topic menu, and a merchant always does business.
        let (scripted, merchant) = (def.monster_bound || def.voice_script, def.merchant);
        let band = self.band(actor);
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
        if !scripted && !merchant && band == Band::Hostile {
            return Ok(self.commands.spawn((conversation(quote(HOSTILE_QUOTE)), Closing)).id());
        }
        let menu = !scripted && !merchant && band == Band::Neutral;
        let greeting = if menu { NEUTRAL_GREETING } else { self.greet(actor) };
        let now = self.clock.ms;
        let average = self.lineup().average_level();
        let def = self.definition(actor);
        if def.owns_stock {
            let stock = &self.npc(actor).stock;
            let gated = stock.restock_at.is_some()
                && now.saturating_sub(stock.maintenance_at) >= crate::game::time::DAY_MS
                && def.stock.iter().any(|r| !r.persistent && r.item == super::trade::TEMPLAR_GATED);
            let templar = gated && self.fact_named("FACT_TEMPLAR").expect("the Templar fact") != 0;
            let (_, mut npc, id, ..) = self.npcs.get_mut(actor).unwrap();
            let content = &self.content;
            npc.stock.maintain(
                &content.npcs[id.npc()],
                &content.items,
                average,
                now,
                templar,
                &mut self.rng,
            );
        }
        // OpenNpcDialoguePanel / MonsterForwardReferencePosition: the accepted
        // speaker faces the party. Physics rotation owns a moving body's facing.
        let (_, _, _, _, mut transform, position, rotation) = self.bodies.get_mut(body).unwrap();
        let here = position.map_or(transform.translation, |position| position.0);
        if let Some(facing) = crate::game::world::facing(here, party_at) {
            if let Some(mut rotation) = rotation {
                rotation.0 = facing;
            }
            transform.rotation = facing;
        }
        Ok(self.commands.spawn(Conversation { menu, ..conversation(quote(greeting)) }).id())
    }

    /// Travel, battle or the speaker's absence ends a conversation at once.
    /// Retail's message queue would still play the answers already under
    /// way, a paid answer or an accepted hand-in among them, so their effects
    /// commit as it ends; only their lines are lost.
    fn close(&mut self, entity: Entity) {
        let Ok((_, mut c, _)) = self.conversations.get_mut(entity) else { return };
        let plans: Vec<_> = c
            .queue
            .drain(..)
            .filter_map(|step| match step {
                Step::Quote { plan: Some(plan), .. } => Some(plan),
                _ => None,
            })
            .collect();
        for plan in plans {
            self.commit_effects(entity, plan.effects);
        }
        self.commands.entity(entity).despawn();
        self.left.push(entity);
    }

    /// The party leaves the dialogue: at once, or once its queued lines
    /// have played out.
    fn leave(&mut self, entity: Entity) {
        let c = self.conversation(entity);
        let pending = matches!(c.wait, ConversationWait::Line { .. })
            || c.queue.iter().any(|step| matches!(step, Step::Quote { .. }));
        if pending {
            self.commands.entity(entity).insert(Closing);
        } else {
            self.commands.entity(entity).despawn();
        }
        self.left.push(entity);
    }

    /// The NPC a script names by its display name (retail
    /// `FindNpcStateByName`).
    fn npc_named(&self, name: &str) -> Option<Entity> {
        let fixed = self.fact_named("FACT_RFS81_HAS_BEEN_FIXED").is_ok_and(|value| value != 0);
        self.npcs.iter().find_map(|(entity, _, id, ..)| {
            let id = id.npc();
            let shown = if id == RFS81 && fixed {
                RFS81_FIXED_NAME
            } else {
                self.content.npcs[id].display_name.as_str()
            };
            shown.eq_ignore_ascii_case(name).then_some(entity)
        })
    }

    /// Another NPC takes over the conversation and says `quote` of its own
    /// script (retail `ProcessNpcQuoteEntry` for a response naming a
    /// speaker). Retail finds the NPC by display name among those it knows;
    /// when none answers to it, the conversation simply waits for the party.
    fn hand_over(&mut self, entity: Entity, name: &str, quote: u16) {
        let Some(speaker) = self.npc_named(name) else { return };
        let mut c = self.conversation_mut(entity);
        c.actor = speaker;
        c.queue.push_front(Step::quote(quote));
    }

    fn plan_quote(&self, q: &Quote) -> Result<QuotePlan, (usize, String)> {
        let catalog = &self.content.facts;
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
                DialogueOp::GiveGold(amount) => plan.effects.push(QuoteEffect::Gold(*amount)),
                DialogueOp::GiveExperience(amount) => {
                    // CanPartySlotParticipate includes the unconscious. Quest
                    // XP is the authored amount for EACH eligible member.
                    let members = self
                        .party
                        .1
                        .iter()
                        .filter(|&member| !self.members.get(member).unwrap().1.is_dead())
                        .collect();
                    plan.effects.push(QuoteEffect::Experience { members, amount: *amount });
                }
                DialogueOp::AdjustFaction { faction: Some(faction), delta } => {
                    plan.effects.push(QuoteEffect::Faction { faction: *faction, delta: *delta });
                }
                // Retail ignores a faction name it does not know.
                DialogueOp::AdjustFaction { faction: None, .. } => {}
                DialogueOp::GiveItem(item) => plan.effects.push(QuoteEffect::GiveItem(*item)),
                DialogueOp::TakeItem(item) => plan.effects.push(QuoteEffect::TakeItem(*item)),
                DialogueOp::Conditional { facts, response } => {
                    let mut matched = true;
                    for (id, expected) in facts {
                        let actual = match writes.get(id) {
                            Some(value) => *value,
                            None => self.story.get(*id, catalog).map_err(|e| (i, e))?,
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
                    plan.following =
                        Some(Continuation::Options { words: words.clone(), typed: false });
                    break;
                }
                DialogueOp::KeywordInput(words) => {
                    plan.following =
                        Some(Continuation::Options { words: words.clone(), typed: true });
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
                    if let Some(npc) = self.npc_named(npc) {
                        plan.effects.push(QuoteEffect::Disposition { npc, delta: *delta });
                    }
                }
                DialogueOp::PartyRemark(event) => {
                    plan.effects.push(QuoteEffect::PartyRemark(*event))
                }
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

    /// Retail `AddItemToPartyOrDrop`: a gift the party's items have no room
    /// for lands at the party's feet, unless the party may not part with it;
    /// that one goes to the hand with retail's notice
    /// (`DropUnstoredCharacterItem`). Retail then puts back whatever the
    /// hand held, so it keeps only the first such gift.
    fn drop_gift(&mut self, gift: crate::game::inventory::ItemStack, crucial: bool) {
        use crate::game::world::items;
        match self.levels.single() {
            Ok((level, _)) if !crucial => {
                let feet = PartyBody::feet(&self.eye);
                items::lay(&mut self.commands, level, feet, items::WorldItem::loose(gift, false));
            }
            _ => {
                let hand = &mut self.party.0.hand;
                if hand.held.is_none() {
                    hand.held = Some(gift);
                }
                self.item_news.write(crate::game::inventory::ItemNews::CannotDrop);
            }
        }
    }

    /// One of the party's own items, in the pool or a member's pack.
    fn sale_item(&self, source: ItemRef) -> Result<&crate::game::inventory::ItemStack, TradeError> {
        match source.owner {
            ItemOwner::Pool => self.party.0.pool.stacks.iter().find(|i| i.id == source.id),
            ItemOwner::Member(member) => {
                self.members.get(member).unwrap().2.backpack_stack(source.id)
            }
        }
        .ok_or(TradeError::StaleSelection)
    }

    /// Commit a removal whose ownership and quantity were resolved in this
    /// operation.
    fn consume_item(&mut self, source: ItemRef, units: u16) {
        match source.owner {
            ItemOwner::Pool => self.party.0.pool.consume_units(source.id, units),
            ItemOwner::Member(member) => {
                self.members.get_mut(member).unwrap().2.consume_backpack_units(source.id, units)
            }
        }
    }

    fn reward(&mut self, entity: Entity, reward: QuestReward) {
        self.conversation_mut(entity).transcript.push(TranscriptLine::Reward(reward));
    }

    fn commit_quote(&mut self, entity: Entity, quote: u16, plan: QuotePlan) {
        self.commit_effects(entity, plan.effects);
        let Some(following) = plan.following else { return };
        let next = match following {
            Continuation::Quote(Response::Speaker { npc, quote }) => {
                return self.hand_over(entity, &npc, quote);
            }
            Continuation::Quote(response) => Step::Quote {
                quote: select_response(&response, &mut self.rng)
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
        self.conversation_mut(entity).queue.push_front(next);
    }

    fn commit_effects(&mut self, entity: Entity, effects: Vec<QuoteEffect>) {
        for effect in effects {
            match effect {
                QuoteEffect::SetFact(id, value) => {
                    let level =
                        self.levels.single().map_or_else(|_| String::new(), |l| l.1.path.clone());
                    self.story
                        .set(id, value, &self.content.facts, level)
                        .expect("planned fact assignment");
                }
                QuoteEffect::Mention(category, text) => self.learn_topic(entity, category, &text),
                QuoteEffect::Gold(amount) => {
                    let purse = &mut self.party.0.gold;
                    *purse = purse.wrapping_add(amount);
                    let actor = self.conversation(entity).actor;
                    let giver = self.definition(actor).source_name.clone();
                    self.reward(entity, QuestReward::Gold { giver, amount });
                }
                QuoteEffect::Experience { members, amount } => {
                    for member in members {
                        self.members.get_mut(member).unwrap().0.award_experience(amount);
                    }
                    self.reward(entity, QuestReward::Experience { amount_each: amount });
                }
                // `executable` checked the content just before.
                QuoteEffect::GiveItem(item) => {
                    let catalog = &self.content.items;
                    let definition = &catalog[item];
                    let stack = make_item(definition, &mut self.ids, &mut self.rng, true, true);
                    let rest = self.party.0.pool.receive(stack, catalog);
                    let (name, crucial) = (definition.name.clone(), definition.crucial);
                    if let Some(rest) = rest {
                        self.drop_gift(rest, crucial);
                    }
                    self.reward(entity, QuestReward::Item { name });
                }
                QuoteEffect::TakeItem(item) => {
                    let source = self
                        .party
                        .1
                        .iter()
                        .find_map(|member| {
                            let pack = self.members.get(member).unwrap().2;
                            let stack = pack.backpack.iter().flatten().find(|s| s.item == item)?;
                            Some(ItemRef { owner: ItemOwner::Member(member), id: stack.id })
                        })
                        .or_else(|| {
                            let pool = &self.party.0.pool;
                            let stack = pool.stacks.iter().find(|s| s.item == item)?;
                            Some(ItemRef { owner: ItemOwner::Pool, id: stack.id })
                        });
                    if let Some(source) = source {
                        self.consume_item(source, 1);
                        let member = match source.owner {
                            ItemOwner::Member(member) => {
                                Some(self.members.get(member).unwrap().0.name.clone())
                            }
                            ItemOwner::Pool => None,
                        };
                        self.reward(entity, QuestReward::Taken { member });
                    }
                }
                QuoteEffect::Disposition { npc, delta } => {
                    shift_disposition(&mut self.npc_mut(npc), delta)
                }
                QuoteEffect::Resume => {
                    let actor = self.conversation(entity).actor;
                    self.resume(actor);
                }
                QuoteEffect::PartyRemark(event) => {
                    self.remarks.write(PartyRemark { event });
                }
                QuoteEffect::Faction { faction, delta } => {
                    // Retail announces it in the text box, not the dialogue.
                    if let Some(news) = self.factions.adjust(faction, delta, self.clock.ms) {
                        self.faction_news.write(news);
                    }
                }
            }
        }
    }

    /// Run the conversation's script as far as it goes without the party.
    fn run(&mut self, entity: Entity) {
        let c = self.conversation(entity);
        if c.wait != ConversationWait::Ready || c.queue.is_empty() {
            return;
        }
        let actor = c.actor;
        // Bound immediate authored transitions, including malicious empty-quote loops.
        for _ in 0..256 {
            let c = self.conversation(entity);
            // A new speaker's script is fetched on the next run.
            if c.wait != ConversationWait::Ready || c.actor != actor {
                return;
            }
            let Some(step) = self.conversation_mut(entity).queue.pop_front() else { return };
            match step {
                Step::End => {
                    self.conversation_mut(entity).queue.push_front(Step::End);
                    return;
                }
                // The NPC joins the party once its acceptance is said.
                Step::Join => {
                    self.joins.write(super::recruit::JoinParty { actor });
                }
                Step::Wait(wait) => {
                    self.conversation_mut(entity).wait = wait;
                    return;
                }
                Step::Quote { quote, subquote, paid, plan } => {
                    let q = self.script(actor).quotes[quote as usize].clone();
                    if !paid
                        && subquote == 0
                        && let Some((gold, always)) = q.operations.iter().find_map(|op| {
                            if let DialogueOp::Price { gold, always } = op {
                                Some((*gold, *always))
                            } else {
                                None
                            }
                        })
                        && (always || !self.npc(actor).paid_topics.contains(&quote))
                    {
                        let charm = self.conversation(entity).charm().max(0) as u32;
                        let gold = gold - gold.saturating_mul(charm) / 100;
                        let gold = (gold - gold.saturating_mul(self.communication()) / 500).max(1);
                        let mut c = self.conversation_mut(entity);
                        c.queue.push_front(Step::Wait(ConversationWait::Price {
                            quote,
                            gold,
                            always,
                        }));
                        c.queue.push_front(Step::said(PRICE_QUOTE));
                        continue;
                    }
                    let plan = match plan.map_or_else(|| self.plan_quote(&q), Ok) {
                        Ok(plan) => plan,
                        Err((entry, reason)) => {
                            let mut c = self.conversation_mut(entity);
                            c.queue.push_front(Step::Quote { quote, subquote, paid, plan: None });
                            block(&mut c, quote, Some(entry), reason);
                            return;
                        }
                    };
                    if quote == 0 {
                        if self.npc(actor).greeting_pending {
                            let name = self.definition(actor).source_name.clone();
                            self.learn_topic(entity, TopicCategory::Person, &name);
                        }
                        self.npc_mut(actor).greeting_pending = false;
                    }
                    self.conversation_mut(entity).current = Some((quote, subquote));
                    if subquote + 1 >= q.text.len() {
                        self.commit_quote(entity, quote, plan);
                    } else {
                        self.conversation_mut(entity).queue.push_front(Step::Quote {
                            quote,
                            subquote: subquote + 1,
                            paid: true,
                            plan: Some(plan),
                        })
                    }
                    if self.say(entity, actor, (&q, quote, subquote)) {
                        return;
                    }
                }
            }
        }
        let mut c = self.conversation_mut(entity);
        let quote = c.current.map_or(0, |v| v.0);
        block(&mut c, quote, None, "script made no bounded progress".into());
    }

    /// Retail `SpeakNpcSubquote`: the speaker says one line of the quote,
    /// and the conversation waits for it to be said. An alert line is shown
    /// with the alert chime and no recording; otherwise a few words stand
    /// for something other than a line, and nothing is said.
    fn say(
        &mut self,
        entity: Entity,
        actor: Entity,
        (q, quote, subquote): (&Quote, u16, usize),
    ) -> bool {
        let text = q.text.get(subquote).cloned().unwrap_or_default();
        let def = self.definition(actor);
        let (speaker, stem) = (def.source_name.clone(), def.voice_stem());
        let is = |word: &str| text.eq_ignore_ascii_case(word);
        let voice = if q.alert {
            None
        } else if text.is_empty() {
            return false;
        } else if is("EMPTY") || is("BLANK") {
            // The answer to a question it has nothing to say to.
            if NO_ANSWER_QUOTES.contains(&quote) {
                let second = self.turn.0;
                self.turn.0 = !second;
                let line = TranscriptLine::NoAnswer { npc: speaker, second };
                self.conversation_mut(entity).transcript.push(line);
            }
            return false;
        } else if is("UNKNOWN") || is("CLASSIFIED") {
            let next = if is("CLASSIFIED") {
                CLASSIFIED_QUOTE
            } else if self.rng.below(100) < 50 {
                UNKNOWN_QUOTES[0]
            } else {
                UNKNOWN_QUOTES[1]
            };
            self.conversation_mut(entity).queue.push_back(Step::quote(next));
            return false;
        } else {
            let suffix = if subquote == 0 {
                String::new()
            } else {
                char::from(b'a' + (subquote - 1) as u8).to_string()
            };
            Some(format!("{stem}_{quote:03}{suffix}"))
        };
        // "SOUND" plays its recording without showing a line.
        let text = if is("SOUND") && !q.alert { String::new() } else { text };
        let speech = self
            .commands
            .spawn((
                ConversationSpeech { voice, text: text.clone(), alert: q.alert },
                ChildOf(entity),
            ))
            .id();
        if self.conversations.get(entity).unwrap().2 && !text.is_empty() {
            self.speech.write(NpcSpeech { speaker, text: text.clone() });
        }
        // The line being spoken is always the transcript's last; one with no
        // text is not shown.
        let mut c = self.conversation_mut(entity);
        c.transcript.push(TranscriptLine::Npc(text));
        c.wait = ConversationWait::Line { speech };
        true
    }

    fn learn_topic(&mut self, entity: Entity, category: TopicCategory, text: &str) {
        if self.story.remember_topic(category, text) {
            let topic = text.trim().to_ascii_lowercase();
            self.conversation_mut(entity).learned_topics.insert(topic);
        }
    }

    /// Whether the NPC would join the party now, and if not, why (retail
    /// `RequestNpcJoinParty`'s order: an NPC without a character declines
    /// before the party's room is counted).
    pub fn joinable(&self, actor: Entity) -> Result<(), super::recruit::JoinRefusal> {
        use super::recruit::{JoinRefusal, NPC_PLACES};
        let (_, npc, id, _, member) = self.npcs.get(actor).unwrap();
        let definition = &self.content.npcs[id.npc()];
        if definition.character.is_none() || npc.dead || member {
            return Err(JoinRefusal::Declines);
        }
        let npcs = self.party.1.iter().filter(|&m| self.npcs.contains(m)).count();
        if npcs >= NPC_PLACES {
            return Err(JoinRefusal::NoRoom);
        }
        let (_, level) = self.levels.single().unwrap();
        let region =
            crate::content::levels::LevelInfo::for_path(&level.path).map_or(0, |info| info.band);
        // Retail counts no able member as an average of nothing.
        let levels: Vec<_> = self.lineup().able().map(|(_, c)| c.level()).collect();
        let average = if levels.is_empty() { 0 } else { crate::game::party::average_level(levels) };
        let fact = |symbol: &str| self.fact_named(symbol).expect("a ported fact");
        if !super::recruit::willing(id.npc(), definition, region, fact, average) {
            return Err(JoinRefusal::Declines);
        }
        Ok(())
    }

    /// The party asks the NPC to join. It answers from its script, and
    /// joining, leaves the conversation.
    fn ask_to_join(&mut self, entity: Entity) {
        let c = self.conversation(entity);
        if !c.ready() {
            return;
        }
        let answer = self.joinable(c.actor);
        let quote = match answer {
            Ok(()) => JOINS,
            Err(super::recruit::JoinRefusal::NoRoom) => NO_ROOM_TO_JOIN,
            Err(super::recruit::JoinRefusal::Declines) => DECLINES_TO_JOIN,
        };
        let mut c = self.conversation_mut(entity);
        c.queue.push_back(Step::quote(quote));
        if answer.is_ok() {
            c.queue.extend([Step::Join, Step::End]);
        }
    }

    fn topic(&mut self, entity: Entity, text: &str) {
        let c = self.conversation(entity);
        if !c.ready() || c.menu {
            return;
        }
        let script = self.script(c.actor);
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
            self.story.remember_topic(TopicCategory::Misc, &cleaned);
            let mut c = self.conversation_mut(entity);
            c.transcript.push(TranscriptLine::Question(text.to_owned()));
            c.queue.push_back(Step::quote(quote));
        } else {
            self.conversation_mut(entity).report(Outcome::NoTopic);
        }
    }

    /// The party charms the NPC or makes small talk with it from the topic
    /// menu (retail `SelectNpcDialogueService`, `SelectNpcDialogueTalk` and
    /// `ApplyNpcInteraction` 1 and 0). Its disposition moves by its record's
    /// scale, more with the party's best Communication, which practises; a
    /// charm that warms it moves it further the higher the party's level is
    /// over its own. The band it ends in answers: won over, it turns to its
    /// dialogue; turned hostile, it sends the party away.
    fn approach(&mut self, entity: Entity, approach: Approach) {
        let c = self.conversation(entity);
        let actor = c.actor;
        let now = self.clock.ms;
        let npc = self.npc(actor);
        let at = match approach {
            Approach::Charm => npc.charmed_at,
            Approach::SmallTalk => npc.small_talk_at,
        };
        let def = self.definition(actor);
        let scale = i32::from(match approach {
            Approach::Charm => def.charm_scale,
            Approach::SmallTalk => def.talk_scale,
        });
        if !c.ready() || !c.menu || !approach_ready(at, now) {
            return;
        }
        let Some((speaker, level)) = self.best_communicator() else { return };
        let mut delta = scale + level as i32 * (scale.abs() / 5) / 100;
        if approach == Approach::Charm && delta > 0 {
            let average =
                crate::game::party::average_level(self.lineup().able().map(|(_, c)| c.level()));
            let own = self
                .active_body(actor)
                .map(|body| &self.content.species[self.bodies.get(body).unwrap().0.species])
                .map_or(1, |kind| u32::from(kind.level).max(1));
            delta = (average * delta as u32 / own) as i32;
        }
        let mut npc = self.npc_mut(actor);
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
        self.practise_communication(&[speaker], usage);
        let answer = match (self.band(actor), approach) {
            (Band::Friendly, Approach::Charm) => CHARMED_QUOTE,
            (Band::Friendly, Approach::SmallTalk) => self.greet(actor),
            (Band::Neutral, Approach::Charm) => UNCHARMED_QUOTE,
            (Band::Neutral, Approach::SmallTalk) => SMALL_TALK_QUOTE,
            (Band::Hostile, _) => {
                self.turn_hostile(actor);
                HOSTILE_QUOTE
            }
        };
        let friendly = self.band(actor) == Band::Friendly;
        let mut c = self.conversation_mut(entity);
        c.queue.push_back(Step::quote(answer));
        if !friendly && answer == HOSTILE_QUOTE {
            c.queue.push_back(Step::End);
        }
        c.menu = !friendly;
    }

    /// Retail `DebitNpcTradePool`: a bribe, Communication-weighted, draws on
    /// what the NPC will take. Once that runs out an NPC not yet friendly
    /// warms to the party, and the pool fills again.
    fn debit_trade_pool(&mut self, actor: Entity, price: u32) {
        let (pool, disposition) = self.debited(actor, price);
        let mut npc = self.npc_mut(actor);
        (npc.trade_pool, npc.disposition) = (pool, disposition);
    }

    /// The NPC's trade pool and own disposition once a bribe of `price` is
    /// debited, without debiting it.
    fn debited(&self, actor: Entity, price: u32) -> (u16, u8) {
        let level = self.communication();
        let adjusted = price.wrapping_add(level * ((price & 0xffff) / 5) / 100);
        let refill = self.definition(actor).trade_pool;
        let cold = self.band(actor) != Band::Friendly;
        let npc = self.npc(actor);
        match npc.trade_pool.saturating_sub(adjusted as u16) {
            0 if cold => (refill, 0x4b),
            0 => (refill, npc.disposition),
            pool => (pool, npc.disposition),
        }
    }

    /// A bribe that wins the NPC over from the topic menu turns it to its
    /// dialogue; it greets the party unless it took all the party's gold
    /// (retail `HandleNpcDialogueItemChoice`, `OnNpcTradeDialogClosed`).
    fn won_over(&mut self, entity: Entity, greeting: bool) {
        let c = self.conversation(entity);
        let actor = c.actor;
        if !c.menu || self.band(actor) != Band::Friendly {
            return;
        }
        let greeting = greeting.then(|| self.greet(actor));
        let mut c = self.conversation_mut(entity);
        c.menu = false;
        if let Some(quote) = greeting {
            c.queue.push_back(Step::quote(quote));
        }
    }

    /// The party offers the NPC an item from a member's pack or the party's
    /// items (retail `HandleNpcDialogueItem`). The quote answering it runs;
    /// the NPC keeps what it asked for, and any other item it is given
    /// unless the party may not part with it. A voice-scripted NPC ignores
    /// an item it has nothing to say about. The item is taken only when its
    /// answer's effects are valid; a content gap keeps it.
    fn give(&mut self, entity: Entity, item: ItemRef) -> Result<(), TradeError> {
        let c = self.conversation(entity);
        if !c.ready() {
            return Err(TradeError::Busy);
        }
        let actor = c.actor;
        let stack = self.sale_item(item)?;
        let (offered, count) = (stack.item, stack.count());
        let definition_of = &self.content.items[offered];
        let (crucial, value) = (definition_of.crucial, stack_value(definition_of, count));
        let def = self.definition(actor);
        let (voiced, kind) = (def.voice_script, def.kind);
        let answer = self.script(actor).item_quote(offered);
        // Retail `HandleNpcDialogueItem`: an NPC not yet friendly takes an
        // item as a bribe, valued by the whole stack offered, and keeps it
        // whatever it says.
        let bribe = !voiced && self.band(actor) != Band::Friendly;
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
                let after = self.debited(actor, value);
                debit = Some(after);
                let warmed = band_of(self.standing(actor, after.1)) == Band::Friendly;
                (if warmed { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE }, true)
            }
            Some(answer) => answer,
            None if voiced => return Ok(()),
            None if crucial => (REFUSED_GIFT_QUOTE, false),
            None => (ACCEPTED_GIFT_QUOTE, true),
        };
        let plan = self
            .script(actor)
            .quotes
            .get(usize::from(quote))
            .and_then(|line| self.plan_quote(line).ok());
        // Retail takes the item at once; the answer's effects follow at its
        // last line, which plays out even if the party leaves the dialogue. A
        // content gap keeps the item and queues the answer to show it.
        if keeps && plan.is_some() {
            self.consume_item(item, 1);
            if let Some((pool, disposition)) = debit {
                let mut npc = self.npc_mut(actor);
                (npc.trade_pool, npc.disposition) = (pool, disposition);
            }
        }
        let step = Step::Quote { quote, subquote: 0, paid: false, plan };
        self.conversation_mut(entity).queue.push_back(step);
        self.won_over(entity, true);
        Ok(())
    }

    /// The party gives the NPC gold (retail `HandleNpcDialogueItem` with
    /// trade gold): all of it only once the party confirms, as retail asks.
    fn give_gold(&mut self, entity: Entity, amount: u32) -> Result<(), TradeError> {
        if !self.conversation(entity).ready() {
            return Err(TradeError::Busy);
        }
        let gold = self.party.0.gold;
        if amount == 0 {
            return Err(TradeError::InvalidQuantity);
        }
        if amount > gold {
            return Err(TradeError::Funds);
        }
        if amount == gold {
            self.conversation_mut(entity).wait = ConversationWait::GiveAll { gold };
            return Ok(());
        }
        self.spend_gift(entity, amount, false)
    }

    /// The NPC takes the gold (retail `ConfirmNpcTradePurchase`): a friendly
    /// one thanks the party for it; one not yet friendly takes it as a
    /// bribe, unless it takes no bribes, and says whether that won it over.
    fn spend_gift(&mut self, entity: Entity, amount: u32, all: bool) -> Result<(), TradeError> {
        let actor = self.conversation(entity).actor;
        let bribe = self.band(actor) != Band::Friendly;
        if bribe && self.definition(actor).trade_pool == 0 {
            self.conversation_mut(entity).queue.push_back(Step::said(UNBRIBED_QUOTE));
            return Ok(());
        }
        let purse = &mut self.party.0.gold;
        *purse = purse.checked_sub(amount).ok_or(TradeError::Funds)?;
        if bribe {
            self.debit_trade_pool(actor, amount);
        }
        let quote =
            if self.band(actor) == Band::Friendly { ACCEPTED_GIFT_QUOTE } else { UNBRIBED_QUOTE };
        self.conversation_mut(entity).queue.push_back(Step::said(quote));
        self.won_over(entity, !all);
        Ok(())
    }

    fn reply(&mut self, entity: Entity, text: &str) {
        let c = self.conversation(entity);
        let ConversationWait::Options { quote, words, .. } = &c.wait else { return };
        let matched = words.iter().any(|w| w.eq_ignore_ascii_case(text));
        let script = self.script(c.actor);
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
            let mut c = self.conversation_mut(entity);
            c.transcript.push(TranscriptLine::Reply(text.to_owned()));
            c.wait = ConversationWait::Ready;
            c.queue.push_back(Step::quote(quote));
        }
    }

    fn answer_price(&mut self, entity: Entity, accept: bool) {
        let c = self.conversation(entity);
        if let ConversationWait::GiveAll { gold } = c.wait {
            self.conversation_mut(entity).wait = ConversationWait::Ready;
            if accept && let Err(refused) = self.spend_gift(entity, gold, true) {
                self.conversation_mut(entity).report(Outcome::GiftRefused(refused));
            }
            return;
        }
        let ConversationWait::Price { quote, gold, always } = c.wait else { return };
        let actor = c.actor;
        let Some(line) = self.script(actor).quotes.get(usize::from(quote)).cloned() else {
            return;
        };
        if accept {
            let (next, plan) = if self.party.0.gold < gold {
                (TOO_POOR_QUOTE, None)
            } else {
                let plan = match self.plan_quote(&line) {
                    Ok(plan) => plan,
                    Err((entry, reason)) => {
                        block(&mut self.conversation_mut(entity), quote, Some(entry), reason);
                        return;
                    }
                };
                self.party.0.gold -= gold;
                if !always {
                    self.npc_mut(actor).paid_topics.push(quote);
                }
                (quote, Some(plan))
            };
            let step = Step::Quote { quote: next, subquote: 0, paid: true, plan };
            self.conversation_mut(entity).queue.push_back(step);
        } else {
            let declines: Vec<_> = line
                .operations
                .iter()
                .filter_map(|op| match op {
                    DialogueOp::Decline(quote) => Some(Step::quote(*quote)),
                    _ => None,
                })
                .collect();
            self.conversation_mut(entity).queue.extend(declines);
        }
        self.conversation_mut(entity).wait = ConversationWait::Ready;
    }

    /// Answer one of the party's requests, and report what came of it.
    fn answer(&mut self, entity: Entity, talk: Talk) {
        let outcome = match talk {
            Talk::Close => {
                self.leave(entity);
                None
            }
            Talk::Topic { text } => {
                self.topic(entity, &text);
                None
            }
            Talk::Reply { text } => {
                self.reply(entity, &text);
                None
            }
            Talk::PriceAnswer { accept } => {
                self.answer_price(entity, accept);
                None
            }
            Talk::Recruit => {
                self.ask_to_join(entity);
                None
            }
            Talk::Give { item } => self.give(entity, item).err().map(Outcome::GiftRefused),
            Talk::GiveGold { amount } => {
                self.give_gold(entity, amount).err().map(Outcome::GiftRefused)
            }
            Talk::Approach { approach } => {
                self.approach(entity, approach);
                None
            }
        };
        if let Some(outcome) = outcome {
            self.conversation_mut(entity).report(outcome);
        }
    }
}

/// [`Dialogue::begin`], for operations that hold the whole world.
pub fn begin(world: &mut World, actor: Entity) -> Result<Entity, TradeError> {
    world
        .run_system_cached_with(
            |In(actor): In<Entity>, mut dialogue: Dialogue| dialogue.begin(actor),
            actor,
        )
        .unwrap()
}

/// Travel ends the conversation; the speaker stays.
pub(crate) fn end_conversations(_: On<Add<Destination>>, mut dialogue: Dialogue) {
    let open: Vec<_> = dialogue.conversations.iter().map(|(entity, ..)| entity).collect();
    for conversation in open {
        dialogue.close(conversation);
    }
}

/// Where an NPC stands in the played level: the body of its that is
/// there and alive.
#[derive(bevy::ecs::system::SystemParam)]
pub struct NpcPresence<'w, 's> {
    bodies: Query<'w, 's, (&'static Monster, &'static crate::game::world::level::LevelOwned)>,
    levels: Query<'w, 's, (), With<Level>>,
}

impl NpcPresence<'_, '_> {
    pub fn body(&self, bodies: Option<&super::NpcBodies>) -> Option<Entity> {
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
        (&'static mut Npc, &'static super::ActorId, Option<&'static super::NpcBodies>),
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
/// The systems that carry conversations: lines spoken, conversations
/// begun, the party's requests answered, then scripts run.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Conversing;

/// The NPC has finished saying a line.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSpoken {
    pub conversation: Entity,
    pub speech: Entity,
}

/// The party remembers or forgets a topic in its notebook.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct EditNotebook {
    pub edit: NotebookEdit,
    pub text: String,
}

/// A spoken line gives way to what follows it.
pub fn finish_lines(
    mut spoken: MessageReader<LineSpoken>,
    mut conversations: Query<&mut Conversation>,
    mut commands: Commands,
) {
    for &LineSpoken { conversation, speech } in spoken.read() {
        let Ok(mut c) = conversations.get_mut(conversation) else { continue };
        if matches!(c.wait, ConversationWait::Line { speech: s, .. } if s == speech) {
            commands.entity(speech).despawn();
            c.wait = ConversationWait::Ready;
        }
    }
}

pub fn edit_notebook(mut edits: MessageReader<EditNotebook>, mut facts: ResMut<StoryFacts>) {
    for EditNotebook { edit, text } in edits.read() {
        match edit {
            NotebookEdit::Remember => facts.remember_topic(TopicCategory::Misc, text),
            NotebookEdit::Forget => facts.forget_topic(text),
        };
    }
}

/// Whether a conversation may go on: its NPC there to talk and no fight
/// under way.
/// Talking to an NPC begins a conversation. An accepted level entry has
/// ended every dialogue (`end_conversations`); no queued request may alter
/// the departing campaign.
pub fn begin_conversations(mut begins: MessageReader<BeginConversation>, mut dialogue: Dialogue) {
    if !dialogue.entering.is_empty() {
        begins.clear();
        return;
    }
    // A conversation begun holds the party until it ends.
    let mut begun = false;
    for &BeginConversation { actor } in begins.read() {
        match if begun { Err(TradeError::Busy) } else { dialogue.begin(actor) } {
            Ok(_) => begun = true,
            Err(e) => warn!("conversation refused: {e:?}"),
        }
    }
}

/// Answer the party's requests in a conversation. A dialogue the party has
/// left takes none; anything but leaving ends a dialogue whose NPC, a
/// fight or a camp no longer allows it.
pub fn answer_requests(mut requests: MessageReader<ConversationRequest>, mut dialogue: Dialogue) {
    dialogue.left.clear();
    if !dialogue.entering.is_empty() {
        requests.clear();
        return;
    }
    for ConversationRequest { conversation: entity, talk } in requests.read().cloned() {
        let Ok((_, _, closing)) = dialogue.conversations.get(entity) else { continue };
        if closing || dialogue.left.contains(&entity) {
            continue;
        }
        if !matches!(talk, Talk::Close) {
            if !dialogue.still_open(entity) || dialogue.camp.is_some() {
                dialogue.close(entity);
                continue;
            }
            dialogue.conversation_mut(entity).result = None;
        }
        dialogue.answer(entity, talk);
    }
}

/// An NPC won over by charm or a bribe turns from its topic menu to its
/// dialogue.
pub fn win_over(mut won: MessageReader<WonOver>, mut dialogue: Dialogue) {
    for &WonOver { conversation, greeting } in won.read() {
        if dialogue.conversations.contains(conversation) {
            dialogue.won_over(conversation, greeting);
        }
    }
}

/// Run each conversation's script as far as it goes, and close those that
/// are done: ended, left with nothing more to say, or no longer possible.
pub fn run_conversations(mut dialogue: Dialogue) {
    dialogue.left.clear();
    let sessions: Vec<_> = dialogue.conversations.iter().map(|(entity, ..)| entity).collect();
    for entity in sessions {
        if !dialogue.still_open(entity) {
            dialogue.close(entity);
            continue;
        }
        dialogue.run(entity);
        let (_, c, closing) = dialogue.conversations.get(entity).unwrap();
        let left = closing
            && match c.wait {
                ConversationWait::Ready => c.queue.is_empty(),
                ConversationWait::Line { .. } => false,
                _ => true,
            };
        if left || (matches!(c.queue.front(), Some(Step::End)) && c.wait == ConversationWait::Ready)
        {
            dialogue.close(entity);
        }
    }
}
