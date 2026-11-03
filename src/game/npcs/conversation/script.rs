//! The script machine. Planning is read-only; a quote commits once, at its
//! last line, or when an interruption drains an already accepted quote.
use super::*;
use crate::content::npcs::*;
use crate::content::{factions::Faction, items::ItemId};

/// Fixed roles in retail's NPC scripts; authored topic numbers stay in content.
pub(super) mod quotes {
    pub const FIRST_GREETING: u16 = 0;
    pub const RETURN_GREETING: u16 = 1;
    pub const NEUTRAL_GREETING: u16 = 2;
    pub const SMALL_TALK: u16 = 3;
    pub const CHARMED: u16 = 4;
    pub const UNCHARMED: u16 = 6;
    pub const UNBRIBED: u16 = 7;
    pub const DECLINES_TO_JOIN: u16 = 0xb;
    pub const NO_ROOM_TO_JOIN: u16 = 0xc;
    pub const JOINS: u16 = 0xd;
    pub const ACCEPTED_GIFT: u16 = 0x10;
    pub const REFUSED_GIFT: u16 = 0x11;
    pub const PRICE_QUESTION: u16 = 0x12;
    pub const TOO_POOR: u16 = 0x14;
    pub const HOSTILE: u16 = 0x18;
    pub const UNKNOWN: [u16; 2] = [0x1c, 0x1d];
    pub const CLASSIFIED: u16 = 0x1e;
    pub const NO_ANSWER: std::ops::RangeInclusive<u16> = 0x67..=0x69;
}

#[derive(Debug)]
pub(super) enum Step {
    Quote { quote: u16, line: usize, paid: bool, plan: Option<QuotePlan> },
    Wait(ConversationWait),
    Join,
    End,
}
impl Step {
    pub fn quote(quote: u16) -> Self {
        Self::Quote { quote, line: 0, paid: false, plan: None }
    }
    pub fn paid(quote: u16, plan: Option<QuotePlan>) -> Self {
        Self::Quote { quote, line: 0, paid: true, plan }
    }
}

#[derive(Debug)]
pub(super) struct QuotePlan {
    pub effects: Vec<Effect>,
    pub next: Option<Next>,
}
#[derive(Debug)]
pub(super) enum Effect {
    Fact(FactId, u8),
    Mention(TopicCategory, String),
    Gold(u32),
    Experience(Vec<Entity>, u32),
    Faction(Faction, i8),
    GiveItem(ItemId),
    TakeItem(ItemId),
    Disposition(Entity, i8),
    Remark(u16),
    Resume,
}
#[derive(Debug)]
pub(super) enum Next {
    Quote(Response),
    Options(Vec<String>, bool),
    End,
}

pub(super) fn plan(
    quote: &Quote,
    story: &StoryFacts,
    facts: &FactCatalog,
    recipients: impl Iterator<Item = Entity> + Clone,
    named: impl Fn(&str) -> Option<Entity>,
) -> Result<QuotePlan, (usize, String)> {
    let mut result = QuotePlan { effects: vec![], next: None };
    let mut writes = BTreeMap::new();
    for (entry, op) in quote.operations.iter().enumerate() {
        let effect = match op {
            DialogueOp::SetFact(id, value) => {
                if !StoryFacts::can_set(*id, facts) {
                    return Err((entry, format!("unported fact assignment {id:?}")));
                }
                writes.insert(*id, *value);
                Effect::Fact(*id, *value)
            }
            DialogueOp::GiveGold(amount) => Effect::Gold(*amount),
            DialogueOp::GiveExperience(amount) => {
                Effect::Experience(recipients.clone().collect(), *amount)
            }
            DialogueOp::GiveItem(item) => Effect::GiveItem(*item),
            DialogueOp::TakeItem(item) => Effect::TakeItem(*item),
            DialogueOp::AdjustFaction { faction: Some(faction), delta } => {
                Effect::Faction(*faction, *delta)
            }
            DialogueOp::AdjustDisposition { npc, delta } => {
                let Some(actor) = named(npc) else { continue };
                Effect::Disposition(actor, *delta)
            }
            DialogueOp::Mention { category, text } => Effect::Mention(*category, text.clone()),
            DialogueOp::PartyRemark(event) => Effect::Remark(*event),
            DialogueOp::Conditional { facts: conditions, response } => {
                let mut matches = true;
                for (id, expected) in conditions {
                    let actual = match writes.get(id) {
                        Some(value) => *value,
                        None => story.get(*id, facts).map_err(|reason| (entry, reason))?,
                    };
                    if actual != *expected {
                        matches = false;
                        break;
                    }
                }
                if !matches {
                    continue;
                }
                result.next = Some(Next::Quote(response.clone()));
                break;
            }
            DialogueOp::Next(response) => {
                result.next = Some(Next::Quote(response.clone()));
                break;
            }
            DialogueOp::Options(words) => {
                result.next = Some(Next::Options(words.clone(), false));
                break;
            }
            DialogueOp::KeywordInput(words) => {
                result.next = Some(Next::Options(words.clone(), true));
                break;
            }
            DialogueOp::CloseAndResume => {
                result.effects.push(Effect::Resume);
                result.next = Some(Next::End);
                break;
            }
            DialogueOp::Close => {
                result.next = Some(Next::End);
                continue;
            }
            DialogueOp::MonsterScriptLabel(Some(label)) => {
                return Err((entry, format!("unported monster script label {label}")));
            }
            DialogueOp::Unsupported { opcode, .. } => {
                return Err((entry, format!("unported mandatory opcode {opcode}")));
            }
            _ => continue,
        };
        result.effects.push(effect);
    }
    Ok(result)
}

