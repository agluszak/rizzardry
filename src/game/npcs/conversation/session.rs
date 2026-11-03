//! Session admission, text input, and speech progression. This module cannot
//! settle a trade or recruit a character.
use super::interaction::{PartyVoices, band, greeting};
use super::script::{self, Effect, Next, quotes};
use super::*;
use crate::content::npcs::*;
use crate::game::npcs::access::Scripts;
use crate::game::{
    combat::Battle,
    factions::{Band, Factions},
    freedom::Freedom,
    inventory::PartyInventory,
    npcs::{Npc, access::NpcActors},
    party::body::PartyBody,
    rng::GameRng,
    world::{facing, level::Destination},
};
use avian3d::prelude::{Position, Rotation};

pub(super) fn begin(
    In(actor): In<Entity>,
    freedom: Freedom,
    battles: Query<(), With<Battle>>,
    actors: NpcActors,
    npcs: Query<&Npc>,
    factions: Res<Factions>,
    party: Single<(&PartyInventory, &Position)>,
    mut commands: Commands,
) -> Result<(Entity, bool), TradeError> {
    if !freedom.free() || !battles.is_empty() || party.0.hand.held.is_some() {
        return Err(TradeError::Busy);
    }
    let body = actors.active_body(actor).ok_or(TradeError::Unavailable)?;
    let at = actors.position(body).ok_or(TradeError::Unavailable)?;
    let centre = PartyBody::centre(PartyBody::eye(party.1));
    if centre.distance(at) > crate::game::world::items::REACH || !actors.available(actor, &factions)
    {
        return Err(TradeError::Unavailable);
    }
    let npc = npcs.get(actor).map_err(|_| TradeError::Unavailable)?;
    let def = actors.definition(actor);
    let standing = if def.monster_bound || def.voice_script || def.merchant {
        Band::Friendly
    } else {
        band(npc.disposition, 0, actors.turned(actor), def, &factions)
    };
    let (hello, practice) = greeting(def, npc);
    let (quote, menu) = match standing {
        Band::Hostile => (quotes::HOSTILE, false),
        Band::Neutral => (quotes::NEUTRAL_GREETING, true),
        Band::Friendly => (hello, false),
    };
    let c = Conversation::new(actor, quote, menu);
    let mut entity = commands.spawn(c);
    if standing == Band::Hostile {
        entity.insert(Closing);
    }
    let entity = entity.id();
    if standing != Band::Hostile
        && let Some(rotation) = facing(at, centre)
    {
        commands.entity(body).entry::<Rotation>().and_modify(move |mut r| r.0 = rotation);
        commands.entity(body).entry::<Transform>().and_modify(move |mut t| t.rotation = rotation);
    }
    Ok((entity, practice && standing == Band::Friendly))
}

/// Revalidate the speaker after preceding operations have applied their commands.
pub(super) fn speaker(
    In(entity): In<Entity>,
    conversations: Query<&Conversation>,
    actors: NpcActors,
    factions: Res<Factions>,
    occupied: Query<(), Or<(With<Destination>, With<Battle>)>>,
    camp: Option<Res<crate::game::camp::Camp>>,
) -> Result<Entity, TradeError> {
    let c = conversations.get(entity).map_err(|_| TradeError::StaleSelection)?;
    if !occupied.is_empty() || camp.is_some() {
        return Err(TradeError::Busy);
    }
    if !actors.available(c.actor, &factions) {
        return Err(TradeError::Unavailable);
    }
    Ok(c.actor)
}

pub(super) fn interrupted(
    conversations: Query<(Entity, &Conversation)>,
    actors: NpcActors,
    factions: Res<Factions>,
    occupied: Query<(), Or<(With<Destination>, With<Battle>)>>,
    camp: Option<Res<crate::game::camp::Camp>>,
) -> Vec<Entity> {
    conversations
        .iter()
        .filter(|(_, c)| {
            !occupied.is_empty() || camp.is_some() || !actors.available(c.actor, &factions)
        })
        .map(|(e, _)| e)
        .collect()
}

