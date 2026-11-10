//! Dialogue-only casts validate the live speaker and ready script before the
//! ordinary source transaction spends points, item uses, fatigue or RNG.
use super::utilities::underwater;
use super::{
    Outcome, Utility,
    casting::{Aim, MagicNews, Refusal, Refused, UseContext},
};
use crate::{
    content::{
        characters::{Attribute, Realm},
        spells::SpellId,
    },
    game::{
        data::Catalogs,
        difficulty::Difficulty,
        hits::{Effect, defences, effects::Body},
        items::Items,
        npcs::{
            Npc, NpcBodies,
            conversation::{Conversation, MagicalReply},
        },
        rng::GameRng,
        world::hostility::Stance,
        world::level::{Level, LevelOwned},
        world::population::Monster,
        world::triggers::LevelState,
    },
};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::CHARM | SpellId::MINDREAD)
}

/// Whether the one spoken to saves against Charm or Mindread: the mental
/// save of its level against the cast's strength, scaled for difficulty by
/// its side like any other save (retail `TargetResistsCondition`,
/// `Magic Effects.cpp:461-510`).
fn saves(
    (level, resistance, against): (u32, i32, u32),
    (difficulty, stance): (Difficulty, Stance),
    rng: &mut GameRng,
) -> bool {
    let chance = defences::condition_chance(level, resistance, against, 0);
    defences::save_roll(rng, difficulty.scale(chance, stance))
}

/// Charm or Mindread, cast at the one the party is talking with while they
/// wait for the party's word: a failed save charms them by the cast's
/// amount or opens their mind; a backfire may be noticed.
pub(crate) fn cast(
    accepted: On<Utility>,
    mut conversation: Single<&mut Conversation>,
    mut speakers: Query<(&mut Npc, Option<&NpcBodies>)>,
    monsters: Query<(&Monster, &LevelOwned)>,
    mut bodies: Query<Body>,
    held: Items,
    (levels, states): (Query<(), With<Level>>, Query<&LevelState, With<Level>>),
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let Utility { caster, spell: id, choice } = *accepted;
    if !spell(id) {
        return;
    }
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
        let stance = their.stance();
        let notices =
            (their.attributes[Attribute::Senses] + their.attributes[Attribute::Intelligence]) / 2;
        let (mut report, strength) = super::casting::pay(
            &mut bodies.get_mut(caster).map_err(|_| Refusal::CannotAct)?,
            &held,
            (choice.source, UseContext::Conversation),
            (&content, *difficulty, underwater(&states)),
            0, // a conversation is outside a fight
            &mut rng,
            &mut commands,
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
                let saved =
                    saves((level, resistance, strength.against), (*difficulty, stance), &mut rng);
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
    match cast() {
        Ok(report) => {
            news.write(MagicNews::Done(report));
        }
        Err(refusal) => commands.trigger(Refused { caster, refusal }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save of (10 - 10)*3 + 0 + 90 = 90 percent. A hostile speaker's is
    /// scaled by 3*20/100 on Novice (54) and by 7*20/100 on Expert (126,
    /// held to the most a save may be, 95); a neutral one's, or any on
    /// Normal, is not (90).
    #[test]
    fn a_save_against_charm_is_scaled_for_difficulty_by_the_speakers_side() {
        let mut rng = GameRng::seeded(11);
        let mut rate = |difficulty, stance| {
            (0..4000).filter(|_| saves((10, 90, 10), (difficulty, stance), &mut rng)).count() / 40
        };
        let near = |percent: usize, expected: usize| percent.abs_diff(expected) <= 4;
        assert!(near(rate(Difficulty::Novice, Stance::Hostile), 54));
        assert!(near(rate(Difficulty::Expert, Stance::Hostile), 95));
        assert!(near(rate(Difficulty::Normal, Stance::Hostile), 90));
        assert!(near(rate(Difficulty::Novice, Stance::Neutral), 90));
    }
}
