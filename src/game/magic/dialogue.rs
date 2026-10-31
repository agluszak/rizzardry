//! Dialogue-only casts validate the live speaker and ready script before the
//! ordinary source transaction spends points, item uses, fatigue or RNG.
use super::{
    Outcome,
    casting::{Aim, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{characters::Realm, spells::SpellId},
    game::npcs::{
        Npc, NpcBodies,
        conversation::{Conversation, MagicalReply, NpcPresence, WonOver},
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
    mut conversations: Query<(Entity, &mut Conversation)>,
    mut npcs: Query<(&mut Npc, Option<&NpcBodies>)>,
    presence: NpcPresence,
    mut payment: super::casting::Payment,
    mut news: MessageWriter<MagicNews>,
    mut won: MessageWriter<WonOver>,
) {
    for &super::utilities::UtilityCast { caster, choice, spell: id } in casts.read() {
        if !spell(id) {
            continue;
        }
        let mut cast = || {
            let Ok((entity, mut conversation)) = conversations.single_mut() else {
                return Err(Refusal::WrongTime);
            };
            if !conversation.ready() {
                return Err(Refusal::WrongTime);
            }
            let (mut npc, bodies) = npcs.get_mut(conversation.actor).unwrap();
            let body = presence.body(bodies).ok_or(Refusal::NoTarget)?;
            if choice.aim != Aim::Monster(body) {
                return Err(Refusal::NoTarget);
            }
            let monster = presence.monster(body);
            let kind = &payment.content.species[monster.species];
            let (level, resistance, notice_chance) = (
                u32::from(kind.level),
                monster.resistance(kind, Realm::Mental),
                (u32::from(kind.attributes.senses) + u32::from(kind.attributes.intelligence)) / 2,
            );
            let (mut report, strength) =
                payment.pay(caster, choice.source, UseContext::Conversation)?;
            let rng = &mut payment.rng;
            let mut reply = MagicalReply::Unnoticed;
            match report.outcome {
                Outcome::Fizzled => {}
                Outcome::Backfired => {
                    if rng.below(100) < notice_chance {
                        reply = MagicalReply::Quote(0x16);
                    }
                }
                Outcome::Cast => {
                    let chance =
                        super::effects::condition_chance(level, resistance, strength.against, 0);
                    let saved = super::effects::save_roll(rng, chance);
                    if id == SpellId::MINDREAD {
                        reply = MagicalReply::Quote(if saved {
                            0x69
                        } else {
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
                            rng,
                            amount,
                            resistance.min(100),
                            level,
                            strength.against,
                        ) as u8 as i8;
                        if amount > conversation.charm() {
                            reply = MagicalReply::Charmed(amount);
                        }
                    }
                }
            }
            let charmed = matches!(reply, MagicalReply::Charmed(_));
            conversation.magical_reply(reply);
            if charmed {
                won.write(WonOver { conversation: entity, greeting: true });
            }
            Ok(report)
        };
        let cast = cast();
        news.write(match cast {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}
