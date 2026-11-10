//! Dialogue-only casts validate the live speaker and ready script before the
//! ordinary source transaction spends points, item uses, fatigue or RNG.
use super::utilities::underwater;
use super::{
    Outcome, Utility,
    casting::{Aim, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{
        characters::{Attribute, Realm},
        spells::SpellId,
    },
    game::{
        combat::CombatNews,
        data::Catalogs,
        difficulty::Difficulty,
        hits::{Effect, defences, effects::Body},
        inventory::PartyInventory,
        npcs::{
            Npc, NpcBodies,
            conversation::{Conversation, MagicalReply},
        },
        rng::GameRng,
        world::level::{Level, LevelOwned},
        world::population::Monster,
        world::triggers::LevelState,
    },
};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::CHARM | SpellId::MINDREAD)
}

/// Charm or Mindread, cast at the one the party is talking with while they
/// wait for the party's word: a failed save charms them by the cast's
/// amount or opens their mind; a backfire may be noticed.
pub(crate) fn cast(
    mut accepted: MessageReader<Utility>,
    mut conversation: Single<&mut Conversation>,
    mut speakers: Query<(&mut Npc, Option<&NpcBodies>)>,
    monsters: Query<(&Monster, &LevelOwned)>,
    mut bodies: Query<Body>,
    mut inventory: Single<&mut PartyInventory>,
    (levels, states): (Query<(), With<Level>>, Query<&LevelState, With<Level>>),
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    (mut news, mut improved): (MessageWriter<MagicNews>, MessageWriter<CombatNews>),
) {
    for &Utility { caster, spell: id, choice } in accepted.read().filter(|cast| spell(cast.spell)) {
        let mut cast = || -> Result<_, Refusal> {
            if !conversation.ready() {
                return Err(Refusal::WrongTime);
            }
            let (mut npc, their_bodies) =
                speakers.get_mut(conversation.actor).map_err(|_| Refusal::NoTarget)?;
            // The speaker's living body in the level played, as dialogues
            // read it.
            let body = their_bodies
                .and_then(|theirs| {
                    theirs.iter().find(|&body| {
                        monsters.get(body).is_ok_and(|(_, level)| levels.contains(level.0))
                            && bodies.get(body).is_ok_and(|body| !body.vitals.is_dead())
                    })
                })
                .filter(|&body| choice.aim == Aim::Body(body))
                .ok_or(Refusal::NoTarget)?;
            let their = bodies.get(body).unwrap();
            let (level, resistance) = (their.level.0, their.resistances.realms[Realm::Mental]);
            let notices = (their.attributes[Attribute::Senses]
                + their.attributes[Attribute::Intelligence])
                / 2;
            let (mut report, strength) = super::casting::pay(
                &mut bodies.get_mut(caster).map_err(|_| Refusal::CannotAct)?,
                &mut inventory,
                (choice.source, UseContext::Conversation),
                (&content, *difficulty, underwater(&states)),
                &mut rng,
                &mut improved,
            )?;
            let mut reply = MagicalReply::Unnoticed;
            match report.outcome {
                Outcome::Fizzled => {}
                Outcome::Backfired => {
                    if rng.below(100) < notices {
                        reply = MagicalReply::Quote(0x16);
                    }
                }
                Outcome::Cast => {
                    let chance = defences::condition_chance(level, resistance, strength.against, 0);
                    let saved = defences::save_roll(&mut rng, chance);
                    if id == SpellId::MINDREAD {
                        reply = MagicalReply::Quote(if saved {
                            0x69
                        } else {
                            let quote = 0x67 + u16::from(npc.mindread_refusals);
                            npc.mindread_refusals = (npc.mindread_refusals + 1).min(2);
                            quote
                        });
                    } else if saved {
                        report.effects.push(Effect::Resisted { victim: body });
                    } else {
                        let amount = strength.power * 7;
                        let amount = defences::resist(
                            &mut rng,
                            amount + amount * strength.boost / 100,
                            resistance.min(100),
                            level,
                            strength.against,
                        );
                        if amount > conversation.charm() {
                            reply = MagicalReply::Charmed(amount);
                        }
                    }
                }
            }
            // An NPC won over leaves its menu as its script next runs.
            conversation.magical_reply(reply);
            Ok(report)
        };
        news.write(match cast() {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}