pub(super) fn text(
    In((entity, talk)): In<(Entity, Talk)>,
    mut conversations: Query<&mut Conversation>,
    scripts: Scripts,
    mut story: ResMut<StoryFacts>,
) {
    let Ok(mut c) = conversations.get_mut(entity) else { return };
    c.result = None;
    let actor = c.actor;
    match talk {
        Talk::Topic { text } => c.topic(scripts.get(actor), &mut story, &text),
        Talk::Reply { text } => c.reply(scripts.get(actor), &text),
        _ => unreachable!(),
    }
}

pub(super) fn leave(In(entity): In<Entity>, mut commands: Commands) {
    commands.entity(entity).insert(Closing);
}

/// Draining takes ownership of accepted effects; calling it again cannot pay twice.
pub(super) fn drain(
    In(entity): In<Entity>,
    mut conversations: Query<&mut Conversation>,
) -> Vec<Effect> {
    let Ok(mut c) = conversations.get_mut(entity) else { return vec![] };
    std::mem::take(&mut c.queue)
        .into_iter()
        .filter_map(|step| match step {
            Step::Quote { plan, .. } => plan,
            _ => None,
        })
        .flat_map(|plan| plan.effects)
        .collect()
}

pub(super) enum Advance {
    Stop,
    Continue,
    Close,
    Join(Entity),
    Line {
        actor: Entity,
        quote: u16,
        line: usize,
        text: String,
        alert: bool,
        commit: Option<QuotePlan>,
    },
}

pub(super) fn advance(
    In(entity): In<Entity>,
    mut conversations: Query<(&mut Conversation, Has<Closing>)>,
    actors: NpcActors,
    scripts: Scripts,
    voices: PartyVoices,
    mut npcs: Query<&mut Npc>,
    mut story: ResMut<StoryFacts>,
) -> Advance {
    let Ok((mut c, closing)) = conversations.get_mut(entity) else { return Advance::Stop };
    if closing && !matches!(c.wait, ConversationWait::Ready | ConversationWait::Line { .. }) {
        return Advance::Close;
    }
    if c.wait != ConversationWait::Ready {
        return Advance::Stop;
    }
    let Some(step) = c.queue.pop_front() else {
        return if closing { Advance::Close } else { Advance::Stop };
    };
    let (quote, line, paid, prepared) = match step {
        Step::End => return Advance::Close,
        Step::Join => return Advance::Join(c.actor),
        Step::Wait(wait) => {
            c.wait = wait;
            return Advance::Stop;
        }
        Step::Quote { quote, line, paid, plan } => (quote, line, paid, plan),
    };
    let actor = c.actor;
    let Some(q) = scripts.get(actor).quotes.get(usize::from(quote)) else {
        c.block(quote, (0, "missing quote".into()));
        return Advance::Stop;
    };
    if !paid
        && line == 0
        && let Some((gold, always)) = q.operations.iter().find_map(|op| match op {
            DialogueOp::Price { gold, always } => Some((*gold, *always)),
            _ => None,
        })
        && (always || !npcs.get(actor).unwrap().paid_topics.contains(&quote))
    {
        let gold = gold - gold * c.charm().min(100) / 100;
        let gold = (gold - gold * voices.communication().min(500) / 500).max(1);
        c.queue.push_front(Step::Wait(ConversationWait::Price { quote, gold, always }));
        c.queue.push_front(Step::paid(quotes::PRICE_QUESTION, None));
        return Advance::Continue;
    }
    let plan = match prepared.map_or_else(
        || {
            script::plan(
                q,
                &story,
                &actors.content.facts,
                voices.recipients().into_iter(),
                |name| actors.actor_shown_as(name, &story),
            )
        },
        Ok,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            c.block(quote, error);
            return Advance::Stop;
        }
    };
    if quote == quotes::FIRST_GREETING
        && std::mem::take(&mut npcs.get_mut(actor).unwrap().greeting_pending)
    {
        c.learn(&mut story, TopicCategory::Person, &actors.definition(actor).source_name);
    }
    c.current = Some((quote, line));
    let last = line + 1 >= q.text.len();
    let commit = if last {
        Some(plan)
    } else {
        c.queue.push_front(Step::Quote { quote, line: line + 1, paid: true, plan: Some(plan) });
        None
    };
    Advance::Line {
        actor,
        quote,
        line,
        text: q.text.get(line).cloned().unwrap_or_default(),
        alert: q.alert,
        commit,
    }
}

