//! Talking with someone (`docs/rules/conversation.md`). A conversation is an
//! entity that names who is spoken with. What the person still has to say
//! and do is its [`Script`]; a line being spoken is an entity of its own
//! ([`ConversationSpeech`]); a question put to the party is [`Asked`]. What
//! the party does is an event at the conversation with one observer each
//! ([`acts`]); saying is one chain ([`say`]).

mod acts;
mod say;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

pub use self::acts::{
    Answer, Ask, Begin, EditNotebook, Invite, Leave, NotebookEdit, Offer, OfferGold, Reply, Sway,
    approach_ready, quote_trade,
};
pub use self::say::Spoken;
pub(crate) use self::say::{Continue, Script};
use super::trade::{self, TradeError};

/// The party has left, or the person will have no more of it: what is being
/// said is finished and nothing more is begun.
#[derive(Component, Debug)]
pub struct Closing;

/// A question the person has put to the party; only an answer is taken
/// while it stands.
#[derive(Component, Clone, Debug, PartialEq)]
pub enum Asked {
    /// One of these words, chosen, or any word `typed`.
    Options { quote: u16, words: Vec<String>, typed: bool },
    /// So much gold for a quote; `always`, however often it was paid for.
    Price { quote: u16, gold: u32, always: bool },
    /// Whether to give everything the party has.
    GiveAll { gold: u32 },
}

/// The person's script asks for something this game cannot do: nothing more
/// is said, and the party can only leave.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Blocked {
    pub quote: u16,
    pub entry: usize,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum QuestReward {
    Gold { giver: String, amount: u32 },
    Experience { amount_each: u32 },
    Item { name: String },
    Taken { member: Option<String> },
}

/// What came of something the party tried, that the person does not answer
/// in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Traded,
    TradeRefused(TradeError),
    GiftRefused(TradeError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptLine {
    Npc(String),
    Question(String),
    Reply(String),
    Reward(QuestReward),
    NoAnswer { npc: String, second: bool },
    Told(Outcome),
}

/// How the party tries to win someone over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    Charm,
    SmallTalk,
}

/// A price the party was quoted in a conversation; triggered, the party
/// buys or sells at it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct TradeQuote {
    #[event_target]
    pub conversation: Entity,
    pub offer: trade::Quote,
}

/// What a person outside a conversation is heard to say.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct NpcSpeech {
    pub speaker: String,
    pub text: String,
}

/// A member has something to say about what was just said.
#[derive(Message, Clone, Copy, Debug)]
pub struct PartyRemark {
    pub event: u16,
}

/// A line being spoken, until the screen says it has ended ([`Spoken`]).
#[derive(Component, Debug)]
pub struct ConversationSpeech {
    pub voice: Option<String>,
    pub text: String,
    pub alert: bool,
}

/// The conversation a line is spoken in.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Speaking)]
pub struct SaidIn(pub Entity);

/// The line being spoken in a conversation, gone with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = SaidIn, linked_spawn)]
pub struct Speaking(Entity);

impl Speaking {
    pub fn line(&self) -> Entity {
        self.0
    }
}

/// The party talking with someone. There is at most one.
#[derive(Component, Debug)]
#[require(Script)]
pub struct Conversation {
    /// Who is spoken with; a quote may hand over to another.
    pub actor: Entity,
    /// The quote and line last begun.
    pub current: Option<(u16, usize)>,
    pub transcript: Vec<TranscriptLine>,
    pub learned_topics: BTreeSet<String>,
    /// The person is not a friend yet: the party may only charm, chat, give
    /// or leave.
    pub menu: bool,
    /// The charm laid on each one spoken with, while this lasts.
    charms: BTreeMap<Entity, u32>,
}

impl Conversation {
    fn new(actor: Entity, menu: bool) -> Self {
        Self {
            actor,
            current: None,
            transcript: vec![],
            learned_topics: default(),
            menu,
            charms: default(),
        }
    }

    /// The charm on the one spoken with.
    pub(crate) fn charm(&self) -> u32 {
        self.charms.get(&self.actor).copied().unwrap_or(0)
    }

    /// A charm takes the one spoken with, if stronger than what holds them.
    pub(crate) fn charmed(&mut self, amount: u32) {
        let held = self.charms.entry(self.actor).or_insert(0);
        *held = (*held).max(amount);
    }
}

/// A conversation and what it is waiting on.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct Dialogue {
    pub entity: Entity,
    pub conversation: &'static mut Conversation,
    script: &'static mut Script,
    pub speaking: Option<&'static Speaking>,
    pub asked: Option<&'static Asked>,
    pub blocked: Option<&'static Blocked>,
    pub closing: Has<Closing>,
}

/// Nothing is being said, asked or still to come, and the party has not
/// left: the party may act.
fn idle(script: &Script, speaking: bool, asked: bool, blocked: bool, closing: bool) -> bool {
    script.is_empty() && !speaking && !asked && !blocked && !closing
}

impl DialogueItem<'_, '_> {
    /// Whether the party may act now.
    pub fn idle(&self) -> bool {
        let waits = (self.speaking.is_some(), self.asked.is_some(), self.blocked.is_some());
        idle(&self.script, waits.0, waits.1, waits.2, self.closing)
    }
}

impl DialogueReadOnlyItem<'_, '_> {
    /// Whether the party may act now.
    pub fn idle(&self) -> bool {
        let waits = (self.speaking.is_some(), self.asked.is_some(), self.blocked.is_some());
        idle(self.script, waits.0, waits.1, waits.2, self.closing)
    }
}

pub fn install(app: &mut App) {
    app.add_message::<NpcSpeech>()
        .add_message::<PartyRemark>()
        .add_observer(say::carry_on)
        .add_observer(say::spoken)
        .add_observer(say::deliver)
        .add_observer(say::interrupt)
        .add_observer(acts::begin)
        .add_observer(acts::ask)
        .add_observer(acts::answer)
        .add_observer(acts::offer)
        .add_observer(acts::offer_gold)
        .add_observer(acts::sway)
        .add_observer(acts::invite)
        .add_observer(acts::trade)
        .add_observer(acts::leave)
        .add_observer(acts::edit_notebook);
}
