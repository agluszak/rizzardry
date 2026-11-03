//! Serial execution boundary for dialogue transactions. Domain systems own all
//! gameplay rules. Running each operation to completion makes its Commands
//! visible to the next request, including starts, joins and interruptions.
use super::*;
use super::{
    effects::{self, Practice},
    interaction,
    script::{Effect, quotes},
    session::{self, Advance, Playback},
};
use crate::game::npcs::{recruit, trade};

fn practice_greeting(world: &mut World, needed: bool) {
    if needed {
        world.run_system_cached_with(effects::practice, Practice::Greeting).unwrap();
    }
}
fn welcome(world: &mut World, entity: Entity, greet: bool) {
    let practice = world.run_system_cached_with(interaction::welcome, (entity, greet)).unwrap();
    practice_greeting(world, practice);
}
fn apply(world: &mut World, entity: Entity, effect: Effect) {
    let reward = match effect {
        Effect::Fact(id, value) => {
            world.run_system_cached_with(effects::set_fact, (id, value)).unwrap();
            None
        }
        Effect::Mention(category, text) => {
            world.run_system_cached_with(effects::mention, (entity, category, text)).unwrap();
            None
        }
        Effect::Disposition(actor, delta) => {
            world.run_system_cached_with(effects::disposition, (actor, delta)).unwrap();
            None
        }
        Effect::Resume => {
            world.run_system_cached_with(effects::resume, entity).unwrap();
            None
        }
        Effect::Faction(faction, delta) => {
            world.run_system_cached_with(effects::faction, (faction, delta)).unwrap();
            None
        }
        Effect::Remark(event) => {
            world.write_message(PartyRemark { event });
            None
        }
        Effect::Gold(amount) => {
            let actor = world.get::<Conversation>(entity).unwrap().actor;
            Some(world.run_system_cached_with(effects::gold, (actor, amount)).unwrap())
        }
        Effect::Experience(members, amount) => {
            Some(world.run_system_cached_with(effects::experience, (members, amount)).unwrap())
        }
        Effect::GiveItem(item) => {
            Some(world.run_system_cached_with(effects::give_item, item).unwrap())
        }
        Effect::TakeItem(item) => world.run_system_cached_with(effects::take_item, item).unwrap(),
    };
    if let Some(reward) = reward {
        world
            .get_mut::<Conversation>(entity)
            .unwrap()
            .transcript
            .push(TranscriptLine::Reward(reward));
    }
}
fn close(world: &mut World, entity: Entity) {
    let effects = world.run_system_cached_with(session::drain, entity).unwrap();
    for effect in effects {
        apply(world, entity, effect);
    }
    world.despawn(entity);
}

/// Accepted travel settles outstanding promises on the source level, before
/// activation can retire it (including when destination assets are already ready).
pub fn interrupt_for_travel(world: &mut World) {
    if world
        .query_filtered::<(), With<crate::game::world::level::Destination>>()
        .iter(world)
        .next()
        .is_none()
    {
        return;
    }
    let sessions: Vec<_> =
        world.query_filtered::<Entity, With<Conversation>>().iter(world).collect();
    for entity in sessions {
        close(world, entity);
    }
    world.resource_mut::<Messages<BeginConversation>>().clear();
    world.resource_mut::<Messages<ConversationRequest>>().clear();
}

