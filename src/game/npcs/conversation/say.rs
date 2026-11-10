//! What the person says and does, in order (`docs/rules/conversation.md`
//! sections 2 and 3): the next line is spoken when nothing is being said or
//! asked, and a quote's entries are carried out as its last line begins.

use std::collections::VecDeque;

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::acts::{Regard, Voices, greeting};
use super::{
    Asked, Blocked, Conversation, ConversationSpeech, Dialogue, DialogueItem, NpcSpeech,
    PartyRemark, QuestReward, SaidIn, TranscriptLine,
};
use crate::content::npcs::{DialogueOp, Gift, Quote, Response, TopicCategory};
use crate::game::camp::Camp;
use crate::game::characters::{Character, Vitals};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::factions::{Band, FactionNews, Factions};
use crate::game::inventory::PartyInventory;
use crate::game::items::{Give, Items, Lose, Place, Put, make, roll_amount};
use crate::game::npcs::Npc;
use crate::game::npcs::access::{NpcActors, Scripts};
use crate::game::npcs::story::{self, Readings, StoryFacts};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::items::CLEARANCE;
use crate::game::world::level::{Destination, Level};

/// One thing still to come in a conversation.
#[derive(Clone, Debug)]
enum Step {
    /// A quote from one of its lines on; `free` of any price it has.
    Say {
        quote: u16,
        line: usize,
        free: bool,
    },
    /// What a quote gives or takes, and who gave it.
    Give {
        gift: Gift,
        giver: Entity,
    },
    Ask(Asked),
    /// The person has no more to say to the party.
    End,
}

/// What the person still has to say and do, in order. Its ending, once
/// queued, stays last.
#[derive(Component, Debug, Default)]
pub struct Script(VecDeque<Step>);

impl Script {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn push(&mut self, step: Step) {
        match self.0.back() {
            Some(Step::End) => self.0.insert(self.0.len() - 1, step),
            _ => self.0.push_back(step),
        }
    }

    /// The person will say a quote, asking its price first if it has one.
    pub(crate) fn say(&mut self, quote: u16) {
        self.push(Step::Say { quote, line: 0, free: false });
    }

    /// The person will say a quote whatever its price.
    pub(super) fn say_free(&mut self, quote: u16) {
        self.push(Step::Say { quote, line: 0, free: true });
    }

    /// The conversation ends once everything queued has been said.
    pub(super) fn end(&mut self) {
        if !matches!(self.0.back(), Some(Step::End)) {
            self.0.push_back(Step::End);
        }
    }

    fn ask(&mut self, asked: Asked) {
        self.push(Step::Ask(asked));
    }
}

/// Something was queued, or what the conversation waited on is over: it
/// goes on with whatever comes next.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub(crate) struct Continue {
    #[event_target]
    pub conversation: Entity,
}

/// A line has been heard out; the screen says so.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Spoken {
    #[event_target]
    pub speech: Entity,
}

/// A quote hands something over or takes it, once its lines are said.
#[derive(EntityEvent, Clone, Debug)]
pub(super) struct Reward {
    #[event_target]
    conversation: Entity,
    giver: Entity,
    gift: Gift,
}

/// A line that was heard out is gone, and the conversation goes on.
pub(super) fn spoken(
    spoken: On<Spoken>,
    lines: Query<&SaidIn, With<ConversationSpeech>>,
    mut commands: Commands,
) {
    let Ok(&SaidIn(conversation)) = lines.get(spoken.speech) else { return };
    commands.entity(spoken.speech).despawn();
    commands.trigger(Continue { conversation });
}

/// A fight or a journey ends every conversation.
pub(super) fn interrupt(
    _: On<Add<(Destination, Battle)>>,
    conversations: Query<Entity, With<Conversation>>,
    mut commands: Commands,
) {
    for conversation in &conversations {
        commands.entity(conversation).despawn();
    }
}

/// The person's script asks for what this game cannot do: nothing more is
/// said.
pub(super) fn block(talk: &mut DialogueItem, commands: &mut Commands, blocked: Blocked) {
    talk.script.0.clear();
    if talk.closing {
        commands.entity(talk.entity).despawn();
    } else {
        commands.entity(talk.entity).insert(blocked);
    }
}

