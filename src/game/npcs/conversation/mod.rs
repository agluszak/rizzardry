//! Conversation state and playback. Economic and membership transactions have other owners.
mod input;
mod playback;
mod rewards;
mod social;
use super::trade::{self, TradeError, TradeSource};
use crate::content::npcs::TopicCategory;
use bevy::prelude::*;
pub use input::quote_trade;
pub use social::approach_ready;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Conversing;
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Input;
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Playback;
#[derive(Component, Debug)]
pub struct Closing;
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationWait {
    Ready,
    Line { speech: Entity },
    Options { quote: u16, words: Vec<String>, typed: bool },
    Price { quote: u16, gold: u32, always: bool },
    GiveAll { gold: u32 },
    Blocked { quote: u16, entry: Option<usize>, reason: String },
}
#[derive(Clone, Debug, PartialEq)]
pub enum QuestReward {
    Gold { giver: String, amount: u32 },
    Experience { amount_each: u32 },
    Item { name: String },
    Taken { member: Option<String> },
}
#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptLine {
    Npc(String),
    Question(String),
    Reply(String),
    Reward(QuestReward),
    NoAnswer { npc: String, second: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NoTopic,
    Traded,
    TradeRefused(TradeError),
    GiftRefused(TradeError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    Charm,
    SmallTalk,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotebookEdit {
    Remember,
    Forget,
}
#[derive(Clone, Copy, Debug)]
pub struct TradeQuote {
    pub conversation: Entity,
    pub offer: trade::Quote,
}
#[derive(Clone, Debug)]
pub enum Talk {
    Topic { text: String },
    Reply { text: String },
    PriceAnswer { accept: bool },
    Close,
    Recruit,
    Trade(TradeQuote),
    Give { item: Entity },
    GiveGold { amount: u32 },
    Approach { approach: Approach },
}
#[derive(Message, Clone, Copy, Debug)]
pub struct BeginConversation {
    pub actor: Entity,
}
#[derive(Message, Clone, Debug)]
pub struct ConversationRequest {
    pub conversation: Entity,
    pub talk: Talk,
}
impl ConversationRequest {
    pub fn new(conversation: Entity, talk: Talk) -> Self {
        Self { conversation, talk }
    }
    pub fn trade(quote: TradeQuote) -> Self {
        Self::new(quote.conversation, Talk::Trade(quote))
    }
}
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSpoken {
    pub conversation: Entity,
    pub speech: Entity,
}
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct EditNotebook {
    pub edit: NotebookEdit,
    pub text: String,
}
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct NpcSpeech {
    pub speaker: String,
    pub text: String,
}
#[derive(Message, Clone, Copy, Debug)]
pub struct PartyRemark {
    pub event: u16,
}
#[derive(Component, Debug)]
pub struct ConversationSpeech {
    pub voice: Option<String>,
    pub text: String,
    pub alert: bool,
}
#[derive(Debug)]
struct Cursor {
    quote: u16,
    line: usize,
    paid: bool,
}
#[derive(Debug)]
enum After {
    Wait(ConversationWait),
    End,
}
#[derive(Component, Debug)]
pub struct Conversation {
    pub actor: Entity,
    pub current: Option<(u16, usize)>,
    pub wait: ConversationWait,
    pub transcript: Vec<TranscriptLine>,
    pub learned_topics: BTreeSet<String>,
    pub menu: bool,
    result: Option<Outcome>,
    charms: BTreeMap<Entity, u32>,
    script: VecDeque<Cursor>,
    after: Option<After>,
    incoming: Option<Talk>,
    transaction: bool,
}
impl Conversation {
    fn new(actor: Entity, quote: u16, menu: bool) -> Self {
        let mut c = Self {
            actor,
            current: None,
            wait: ConversationWait::Ready,
            transcript: vec![],
            learned_topics: default(),
            menu,
            result: None,
            charms: default(),
            script: default(),
            after: None,
            incoming: None,
            transaction: false,
        };
        c.say(quote);
        c
    }
    pub fn ready(&self) -> bool {
        self.wait == ConversationWait::Ready
            && self.script.is_empty()
            && self.after.is_none()
            && !self.transaction
    }
    pub fn outcome(&self) -> Option<Outcome> {
        self.result
    }
    pub(crate) fn charm(&self) -> u32 {
        self.charms.get(&self.actor).copied().unwrap_or(0)
    }
    fn say(&mut self, quote: u16) {
        self.script.push_back(Cursor { quote, line: 0, paid: false });
    }
    fn paid(&mut self, quote: u16) {
        self.script.push_back(Cursor { quote, line: 0, paid: true });
    }
    fn block(&mut self, quote: u16, entry: usize, reason: String) {
        self.wait = ConversationWait::Blocked { quote, entry: Some(entry), reason };
        self.script.clear();
        self.after = None;
    }
    pub(crate) fn magical_reply(&mut self, reply: MagicalReply) {
        match reply {
            MagicalReply::Unnoticed => {}
            MagicalReply::Quote(q) => self.say(q),
            MagicalReply::Charmed(n) => {
                self.charms.entry(self.actor).and_modify(|old| *old = (*old).max(n)).or_insert(n);
            }
        }
    }
}
pub(crate) enum MagicalReply {
    Unnoticed,
    Charmed(u32),
    Quote(u16),
}
pub fn install(app: &mut App) {
    app.add_message::<BeginConversation>()
        .add_message::<ConversationRequest>()
        .add_message::<LineSpoken>()
        .add_message::<EditNotebook>()
        .add_message::<NpcSpeech>()
        .add_message::<PartyRemark>()
        .add_message::<rewards::Reward>()
        .add_message::<rewards::Practice>()
        .init_resource::<playback::NoAnswer>()
        .configure_sets(
            Update,
            (Input, trade::Trading, super::recruit::Recruiting, Playback)
                .chain()
                .in_set(Conversing)
                .after(crate::game::GameplaySet::Combat),
        )
        .add_systems(
            Update,
            (
                input::interrupt,
                input::finish,
                input::begin,
                input::receive,
                input::text,
                social::approach,
                social::gift,
                social::gold,
                input::payment,
                input::transactions,
            )
                .chain()
                .in_set(Input),
        )
        .add_systems(
            Update,
            (
                input::receipts,
                social::welcome,
                playback::advance,
                rewards::deliver,
                rewards::practise,
            )
                .chain()
                .in_set(Playback),
        )
        .add_systems(Update, input::interrupt.before(crate::game::GameplaySet::LevelActivation));
}