pub fn converse(world: &mut World) {
    if !world.contains_resource::<crate::game::data::Catalogs>() {
        return;
    }
    let interrupted = world.run_system_cached(session::interrupted).unwrap();
    for entity in interrupted {
        close(world, entity);
    }
    let begins: Vec<_> = world.resource_mut::<Messages<BeginConversation>>().drain().collect();
    for BeginConversation { actor } in begins {
        match world.run_system_cached_with(session::begin, actor).unwrap() {
            Ok((entity, practice)) => {
                practice_greeting(world, practice);
                if world.get::<Closing>(entity).is_none() {
                    world.run_system_cached_with(trade::maintain, actor).unwrap();
                }
            }
            Err(error) => warn!("conversation refused: {error:?}"),
        }
    }
    let requests: Vec<_> = world.resource_mut::<Messages<ConversationRequest>>().drain().collect();
    for ConversationRequest { conversation: entity, talk } in requests {
        if world.get::<Conversation>(entity).is_none() || world.get::<Closing>(entity).is_some() {
            continue;
        }
        if matches!(talk, Talk::Close) {
            world.run_system_cached_with(session::leave, entity).unwrap();
            continue;
        }
        let Ok(actor) = world.run_system_cached_with(session::speaker, entity).unwrap() else {
            close(world, entity);
            continue;
        };
        world.get_mut::<Conversation>(entity).unwrap().result = None;
        let outcome = match talk {
            Talk::Topic { .. } | Talk::Reply { .. } => {
                world.run_system_cached_with(session::text, (entity, talk)).unwrap();
                None
            }
            Talk::Approach { approach } => {
                let (member, greeting) = world
                    .run_system_cached_with(interaction::approach, (entity, approach))
                    .unwrap();
                if let Some((member, usage)) = member {
                    world
                        .run_system_cached_with(effects::practice, Practice::Member(member, usage))
                        .unwrap();
                }
                practice_greeting(world, greeting);
                None
            }
            Talk::Give { item } => {
                let result =
                    world.run_system_cached_with(interaction::offer, (entity, item)).unwrap();
                welcome(world, entity, true);
                result.err().map(Outcome::GiftRefused)
            }
            Talk::GiveGold { amount } => {
                let result = world
                    .run_system_cached_with(interaction::gold, (entity, amount, false))
                    .unwrap();
                welcome(world, entity, true);
                result.err().map(Outcome::GiftRefused)
            }
            Talk::PriceAnswer { accept } => {
                let wait = world.get::<Conversation>(entity).unwrap().wait.clone();
                if let ConversationWait::GiveAll { gold } = wait {
                    if accept {
                        let result = world
                            .run_system_cached_with(interaction::gold, (entity, gold, true))
                            .unwrap();
                        welcome(world, entity, false);
                        result.err().map(Outcome::GiftRefused)
                    } else {
                        world.get_mut::<Conversation>(entity).unwrap().wait =
                            ConversationWait::Ready;
                        None
                    }
                } else {
                    world.run_system_cached_with(interaction::price, (entity, accept)).unwrap();
                    None
                }
            }
            Talk::Recruit => {
                if world.get::<Conversation>(entity).unwrap().ready() {
                    let answer = world.run_system_cached_with(recruit::joinable, actor).unwrap();
                    let quote = match answer {
                        Ok(()) => quotes::JOINS,
                        Err(recruit::JoinRefusal::NoRoom) => quotes::NO_ROOM_TO_JOIN,
                        Err(recruit::JoinRefusal::Declines) => quotes::DECLINES_TO_JOIN,
                    };
                    let mut c = world.get_mut::<Conversation>(entity).unwrap();
                    c.queue.push_back(Step::quote(quote));
                    if answer.is_ok() {
                        c.queue.extend([Step::Join, Step::End]);
                    }
                }
                None
            }
            Talk::Trade(quote) => {
                let result = if quote.conversation != entity
                    || quote.merchant != actor
                    || !world.get::<Conversation>(entity).unwrap().ready()
                {
                    Err(TradeError::StaleSelection)
                } else {
                    world.run_system_cached_with(trade::settle, quote).unwrap()
                };
                Some(match result {
                    Ok(()) => Outcome::Traded,
                    Err(error) => Outcome::TradeRefused(error),
                })
            }
            Talk::Close => unreachable!(),
        };
        if let Some(outcome) = outcome {
            world.get_mut::<Conversation>(entity).unwrap().result = Some(outcome);
        }
    }
    let sessions: Vec<_> =
        world.query_filtered::<Entity, With<Conversation>>().iter(world).collect();
    for entity in sessions {
        if world.run_system_cached_with(session::speaker, entity).unwrap().is_err() {
            close(world, entity);
            continue;
        }
        welcome(world, entity, true);
        run(world, entity);
    }
}

fn run(world: &mut World, entity: Entity) {
    // Malformed empty-quote loops must not monopolize the frame.
    for _ in 0..256 {
        match world.run_system_cached_with(session::advance, entity).unwrap() {
            Advance::Stop => return,
            Advance::Continue => {}
            Advance::Close => {
                close(world, entity);
                return;
            }
            Advance::Join(actor) => {
                if let Err(error) = world.run_system_cached_with(recruit::join, actor).unwrap() {
                    warn!("NPC did not join: {error:?}");
                }
            }
            Advance::Line { actor, quote, line, text, alert, commit } => {
                let next = if let Some(plan) = commit {
                    for effect in plan.effects {
                        apply(world, entity, effect);
                    }
                    plan.next
                } else {
                    None
                };
                let again = world
                    .run_system_cached_with(
                        session::speak,
                        Playback { entity, actor, quote, line, text, alert, next },
                    )
                    .unwrap();
                if !again {
                    return;
                }
            }
        }
    }
    let mut c = world.get_mut::<Conversation>(entity).unwrap();
    c.wait = ConversationWait::Blocked {
        quote: c.current.map_or(0, |q| q.0),
        entry: None,
        reason: "script made no bounded progress".into(),
    };
}

/// Read-only session admission followed by the trade owner's price calculation.
pub fn quote_trade(
    In((entity, source, quantity)): In<(Entity, trade::TradeSource, u32)>,
    world: &mut World,
) -> Result<TradeQuote, TradeError> {
    let c = world.get::<Conversation>(entity).ok_or(TradeError::StaleSelection)?;
    if !c.ready() || world.get::<Closing>(entity).is_some() {
        return Err(TradeError::Busy);
    }
    let merchant = world.run_system_cached_with(session::speaker, entity).unwrap()?;
    world.run_system_cached_with(trade::quote, (entity, merchant, source, quantity)).unwrap()
}
