use super::*;
use crate::{
    content::npcs::{DialogueOp, TopicCategory},
    game::{
        combat::Battle,
        data::Catalogs,
        factions::{Band, Factions},
        freedom::Freedom,
        inventory::PartyInventory,
        npcs::{
            Npc,
            access::{NpcActors, Scripts},
            story::StoryFacts,
        },
        party::body::PartyBody,
        world::{facing, level::Destination},
    },
};
use avian3d::prelude::{Position, Rotation};

pub(super) fn begin(
    mut requests: MessageReader<BeginConversation>,
    freedom: Freedom,
    conversations: Query<(), With<Conversation>>,
    battles: Query<(), With<Battle>>,
    actors: NpcActors,
    npcs: Query<&Npc>,
    factions: Res<Factions>,
    scripts: Scripts,
    party: Single<(Entity, &Position)>,
    held: crate::game::items::Items,
    mut practice: MessageWriter<rewards::Practice>,
    mut visits: MessageWriter<trade::Visit>,
    mut commands: Commands,
) {
    let mut admitted = !conversations.is_empty();
    for request in requests.read() {
        if admitted || !freedom.free() || !battles.is_empty() || held.held(party.0).is_some() {
            continue;
        }
        if !actors.available(request.actor) || scripts.get(request.actor).is_none() {
            continue;
        }
        let centre = PartyBody::centre(PartyBody::eye(party.1));
        if let Some(body) = actors.active_body(request.actor) {
            let Some(at) = actors.position(body) else { continue };
            if at.distance(centre) > crate::game::world::items::REACH {
                continue;
            }
            if let Some(rotation) = facing(at, centre) {
                commands.entity(body).insert(Rotation(rotation));
            }
        }
        let (Some(d), Ok(npc)) = (actors.definition(request.actor), npcs.get(request.actor)) else {
            continue;
        };
        let band = if d.monster_bound || d.voice_script || d.merchant {
            Band::Friendly
        } else {
            social::band(npc.disposition, 0, actors.turned(request.actor), d, &factions)
        };
        let quote = match band {
            Band::Hostile => 0x18,
            Band::Neutral => 2,
            Band::Friendly => {
                if npc.greeting_pending || d.merchant {
                    0
                } else {
                    1
                }
            }
        };
        let mut e = commands.spawn(Conversation::new(request.actor, quote, band == Band::Neutral));
        if band == Band::Hostile {
            e.insert(Closing);
        }
        if quote == 0 && !d.monster_bound && !d.voice_script && !d.merchant {
            practice.write(rewards::Practice::Greeting);
        }
        if band != Band::Hostile {
            visits.write(trade::Visit(request.actor));
        }
        admitted = true;
    }
}
pub(super) fn interrupt(
    conversations: Query<(Entity, &Conversation)>,
    actors: NpcActors,
    occupied: Query<(), Or<(With<Destination>, With<Battle>)>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut commands: Commands,
) {
    for (entity, c) in &conversations {
        if !occupied.is_empty() || camp.is_some() || !actors.available(c.actor) {
            commands.entity(entity).despawn();
        }
    }
}
pub(super) fn finish(
    mut spoken: MessageReader<LineSpoken>,
    mut conversations: Query<&mut Conversation>,
    mut edits: MessageReader<EditNotebook>,
    mut story: ResMut<StoryFacts>,
    mut commands: Commands,
) {
    for m in spoken.read() {
        if let Ok(mut c) = conversations.get_mut(m.conversation)
            && c.wait == (ConversationWait::Line { speech: m.speech })
        {
            c.wait = ConversationWait::Ready;
            commands.entity(m.speech).despawn();
        }
    }
    for e in edits.read() {
        match e.edit {
            NotebookEdit::Remember => story.remember_topic(TopicCategory::Misc, &e.text),
            NotebookEdit::Forget => story.forget_topic(&e.text),
        };
    }
}
pub(super) fn receive(
    mut requests: MessageReader<ConversationRequest>,
    mut conversations: Query<(&mut Conversation, Has<Closing>)>,
    mut commands: Commands,
) {
    let mut accepted = std::collections::HashSet::new();
    for request in requests.read() {
        let Ok((mut c, closing)) = conversations.get_mut(request.conversation) else { continue };
        if closing || c.transaction || !accepted.insert(request.conversation) {
            continue;
        }
        if matches!(request.talk, Talk::Close) {
            c.charms.clear();
            commands.entity(request.conversation).insert(Closing);
            continue;
        }
        let accepts = match &request.talk {
            Talk::PriceAnswer { .. } => {
                matches!(c.wait, ConversationWait::Price { .. } | ConversationWait::GiveAll { .. })
            }
            Talk::Reply { .. } => matches!(c.wait, ConversationWait::Options { .. }),
            _ => c.ready(),
        };
        if !accepts {
            continue;
        }
        c.result = None;
        c.incoming = Some(request.talk.clone());
    }
}
pub(super) fn text(
    mut conversations: Query<&mut Conversation>,
    scripts: Scripts,
    mut story: ResMut<StoryFacts>,
) {
    for mut c in &mut conversations {
        let Some(talk) =
            c.incoming.take_if(|t| matches!(t, Talk::Topic { .. } | Talk::Reply { .. }))
        else {
            continue;
        };
        let Some(script) = scripts.get(c.actor) else { continue };
        match talk {
            Talk::Topic { text } if c.ready() && !c.menu => {
                let cleaned: String = text
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
                    .collect();
                let words: Vec<_> = cleaned.split_whitespace().collect();
                let quote = script
                    .topic(&cleaned)
                    .or_else(|| words.windows(2).find_map(|w| script.topic(&w.join(" "))))
                    .or_else(|| words.iter().find_map(|w| script.topic(w)));
                if let Some(q) = quote {
                    story.remember_topic(TopicCategory::Misc, &cleaned);
                    c.transcript.push(TranscriptLine::Question(text));
                    c.say(q);
                } else {
                    c.result = Some(Outcome::NoTopic);
                }
            }
            Talk::Reply { text } => {
                let ConversationWait::Options { quote, words, .. } = &c.wait else { continue };
                let Some(q) = script.quotes.get(usize::from(*quote)) else { continue };
                let matched = words.iter().any(|w| w.eq_ignore_ascii_case(&text));
                let explicit = q.operations.iter().find_map(|op| match op {
                    DialogueOp::Reply { response, role: 3, words }
                        if matched && words.iter().any(|w| w.eq_ignore_ascii_case(&text)) =>
                    {
                        Some(*response)
                    }
                    _ => None,
                });
                let next = explicit.or_else(|| {
                    q.operations.iter().find_map(|op| match op {
                        DialogueOp::Reply { response, role, .. }
                            if *role == if matched { 1 } else { 2 } =>
                        {
                            Some(*response)
                        }
                        _ => None,
                    })
                });
                if let Some(q) = next {
                    c.transcript.push(TranscriptLine::Reply(text));
                    c.wait = ConversationWait::Ready;
                    c.say(q);
                }
            }
            _ => {}
        }
    }
}
pub(super) fn payment(
    mut conversations: Query<&mut Conversation>,
    scripts: Scripts,
    content: If<Res<Catalogs>>,
    mut npcs: Query<&mut Npc>,
    mut inventory: Single<&mut PartyInventory>,
) {
    for mut c in &mut conversations {
        if !matches!(c.wait, ConversationWait::Price { .. }) {
            continue;
        }
        let Some(Talk::PriceAnswer { accept }) =
            c.incoming.take_if(|t| matches!(t, Talk::PriceAnswer { .. }))
        else {
            continue;
        };
        let ConversationWait::Price { quote, gold, always } = c.wait else { continue };
        let Some(q) = scripts.get(c.actor).and_then(|s| s.quotes.get(usize::from(quote))) else {
            continue;
        };
        c.wait = ConversationWait::Ready;
        if !accept {
            for op in &q.operations {
                if let DialogueOp::Decline(q) = op {
                    c.say(*q);
                }
            }
            continue;
        }
        if let Err((entry, error)) = playback::validate(q, &content) {
            c.block(quote, entry, error);
            continue;
        }
        if inventory.gold < gold {
            c.paid(0x14);
            continue;
        }
        let Ok(mut npc) = npcs.get_mut(c.actor) else { continue };
        inventory.gold -= gold;
        if !always {
            npc.paid_topics.push(quote);
        }
        c.paid(quote);
    }
}
pub(super) fn transactions(
    mut conversations: Query<(Entity, &mut Conversation)>,
    mut trades: MessageWriter<trade::Order>,
    mut joins: MessageWriter<super::super::recruit::Join>,
) {
    for (entity, mut c) in &mut conversations {
        let Some(talk) = c.incoming.take_if(|t| matches!(t, Talk::Trade(_) | Talk::Recruit)) else {
            continue;
        };
        if !c.ready() {
            continue;
        }
        match talk {
            Talk::Trade(q) => {
                if q.conversation != entity || q.offer.merchant != c.actor {
                    c.result = Some(Outcome::TradeRefused(TradeError::StaleSelection));
                    continue;
                }
                trades.write(trade::Order { reply_to: entity, quote: q.offer });
                c.transaction = true;
            }
            Talk::Recruit => {
                joins.write(super::super::recruit::Join { actor: c.actor, reply_to: entity });
                c.transaction = true;
            }
            _ => {}
        }
    }
}
pub(super) fn receipts(
    mut trades: MessageReader<trade::Receipt>,
    mut joins: MessageReader<super::super::recruit::Joined>,
    mut conversations: Query<&mut Conversation>,
) {
    for receipt in trades.read() {
        if let Ok(mut c) = conversations.get_mut(receipt.reply_to) {
            c.transaction = false;
            c.result = Some(match receipt.result {
                Ok(()) => Outcome::Traded,
                Err(e) => Outcome::TradeRefused(e),
            });
        }
    }
    for receipt in joins.read() {
        if let Ok(mut c) = conversations.get_mut(receipt.reply_to) {
            c.transaction = false;
            let q = match receipt.result {
                Ok(()) => {
                    c.after = Some(After::End);
                    0xd
                }
                Err(super::super::recruit::JoinRefusal::NoRoom) => 0xc,
                Err(_) => 0xb,
            };
            c.say(q);
        }
    }
}
pub fn quote_trade(
    In((entity, source, quantity)): In<(Entity, TradeSource, u32)>,
    conversations: Query<(&Conversation, Has<Closing>)>,
    market: trade::Pricing,
) -> Result<TradeQuote, TradeError> {
    let (c, closing) = conversations.get(entity).map_err(|_| TradeError::StaleSelection)?;
    if closing || !c.ready() {
        return Err(TradeError::Busy);
    }
    Ok(TradeQuote { conversation: entity, offer: market.quote(c.actor, source, quantity)? })
}