/// What this game cannot yet do of a quote, checked before the party pays
/// or gives anything for it: which entry, and why. Conditions are evaluated
/// later, when the quote reaches its entries.
pub(super) fn unsupported(q: &Quote, content: &Catalogs) -> Option<(usize, String)> {
    q.operations.iter().enumerate().find_map(|(entry, op)| {
        let error = match op {
            DialogueOp::SetFact(id, _) if !StoryFacts::writable(*id, &content.facts) => {
                Some(format!("unimplemented fact consequences {id:?}"))
            }
            DialogueOp::Conditional { facts, .. } => {
                facts.iter().find_map(|(id, _)| story::unreadable(*id, &content.facts))
            }
            DialogueOp::MonsterScriptLabel(Some(label)) => {
                Some(format!("unimplemented monster script label {label}"))
            }
            DialogueOp::LeaveParty { .. } => Some("unimplemented scripted departure".into()),
            DialogueOp::Unsupported { opcode, .. } => {
                Some(format!("unimplemented opcode {opcode}"))
            }
            _ => None,
        };
        error.map(|error| (entry, error))
    })
}

/// Go on with a conversation (retail `ProcessMessageBoxQueue`,
/// `RunNpcScriptLine`): unless a line is being spoken or a question stands,
/// take what comes next. A quote's next line is spoken; reaching its last,
/// its entries are carried out first. What a quote gives is given; a
/// question is put; an ending ends it. With nothing left, one who has
/// become a friend greets the party, and a conversation the party has left
/// is over.
pub(super) fn carry_on(
    go: On<Continue>,
    mut talks: Query<Dialogue>,
    (scripts, actors, voices, readings): (Scripts, NpcActors, Voices, Readings),
    mut npcs: Query<&mut Npc>,
    (mut story, mut factions): (ResMut<StoryFacts>, ResMut<Factions>),
    (clock, camp): (Res<GameClock>, Option<Res<Camp>>),
    occupied: Query<(), Or<(With<Destination>, With<Battle>)>>,
    mut rng: ResMut<GameRng>,
    mut alternate: Local<bool>,
    (mut remarks, mut speech, mut news): (
        MessageWriter<PartyRemark>,
        MessageWriter<NpcSpeech>,
        MessageWriter<FactionNews>,
    ),
    mut commands: Commands,
) {
    let conversation = go.conversation;
    let Ok(mut talk) = talks.get_mut(conversation) else { return };
    let closing = talk.closing;
    // The person must still be there to talk with, and the party free to.
    if !occupied.is_empty() || camp.is_some() || !actors.available(talk.conversation.actor) {
        commands.entity(conversation).despawn();
        return;
    }
    if talk.speaking.is_some() {
        return;
    }
    if talk.asked.is_some() || talk.blocked.is_some() {
        if closing {
            commands.entity(conversation).despawn();
        }
        return;
    }
    loop {
        let actor = talk.conversation.actor;
        let Some(def) = actors.definition(actor) else { return };
        let Some(step) = talk.script.0.pop_front() else {
            if closing {
                commands.entity(conversation).despawn();
                return;
            }
            // One who has become a friend greets the party as one.
            if let Ok(npc) = npcs.get(actor)
                && talk.conversation.menu
                && Regard::of(&talk, &actors, &voices, &factions)
                    .is_some_and(|regard| regard.band(npc) == Band::Friendly)
            {
                talk.conversation.menu = false;
                talk.script.say(greeting(npc, def));
                continue;
            }
            return;
        };
        let (quote, line, free) = match step {
            Step::End => {
                commands.entity(conversation).despawn();
                return;
            }
            Step::Ask(asked) => {
                if closing {
                    commands.entity(conversation).despawn();
                } else {
                    commands.entity(conversation).insert(asked);
                }
                return;
            }
            // It is given before anything more is said.
            Step::Give { gift, giver } => {
                commands.trigger(Reward { conversation, giver, gift });
                commands.trigger(Continue { conversation });
                return;
            }
            Step::Say { quote, line, free } => (quote, line, free),
        };
        let blocked = |entry, reason: String| Blocked { quote, entry, reason };
        let Some(script) = scripts.get(actor) else {
            block(&mut talk, &mut commands, blocked(0, "speaker script is not loaded".into()));
            return;
        };
        let Some(q) = script.quotes.get(usize::from(quote)) else {
            block(&mut talk, &mut commands, blocked(0, "quote does not exist".into()));
            return;
        };
        if let Some((entry, reason)) = unsupported(q, &actors.content) {
            block(&mut talk, &mut commands, blocked(entry, reason));
            return;
        }
        // A quote with a price is asked for before any of it is said,
        // unless it was paid for: the charm on the person takes its percent
        // off, and the party's best Communication a five-hundredth a point
        // (retail `OpenNpcDialog`).
        if !free
            && line == 0
            && let Some((gold, always)) = q.operations.iter().find_map(|op| match op {
                DialogueOp::Price { gold, always } => Some((*gold, *always)),
                _ => None,
            })
            && npcs.get(actor).is_ok_and(|n| always || !n.paid_topics.contains(&quote))
        {
            let gold = gold - gold * talk.conversation.charm().min(100) / 100;
            let gold = (gold - gold * voices.skill().min(500) / 500).max(1);
            talk.script.say_free(0x12);
            talk.script.ask(Asked::Price { quote, gold, always });
            continue;
        }
        if quote == 0
            && line == 0
            && let Ok(mut npc) = npcs.get_mut(actor)
            && std::mem::take(&mut npc.greeting_pending)
            && story.remember_topic(TopicCategory::Person, &def.source_name)
        {
            talk.conversation.learned_topics.insert(def.source_name.to_lowercase());
        }
        talk.conversation.current = Some((quote, line));
        let text = q.text.get(line).cloned().unwrap_or_default();
        let last = line + 1 >= q.text.len();
        if last {
            let mut failure = None;
            for (entry, op) in q.operations.iter().enumerate() {
                let response = match op {
                    DialogueOp::SetFact(id, value) => {
                        let level = actors.level().map_or("", |(_, l)| l.path.as_str());
                        if let Err(e) = story.set(*id, *value, &actors.content.facts, level) {
                            failure = Some((entry, e));
                            break;
                        }
                        commands.trigger(story::FactSet { fact: *id, value: *value });
                        None
                    }
                    DialogueOp::Mention { category, text } => {
                        if story.remember_topic(*category, text) {
                            talk.conversation.learned_topics.insert(text.trim().to_lowercase());
                        }
                        None
                    }
                    DialogueOp::Give(gift) => {
                        talk.script.push(Step::Give { gift: *gift, giver: actor });
                        None
                    }
                    DialogueOp::AdjustDisposition { npc, delta } => {
                        if let Some(target) = actors.shown(npc)
                            && let Ok(mut n) = npcs.get_mut(target)
                        {
                            n.disposition =
                                (i16::from(n.disposition) + i16::from(*delta)).clamp(0, 99) as u8;
                        }
                        None
                    }
                    DialogueOp::AdjustFaction { faction: Some(f), delta } => {
                        if let Some(change) = factions.adjust(*f, *delta, clock.ms) {
                            news.write(change);
                        }
                        None
                    }
                    DialogueOp::PartyRemark(event) => {
                        remarks.write(PartyRemark { event: *event });
                        None
                    }
                    DialogueOp::Conditional { facts, response } => {
                        let mut holds = true;
                        for (id, value) in facts {
                            match readings.read(*id, &story, &factions) {
                                Ok(actual) if actual == *value => {}
                                Ok(_) => {
                                    holds = false;
                                    break;
                                }
                                Err(e) => {
                                    failure = Some((entry, e));
                                    break;
                                }
                            }
                        }
                        if failure.is_some() {
                            break;
                        }
                        holds.then_some(response)
                    }
                    DialogueOp::Next(response) => Some(response),
                    DialogueOp::Options(words) | DialogueOp::KeywordInput(words) => {
                        talk.script.ask(Asked::Options {
                            quote,
                            words: words.clone(),
                            typed: matches!(op, DialogueOp::KeywordInput(_)),
                        });
                        break;
                    }
                    DialogueOp::Close => {
                        talk.script.end();
                        None
                    }
                    // The person goes back to what they were doing, cool
                    // toward the party.
                    DialogueOp::CloseAndResume => {
                        if let Ok(mut npc) = npcs.get_mut(actor) {
                            npc.disposition = 25;
                        }
                        talk.script.end();
                        break;
                    }
                    _ => None,
                };
                if let Some(response) = response {
                    match response {
                        Response::Fixed(q) => talk.script.say(*q),
                        Response::Random { first, last } if last >= first => {
                            talk.script.say(*first + rng.below(u32::from(last - first) + 1) as u16)
                        }
                        Response::Speaker { npc, quote } => {
                            if let Some(next) = actors.shown(npc) {
                                talk.conversation.actor = next;
                                talk.script.say(*quote);
                            } else {
                                failure = Some((entry, format!("speaker {npc} is unavailable")));
                            }
                        }
                        _ => failure = Some((entry, "invalid random quote range".into())),
                    }
                    break;
                }
            }
            if let Some((entry, reason)) = failure {
                block(&mut talk, &mut commands, blocked(entry, reason));
                return;
            }
        } else {
            talk.script.0.push_front(Step::Say { quote, line: line + 1, free: true });
        }
        // Texts that are not spoken (retail `SpeakNpcSubquote`).
        let token = text.to_ascii_uppercase();
        if !q.alert {
            match token.as_str() {
                "" => continue,
                "EMPTY" | "BLANK" => {
                    if (0x67..=0x69).contains(&quote) {
                        talk.conversation.transcript.push(TranscriptLine::NoAnswer {
                            npc: def.source_name.clone(),
                            second: *alternate,
                        });
                        *alternate = !*alternate;
                    }
                    continue;
                }
                "UNKNOWN" => {
                    talk.script.say(if rng.below(100) < 50 { 0x1c } else { 0x1d });
                    continue;
                }
                "CLASSIFIED" => {
                    talk.script.say(0x1e);
                    continue;
                }
                _ => {}
            }
        }
        let voice = (!q.alert).then(|| {
            let letter = match line {
                0 => String::new(),
                line => char::from(b'a' + (line - 1).min(25) as u8).to_string(),
            };
            format!("{}_{quote:03}{letter}", def.voice_stem())
        });
        let text = if token == "SOUND" && !q.alert { String::new() } else { text };
        commands.spawn((
            ConversationSpeech { voice, text: text.clone(), alert: q.alert },
            SaidIn(conversation),
        ));
        if closing {
            speech.write(NpcSpeech { speaker: def.source_name.clone(), text: text.clone() });
        }
        talk.conversation.transcript.push(TranscriptLine::Npc(text));
        return;
    }
}