pub(super) enum Line {
    Alert,
    Empty,
    NoAnswer,
    Unknown,
    Classified,
    Sound,
    Spoken,
}
pub(super) fn classify(text: &str, alert: bool) -> Line {
    if alert {
        return Line::Alert;
    }
    match text.to_ascii_uppercase().as_str() {
        "" => Line::Empty,
        "EMPTY" | "BLANK" => Line::NoAnswer,
        "UNKNOWN" => Line::Unknown,
        "CLASSIFIED" => Line::Classified,
        "SOUND" => Line::Sound,
        _ => Line::Spoken,
    }
}

impl Conversation {
    pub(super) fn new(actor: Entity, greeting: u16, menu: bool) -> Self {
        Self {
            actor,
            charms: default(),
            current: None,
            wait: ConversationWait::Ready,
            transcript: vec![],
            learned_topics: default(),
            menu,
            result: None,
            queue: VecDeque::from([Step::quote(greeting)]),
        }
    }
    pub(super) fn block(&mut self, quote: u16, error: (usize, String)) {
        self.wait = ConversationWait::Blocked { quote, entry: Some(error.0), reason: error.1 };
    }
    pub(super) fn learn(&mut self, story: &mut StoryFacts, category: TopicCategory, text: &str) {
        if story.remember_topic(category, text) {
            self.learned_topics.insert(text.trim().to_ascii_lowercase());
        }
    }
    pub(super) fn topic(&mut self, script: &DialogueScript, story: &mut StoryFacts, text: &str) {
        if !self.ready() || self.menu {
            return;
        }
        let cleaned: String = text
            .chars()
            .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '\'')
            .collect();
        let words: Vec<_> = cleaned.split_whitespace().collect();
        let matched = script
            .topic(&cleaned)
            .or_else(|| words.windows(2).find_map(|pair| script.topic(&pair.join(" "))))
            .or_else(|| words.iter().find_map(|word| script.topic(word)));
        match matched {
            Some(quote) => {
                story.remember_topic(TopicCategory::Misc, &cleaned);
                self.transcript.push(TranscriptLine::Question(text.into()));
                self.queue.push_back(Step::quote(quote));
            }
            None => self.result = Some(Outcome::NoTopic),
        }
    }
    pub(super) fn reply(&mut self, script: &DialogueScript, text: &str) {
        let ConversationWait::Options { quote, words, .. } = &self.wait else { return };
        let Some(quote) = script.quotes.get(usize::from(*quote)) else { return };
        let matched = words.iter().any(|w| w.eq_ignore_ascii_case(text));
        let role = if matched { 1 } else { 2 };
        let response = quote
            .operations
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
                quote.operations.iter().find_map(|op| match op {
                    DialogueOp::Reply { response, role: r, .. } if *r == role => Some(*response),
                    _ => None,
                })
            });
        if let Some(quote) = response {
            self.transcript.push(TranscriptLine::Reply(text.into()));
            self.wait = ConversationWait::Ready;
            self.queue.push_back(Step::quote(quote));
        }
    }
}
