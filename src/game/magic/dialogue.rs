//! Dialogue-only casts validate the live speaker and ready script before the
//! ordinary source transaction spends points, item uses, fatigue or RNG.
use super::{
    Outcome,
    casting::{Aim, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{characters::Realm, spells::SpellId},
    game::npcs::{
        NpcBodies,
        conversation::{Conversation, MagicalReply},
    },
};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::CHARM | SpellId::MINDREAD)
}

/// Charm and Mindread, cast at the speaker of a conversation ready for it:
/// the speaker may notice a backfire, see through a mindread, or be charmed
/// past what charmed it before, which may win it over.
pub(crate) fn cast_at_speakers(
    mut casts: MessageReader<super::utilities::UtilityCast>,
    mut conversations: Query<&mut Conversation>,
    bodies: Query<Option<&NpcBodies>>,
    mut field: super::Field,
) {
    for &super::utilities::UtilityCast { caster, choice, spell: id } in casts.read() {
        if !spell(id) {
            continue;
        }
        let mut cast = || {
            let Ok(mut conversation) = conversations.single_mut() else {
                return Err(Refusal::WrongTime);
            };
            if !conversation.ready() {
                return Err(Refusal::WrongTime);
            }
            let actor = conversation.actor;
            let body =
                field.monsters.npc_body(bodies.get(actor).unwrap()).ok_or(Refusal::NoTarget)?;
            if choice.aim != Aim::Monster(body) {
                return Err(Refusal::NoTarget);
            }
            let monster = field.monsters.monsters().get(body).unwrap().0;
            let kind = &field.species()[monster.species];
            let (level, resistance, notice_chance) = (
                u32::from(kind.level),
                monster.resistance(kind, Realm::Mental),
                (u32::from(kind.attributes.senses) + u32::from(kind.attributes.intelligence)) / 2,
            );
            let (mut report, strength) =
                field.pay(caster, choice.source, UseContext::Conversation)?;
            let rng = &mut field.rng;
            let mut reply = None;
            match report.outcome {
                Outcome::Fizzled => {}
                Outcome::Backfired => {
                    if rng.below(100) < notice_chance {
                        reply = Some(MagicalReply::Quote(0x16));
                    }
                }
                Outcome::Cast => {
                    let chance =
                        super::effects::condition_chance(level, resistance, strength.against, 0);
                    let saved = super::effects::save_roll(rng, chance);
                    if id == SpellId::MINDREAD {
                        reply = Some(MagicalReply::Quote(if saved {
                            0x69
                        } else {
                            let mut npc = field.monsters.npc_mut(actor);
                            let quote = 0x67 + u16::from(npc.mindread_refusals);
                            npc.mindread_refusals = (npc.mindread_refusals + 1).min(2);
                            quote
                        }));
                    } else if saved {
                        report
                            .effects
                            .push(super::Effect::Resisted { victim: super::Victim::Monster(body) });
                    } else {
                        let amount = strength.power * 7;
                        let amount = amount + amount * strength.boost / 100;
                        let amount = super::effects::resist(
                            rng,
                            amount,
                            resistance.min(100),
                            level,
                            strength.against,
                        );
                        if amount > conversation.charm() {
                            reply = Some(MagicalReply::Charmed(amount));
                        }
                    }
                }
            }
            conversation.magical_reply(reply);
            Ok(report)
        };
        let cast = cast();
        field.news.write(match cast {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}