/// What a quote gives or takes changes hands, and the party is told (retail
/// `ProcessNpcQuoteEntry`): gold to the purse; experience to each living
/// member; an item to the pool, or what has no room there to the hand if it
/// must not be lost, else to the ground; one of an item out of a pack or the
/// pool.
pub(super) fn deliver(
    reward: On<Reward>,
    mut conversations: Query<&mut Conversation>,
    givers: Query<&crate::game::npcs::ActorId>,
    mut party: Single<(Entity, &PartyRoster, &mut PartyInventory, &Position)>,
    mut characters: Query<(&mut Character, &Vitals)>,
    held: Items,
    content: If<Res<Catalogs>>,
    level: Single<Entity, With<Level>>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let (party, roster, inventory, at) = &mut *party;
    let told = match &reward.gift {
        Gift::Gold(amount) => {
            inventory.gold += *amount;
            let giver = givers
                .get(reward.giver)
                .map_or_else(|_| String::new(), |id| content.npcs[id.0].source_name.clone());
            Some(QuestReward::Gold { giver, amount: *amount })
        }
        Gift::Experience(amount) => {
            for member in roster.iter() {
                if let Ok((mut character, vitals)) = characters.get_mut(member)
                    && !vitals.is_dead()
                {
                    character.award_experience(*amount);
                }
            }
            Some(QuestReward::Experience { amount_each: *amount })
        }
        Gift::Item(id) => {
            let d = &content.items[*id];
            let item = make(&mut commands, d, roll_amount(d, &mut rng, true), true);
            let rests = if d.crucial && held.held(*party).is_none() {
                Place::Hand { party: *party }
            } else {
                Place::Ground { level: *level, at: PartyBody::feet(at) + Vec3::Y * CLEARANCE }
            };
            commands.trigger(Put { item, at: rests });
            commands.trigger(Give { item, to: None, party: *party, pool_first: true, told: false });
            Some(QuestReward::Item { name: d.name.clone() })
        }
        Gift::Take(id) => {
            let is = |&item: &Entity| held.id(item) == Some(*id);
            let packed = roster.iter().find_map(|member| {
                held.pack(member).into_iter().flatten().find(is).map(|item| (Some(member), item))
            });
            let pooled = held.pool(*party).iter().copied().find(is).map(|item| (None, item));
            packed.or(pooled).map(|(member, item)| {
                commands.trigger(Lose { item, amount: 1 });
                QuestReward::Taken {
                    member: member
                        .and_then(|member| characters.get(member).ok())
                        .map(|(character, _)| character.name.clone()),
                }
            })
        }
    };
    if let Some(told) = told
        && let Ok(mut conversation) = conversations.get_mut(reward.conversation)
    {
        conversation.transcript.push(TranscriptLine::Reward(told));
    }
}