pub(super) struct Playback {
    pub entity: Entity,
    pub actor: Entity,
    pub quote: u16,
    pub line: usize,
    pub text: String,
    pub alert: bool,
    pub next: Option<Next>,
}
/// Effects have already committed when playback chooses the continuation.
pub(super) fn speak(
    In(play): In<Playback>,
    mut conversations: Query<(&mut Conversation, Has<Closing>)>,
    actors: NpcActors,
    story: Res<StoryFacts>,
    mut rng: ResMut<GameRng>,
    mut turn: ResMut<super::NoAnswerTurn>,
    mut news: MessageWriter<NpcSpeech>,
    mut commands: Commands,
) -> bool {
    let Ok((mut c, closing)) = conversations.get_mut(play.entity) else { return false };
    let next = match play.next {
        Some(Next::Quote(Response::Fixed(q))) => Some(Step::quote(q)),
        Some(Next::Quote(Response::Random { first, last })) => {
            Some(Step::quote(first + rng.below(u32::from(last - first) + 1) as u16))
        }
        Some(Next::Quote(Response::Speaker { npc, quote })) => {
            actors.actor_shown_as(&npc, &story).map(|actor| {
                c.actor = actor;
                Step::quote(quote)
            })
        }
        Some(Next::Options(words, typed)) => {
            Some(Step::Wait(ConversationWait::Options { quote: play.quote, words, typed }))
        }
        Some(Next::End) => Some(Step::End),
        None => None,
    };
    if let Some(next) = next {
        c.queue.push_front(next);
    }
    let def = actors.definition(play.actor);
    let line = script::classify(&play.text, play.alert);
    let voice = match line {
        script::Line::Empty => return c.actor == play.actor,
        script::Line::NoAnswer => {
            if quotes::NO_ANSWER.contains(&play.quote) {
                c.transcript.push(TranscriptLine::NoAnswer {
                    npc: def.source_name.clone(),
                    second: turn.0,
                });
                turn.0 = !turn.0;
            }
            return c.actor == play.actor;
        }
        script::Line::Unknown => {
            c.queue.push_back(Step::quote(quotes::UNKNOWN[usize::from(rng.below(100) >= 50)]));
            return c.actor == play.actor;
        }
        script::Line::Classified => {
            c.queue.push_back(Step::quote(quotes::CLASSIFIED));
            return c.actor == play.actor;
        }
        script::Line::Alert => None,
        script::Line::Sound | script::Line::Spoken => {
            let suffix = if play.line == 0 {
                String::new()
            } else {
                char::from(b'a' + (play.line - 1) as u8).to_string()
            };
            Some(format!("{}_{:03}{suffix}", def.voice_stem(), play.quote))
        }
    };
    let text = if matches!(line, script::Line::Sound) { String::new() } else { play.text };
    let speech = commands
        .spawn((
            ConversationSpeech { voice, text: text.clone(), alert: play.alert },
            ChildOf(play.entity),
        ))
        .id();
    if closing && !text.is_empty() {
        news.write(NpcSpeech { speaker: def.source_name.clone(), text: text.clone() });
    }
    c.transcript.push(TranscriptLine::Npc(text));
    c.wait = ConversationWait::Line { speech };
    false
}
