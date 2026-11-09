//! Dialogue-only casts validate the live speaker and ready script before the
//! ordinary source transaction spends points, item uses, fatigue or RNG.
use super::{
    Outcome,
    casting::{Aim, MagicChoice, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{characters::Realm, spells::SpellId},
    game::{
        npcs::{
            Npc,
            access::NpcActors,
            conversation::{Conversation, MagicalReply},
        },
        rng::GameRng,
        world::population::Monster,
    },
};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::CHARM | SpellId::MINDREAD)
}

pub(crate) fn cast(
    world: &mut World,
    caster: Entity,
    choice: MagicChoice,
    id: SpellId,
) -> Result<(), Refusal> {
    let (entity, conversation) =
        world.query::<(Entity, &Conversation)>().single(world).map_err(|_| Refusal::WrongTime)?;
    if !conversation.ready() {
        return Err(Refusal::WrongTime);
    }
    let actor = conversation.actor;
    // The casting path still holds the world; the speaker's body is read as
    // dialogues read it.
    let body = world
        .run_system_cached_with(
            |In(actor): In<Entity>, speakers: NpcActors| speakers.active_body(actor),
            actor,
        )
        .unwrap()
        .ok_or(Refusal::NoTarget)?;
    if choice.aim != Aim::Body(body) {
        return Err(Refusal::NoTarget);
    }
    let monster = world.get::<Monster>(body).unwrap();
    let vitals = world.get::<crate::game::characters::Vitals>(body).unwrap();
    let kind = &world.resource::<crate::game::data::Catalogs>().species[monster.species];
    let (level, resistance, notice_chance) = (
        u32::from(kind.level),
        crate::game::world::population::resistance(kind, vitals, Realm::Mental),
        (u32::from(kind.attributes.senses) + u32::from(kind.attributes.intelligence)) / 2,
    );
    let (mut report, strength) =
        super::casting::cast_source(world, caster, choice.source, UseContext::Conversation)?;
    let mut reply = MagicalReply::Unnoticed;
    match report.outcome {
        Outcome::Fizzled => {}
        Outcome::Backfired => {
            if world.resource_mut::<GameRng>().below(100) < notice_chance {
                reply = MagicalReply::Quote(0x16);
            }
        }
        Outcome::Cast => {
            let chance = crate::game::hits::defences::condition_chance(
                level,
                resistance,
                strength.against,
                0,
            );
            let saved = crate::game::hits::defences::save_roll(
                &mut world.resource_mut::<GameRng>(),
                chance,
            );
            if id == SpellId::MINDREAD {
                reply = MagicalReply::Quote(if saved {
                    0x69
                } else {
                    let mut npc = world.get_mut::<Npc>(actor).unwrap();
                    let quote = 0x67 + u16::from(npc.mindread_refusals);
                    npc.mindread_refusals = (npc.mindread_refusals + 1).min(2);
                    quote
                });
            } else if saved {
                report.effects.push(crate::game::hits::Effect::Resisted { victim: body });
            } else {
                let amount = strength.power * 7;
                let amount = amount + amount * strength.boost / 100;
                let amount = crate::game::hits::defences::resist(
                    &mut world.resource_mut::<GameRng>(),
                    amount,
                    resistance.min(100),
                    level,
                    strength.against,
                );
                if amount > world.get::<Conversation>(entity).unwrap().charm() {
                    reply = MagicalReply::Charmed(amount);
                }
            }
        }
    }
    // An NPC won over leaves its menu as its script next runs.
    world.get_mut::<Conversation>(entity).unwrap().magical_reply(reply);
    world.write_message(MagicNews::Done(report));
    Ok(())
}
