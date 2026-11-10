//! Utility magic: what is not a deed on bodies. Each kind's observer takes its
//! own of the accepted casts ([`Utility`]), validates its target, and only
//! then shares the cast's payment and roll ([`super::casting::pay`]).
use super::casting::{Aim, MagicNews, Refusal, Refused, Source, UseContext};
use super::{Outcome, Strength, Utility};
use crate::content::spells::{MAX_POWER, SpellId};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::freedom::Freedom;
use crate::game::hits::effects::Body;
use crate::game::items::{Items, Known, Place};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::level::Level;
use crate::game::world::triggers::LevelState;
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    super::portals::spell(id)
        || super::dialogue::spell(id)
        || super::pointcasts::spell(id)
        || id == SpellId::IDENTIFY_ITEM
}

/// Whether Fire is quenched: the party is under water.
pub(crate) fn underwater(states: &Query<&LevelState, With<Level>>) -> bool {
    states.single().is_ok_and(|state| state.camera_sway)
}

/// Whether an identify at `strength` reveals an item this hard (retail
/// `ApplyIdentifyAttempt`: power × 3, raised by Power Cast).
fn identifies(strength: Strength, difficulty: u8) -> bool {
    let score = strength.power * 3;
    score + score * strength.boost / 100 >= u32::from(difficulty)
}

/// Identify Item on a stack the party holds, wherever it has moved among
/// the members and the pool since it was chosen: an auto cast takes the
/// least power that reveals it. Success reveals its identity, spell and
/// binding; there is no item roll.
pub(crate) fn identify(
    accepted: On<Utility>,
    mut bodies: Query<Body>,
    roster: Single<&PartyRoster>,
    held: Items,
    (freedom, battle): (Freedom, Query<(), With<Battle>>),
    states: Query<&LevelState, With<Level>>,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let Utility { caster, spell, choice } = *accepted;
    if spell != SpellId::IDENTIFY_ITEM {
        return;
    }
    let roster = *roster;
    let mut cast = || -> Result<_, Refusal> {
        if !freedom.unoccupied() || !battle.is_empty() {
            return Err(Refusal::WrongTime);
        }
        let Aim::Item(target) = choice.aim else { return Err(Refusal::NoTarget) };
        // Something the party has: in hand, in the pool, or with a member.
        let known = held
            .place(target)
            .filter(|place| match place {
                Place::Slot { member, .. } | Place::Pack { member, .. } => roster.contains(member),
                _ => true,
            })
            .and(held.known(target))
            .ok_or(Refusal::NoTarget)?;
        let hard = content.items[held.id(target).ok_or(Refusal::NoTarget)?].identify_difficulty;
        // The source cannot consume its own target and then silently
        // identify a replacement. A fully known item likewise needs no
        // paid attempt.
        if choice.source == Source::Item(target) || known.identified && known.spell && known.curse {
            return Err(Refusal::NoTarget);
        }
        let mut body = bodies.get_mut(caster).map_err(|_| Refusal::CannotAct)?;
        let source = match (choice.source, body.character.as_deref()) {
            (Source::Spell { spell, power: super::casting::AUTO_POWER }, Some(profile)) => {
                let definition = &content.spells[spell];
                let power = (1..=MAX_POWER)
                    .find(|&power| identifies(Strength::of(profile, definition, power), hard))
                    .unwrap_or(MAX_POWER);
                Source::Spell { spell, power }
            }
            (source, _) => source,
        };
        let (report, strength) = super::casting::pay(
            &mut body,
            &held,
            (source, UseContext::World),
            (&content, *difficulty, underwater(&states)),
            &mut rng,
            &mut commands,
        )?;
        if report.outcome == Outcome::Cast && identifies(strength, hard) {
            commands.entity(target).insert(Known { identified: true, spell: true, curse: true });
        }
        Ok(report)
    };
    match cast() {
        Ok(report) => {
            news.write(MagicNews::Done(report));
        }
        Err(refusal) => commands.trigger(Refused { caster, refusal }),
    }
}
