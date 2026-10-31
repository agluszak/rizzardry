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

mod dialogue;
use dialogue::QuotePlan;
mod trading;
pub use dialogue::*;
pub use trading::*;

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
