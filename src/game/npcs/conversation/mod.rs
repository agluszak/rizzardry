//! Script progression and the lifetime of an NPC conversation.
//!
//! `Conversation` owns the queue, waits, transcript and temporary charms;
//! `Closing` means its remaining speech plays without accepting more input.
//! Script planning reads facts without changing gameplay. Accepted plans stay
//! with their quote until its last line or an interruption commits them.
//!
//! The serial driver invokes independently borrowed systems and applies their
//! commands before advancing. Recruitment and trade own their transactions;
//! the driver owns ordering, not their rules. Travel drains accepted plans
//! before level activation, so rewards and journal entries use the source level.
mod driver;
mod effects;
mod interaction;
mod script;
mod session;

use super::story::StoryFacts;
use super::trade::{TradeError, TradeQuote};
use crate::content::npcs::TopicCategory;
use crate::game::inventory::ItemRef;
use bevy::prelude::*;
pub use driver::{converse, interrupt_for_travel, quote_trade};
pub use interaction::approach_ready;
use script::{QuotePlan, Step};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Resource, Debug, Default)]
pub struct NoAnswerTurn(bool);

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
    charms: BTreeMap<Entity, u32>,
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
    Charmed(u32),
    Quote(u16),
}
impl Conversation {
    pub fn ready(&self) -> bool {
        self.wait == ConversationWait::Ready && self.queue.is_empty()
    }

    pub(crate) fn charm(&self) -> u32 {
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
    Close,
    Recruit,
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
#[derive(Component, Debug)]
pub struct Closing;
/// What the speaker of a dialogue the party has left says, for the text box
/// (retail `SpeakNpcSubquote` outside dialogue mode: "Name: text").
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct NpcSpeech {
    pub speaker: String,
    pub text: String,
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
