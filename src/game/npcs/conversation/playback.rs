use super::*;
use crate::{
    content::npcs::{DialogueOp, Quote, Response},
    game::{
        data::Catalogs,
        factions::{FactionNews, Factions},
        npcs::{
            Npc,
            access::{NpcActors, Scripts},
            story::StoryFacts,
        },
        rng::GameRng,
        time::GameClock,
    },
};
#[derive(Resource, Default)]
pub(super) struct NoAnswer(bool);
/// Check support before the player pays or surrenders an item. Conditions are
/// evaluated later, when the quote actually reaches its entries.
pub(super) fn validate(q: &Quote, content: &Catalogs) -> Result<(), (usize, String)> {
    for (entry, op) in q.operations.iter().enumerate() {
        let error = match op {
            DialogueOp::SetFact(id, _) if !StoryFacts::writable(*id, &content.facts) => {
                Some(format!("unimplemented fact consequences {id:?}"))
            }
            DialogueOp::Conditional { facts, .. } => facts
                .iter()
                .find_map(|(id, _)| StoryFacts::default().get(*id, &content.facts).err()),
            DialogueOp::MonsterScriptLabel(Some(label)) => {
                Some(format!("unimplemented monster script label {label}"))
            }
            DialogueOp::LeaveParty { .. } => Some("unimplemented scripted departure".into()),
            DialogueOp::Unsupported { opcode, .. } => {
                Some(format!("unimplemented opcode {opcode}"))
            }
            _ => None,
        };
        if let Some(error) = error {
            return Err((entry, error));
        }
    }
    Ok(())
}
pub(super) fn advance(
    mut conversations: Query<(Entity, &mut Conversation, Has<Closing>)>,
    scripts: Scripts,
    actors: NpcActors,
    mut npcs: Query<&mut Npc>,
    voices: super::social::Voices,
    mut story: ResMut<StoryFacts>,
    mut factions: ResMut<Factions>,
    clock: Res<GameClock>,
    mut rng: ResMut<GameRng>,
    mut alternate: ResMut<NoAnswer>,
    mut rewards: MessageWriter<rewards::Reward>,
    mut remarks: MessageWriter<PartyRemark>,
    mut speech: MessageWriter<NpcSpeech>,
    mut news: MessageWriter<FactionNews>,
    mut commands: Commands,
) {
    for (entity, mut c, closing) in &mut conversations {
        if closing && !matches!(c.wait, ConversationWait::Ready | ConversationWait::Line { .. }) {
            commands.entity(entity).despawn();
            continue;
        }
        if c.wait != ConversationWait::Ready || c.transaction {
            continue;
        }
        let Some(mut cursor) = c.script.pop_front() else {
            match c.after.take() {
                Some(After::Wait(wait)) if !closing => c.wait = wait,
                Some(After::End) | Some(After::Wait(_)) => {
                    commands.entity(entity).despawn();
                }
                None if closing => {
                    commands.entity(entity).despawn();
                }
                None => {}
            }
            continue;
        };
        let actor = c.actor;
        let Some(script) = scripts.get(actor) else {
            c.block(cursor.quote, 0, "speaker script is not loaded".into());
            continue;
        };
        let Some(q) = script.quotes.get(usize::from(cursor.quote)) else {
            c.block(cursor.quote, 0, "quote does not exist".into());
            continue;
        };
        let Some(def) = actors.definition(actor) else { continue };
        if let Err((entry, error)) = validate(q, &actors.content) {
            c.block(cursor.quote, entry, error);
            continue;
        }
        if !cursor.paid
            && cursor.line == 0
            && let Some((gold, always)) = q.operations.iter().find_map(|op| match op {
                DialogueOp::Price { gold, always } => Some((*gold, *always)),
                _ => None,
            })
            && npcs.get(actor).is_ok_and(|n| always || !n.paid_topics.contains(&cursor.quote))
        {
            let gold = gold - gold * c.charm().min(100) / 100;
            let gold = (gold - gold * voices.skill().min(500) / 500).max(1);
            c.after =
                Some(After::Wait(ConversationWait::Price { quote: cursor.quote, gold, always }));
            c.paid(0x12);
            continue;
        }
        if cursor.quote == 0
            && cursor.line == 0
            && let Ok(mut npc) = npcs.get_mut(actor)
            && std::mem::take(&mut npc.greeting_pending)
            && story.remember_topic(TopicCategory::Person, &def.source_name)
        {
            c.learned_topics.insert(def.source_name.to_lowercase());
        }
        c.current = Some((cursor.quote, cursor.line));
        let text = q.text.get(cursor.line).cloned().unwrap_or_default();
        let last = cursor.line + 1 >= q.text.len();
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
                        None
                    }
                    DialogueOp::Mention { category, text } => {
                        if story.remember_topic(*category, text) {
                            c.learned_topics.insert(text.trim().to_lowercase());
                        }
                        None
                    }
                    DialogueOp::GiveGold(n) => {
                        rewards.write(rewards::Reward {
                            session: entity,
                            kind: rewards::Kind::Gold {
                                giver: def.source_name.clone(),
                                amount: *n,
                            },
                        });
                        None
                    }
                    DialogueOp::GiveExperience(n) => {
                        rewards.write(rewards::Reward {
                            session: entity,
                            kind: rewards::Kind::Experience(*n),
                        });
                        None
                    }
                    DialogueOp::GiveItem(id) => {
                        rewards.write(rewards::Reward {
                            session: entity,
                            kind: rewards::Kind::Give(*id),
                        });
                        None
                    }
                    DialogueOp::TakeItem(id) => {
                        rewards.write(rewards::Reward {
                            session: entity,
                            kind: rewards::Kind::Take(*id),
                        });
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
                        let mut yes = true;
                        for (id, value) in facts {
                            match story.get(*id, &actors.content.facts) {
                                Ok(actual) => {
                                    if actual != *value {
                                        yes = false;
                                        break;
                                    }
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
                        if yes { Some(response) } else { None }
                    }
                    DialogueOp::Next(response) => Some(response),
                    DialogueOp::Options(words) | DialogueOp::KeywordInput(words) => {
                        c.after = Some(After::Wait(ConversationWait::Options {
                            quote: cursor.quote,
                            words: words.clone(),
                            typed: matches!(op, DialogueOp::KeywordInput(_)),
                        }));
                        break;
                    }
                    DialogueOp::Close => {
                        c.after = Some(After::End);
                        None
                    }
                    DialogueOp::CloseAndResume => {
                        if let Ok(mut npc) = npcs.get_mut(actor) {
                            npc.disposition = 25;
                        }
                        c.after = Some(After::End);
                        break;
                    }
                    _ => None,
                };
                if let Some(response) = response {
                    match response {
                        Response::Fixed(q) => c.say(*q),
                        Response::Random { first, last } if last >= first => {
                            c.say(*first + rng.below(u32::from(last - first) + 1) as u16)
                        }
                        Response::Speaker { npc, quote } => {
                            if let Some(next) = actors.shown(npc) {
                                c.actor = next;
                                c.say(*quote);
                            } else {
                                failure = Some((entry, format!("speaker {npc} is unavailable")));
                            }
                        }
                        _ => failure = Some((entry, "invalid random quote range".into())),
                    }
                    break;
                }
            }
            if let Some((entry, error)) = failure {
                c.block(cursor.quote, entry, error);
                continue;
            }
        }
        let quote = cursor.quote;
        let line = cursor.line;
        if !last {
            cursor.line += 1;
            cursor.paid = true;
            c.script.push_front(cursor);
        }
        let token = text.to_ascii_uppercase();
        if !q.alert {
            match token.as_str() {
                "" => continue,
                "EMPTY" | "BLANK" => {
                    if (0x67..=0x69).contains(&quote) {
                        c.transcript.push(TranscriptLine::NoAnswer {
                            npc: def.source_name.clone(),
                            second: alternate.0,
                        });
                        alternate.0 = !alternate.0;
                    }
                    continue;
                }
                "UNKNOWN" => {
                    c.say(if rng.below(100) < 50 { 0x1c } else { 0x1d });
                    continue;
                }
                "CLASSIFIED" => {
                    c.say(0x1e);
                    continue;
                }
                _ => {}
            }
        }
        let voice = if q.alert {
            None
        } else {
            Some(format!(
                "{}_{quote:03}{}",
                def.voice_stem(),
                if line == 0 {
                    String::new()
                } else {
                    char::from(b'a' + (line - 1).min(25) as u8).to_string()
                }
            ))
        };
        let text = if token == "SOUND" && !q.alert { String::new() } else { text };
        let line = commands
            .spawn((
                ConversationSpeech { voice, text: text.clone(), alert: q.alert },
                ChildOf(entity),
            ))
            .id();
        if closing {
            speech.write(NpcSpeech { speaker: def.source_name.clone(), text: text.clone() });
        }
        c.transcript.push(TranscriptLine::Npc(text));
        c.wait = ConversationWait::Line { speech: line };
    }
}
