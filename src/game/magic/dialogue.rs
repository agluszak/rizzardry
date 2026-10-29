//! Dialogue-only casts validate the live speaker and ready script before the
//! ordinary source transaction spends points, item uses, fatigue or RNG.
use super::{
    Outcome,
    casting::{Aim, MagicChoice, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{characters::Realm, npcs::DialogueScript, spells::SpellId},
    game::{
        data::GameData,
        npcs::{
            Npc,
            conversation::{Conversation, MagicalReply, active_body},
        },
        rng::GameRng,
        world::population::Monster,
    },
};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::CHARM | SpellId::MINDREAD)
}

pub(crate) fn cast(world: &mut World, caster: Entity, choice: MagicChoice) -> Result<(), Refusal> {
    let id = super::utilities::source_spell(world, caster, choice.source)?;
    if !spell(id) {
        return Err(Refusal::Unsupported);
    }
    let (entity, conversation) =
        world.query::<(Entity, &Conversation)>().single(world).map_err(|_| Refusal::WrongTime)?;
    if !conversation.ready() {
        return Err(Refusal::WrongTime);
    }
    let actor = conversation.actor;
    let body = active_body(world, actor).ok_or(Refusal::NoTarget)?;
    if choice.aim != Aim::Monster(body) {
        return Err(Refusal::NoTarget);
    }
    let script = world
        .get_resource::<Assets<DialogueScript>>()
        .and_then(|scripts| scripts.get(&conversation.script))
        .ok_or(Refusal::MissingContent)?;
    if script.quotes.get(0x16).is_none() || id == SpellId::MINDREAD && script.quotes.len() <= 0x69 {
        return Err(Refusal::MissingContent);
    }
    world.get::<Npc>(actor).ok_or(Refusal::NoTarget)?;
    let monster = world.get::<Monster>(body).ok_or(Refusal::NoTarget)?;
    let kind = GameData::species(world)
        .and_then(|catalog| catalog.kind(monster.species))
        .ok_or(Refusal::MissingContent)?;
    let (level, resistance, notice_chance) = (
        u32::from(kind.level),
        monster.resistance(kind, Realm::Mental),
        (u32::from(kind.attributes.senses) + u32::from(kind.attributes.intelligence)) / 2,
    );
    let (mut report, strength) =
        super::sources::cast_source(world, caster, choice.source, UseContext::Conversation)?;
    let mut reply = MagicalReply::Unnoticed;
    match report.outcome {
        Outcome::Fizzled => {}
        Outcome::Backfired => {
            if world.resource_mut::<GameRng>().below(100) < notice_chance {
                reply = MagicalReply::Quote(0x16);
            }
        }
        Outcome::Cast => {
            let chance = super::effects::condition_chance(level, resistance, strength.against, 0);
            let saved = super::effects::save_roll(&mut world.resource_mut::<GameRng>(), chance);
            if id == SpellId::MINDREAD {
                reply = MagicalReply::Quote(if saved {
                    0x69
                } else {
                    let mut npc = world.get_mut::<Npc>(actor).ok_or(Refusal::NoTarget)?;
                    let quote = 0x67 + u16::from(npc.mindread_refusals);
                    npc.mindread_refusals = (npc.mindread_refusals + 1).min(2);
                    quote
                });
            } else if saved {
                report
                    .effects
                    .push(super::Effect::Resisted { victim: super::Victim::Monster(body) });
            } else {
                let amount = u32::from(strength.power) * 7;
                let amount = amount + amount * strength.boost / 100;
                let amount = super::effects::resist(
                    &mut world.resource_mut::<GameRng>(),
                    amount,
                    resistance.min(100),
                    level,
                    strength.against,
                ) as u8 as i8;
                if amount > world.get::<Conversation>(entity).unwrap().charm() {
                    reply = MagicalReply::Charmed(amount);
                }
            }
        }
    }
    let charmed = matches!(reply, MagicalReply::Charmed(_));
    world.get_mut::<Conversation>(entity).unwrap().magical_reply(reply);
    if charmed {
        crate::game::npcs::conversation::won_over(world, entity, true);
    }
    world.write_message(MagicNews::Done(report));
    Ok(())
}
