//! Utility magic: what is not a deed on bodies. Each kind's system takes its
//! own of the accepted casts ([`Utility`]), validates its target, and only
//! then shares the cast's payment and roll ([`super::casting::pay`]).
use super::casting::{Aim, MagicNews, Refusal, Source, UseContext};
use super::{Outcome, Strength, Utility};
use crate::content::spells::{MAX_POWER, SpellId};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::freedom::Freedom;
use crate::game::hits::effects::Body;
use crate::game::inventory::{ItemStack, PartyInventory};
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
    mut accepted: MessageReader<Utility>,
    mut bodies: Query<Body>,
    party: Single<(&PartyRoster, &mut PartyInventory)>,
    (freedom, battle): (Freedom, Query<(), With<Battle>>),
    states: Query<&LevelState, With<Level>>,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let (roster, mut inventory) = party.into_inner();
    for &Utility { caster, choice, .. } in
        accepted.read().filter(|cast| cast.spell == SpellId::IDENTIFY_ITEM)
    {
        let mut cast = || -> Result<_, Refusal> {
            if !freedom.unoccupied() || !battle.is_empty() {
                return Err(Refusal::WrongTime);
            }
            let Aim::Item(target) = choice.aim else { return Err(Refusal::NoTarget) };
            let holder = roster.iter().find(|&member| {
                bodies
                    .get(member)
                    .is_ok_and(|body| body.carried.is_some_and(|c| c.find_stack(target).is_some()))
            });
            let stack = inventory
                .find_stack(target)
                .or_else(|| bodies.get(holder?).ok()?.carried?.find_stack(target))
                .ok_or(Refusal::NoTarget)?;
            let hard = content.items[stack.item].identify_difficulty;
            // The source cannot consume its own target and then silently
            // identify a replacement. A fully known stack likewise needs no
            // paid attempt.
            if matches!(choice.source, Source::Item(s) | Source::PoolItem(s) if s == target)
                || stack.identified && stack.spell_known && stack.curse_known
            {
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
                &mut inventory,
                (source, UseContext::World),
                (&content, *difficulty, underwater(&states)),
                &mut rng,
                &mut commands,
            )?;
            if report.outcome == Outcome::Cast && identifies(strength, hard) {
                let reveal = |stack: &mut ItemStack| {
                    stack.identified = true;
                    stack.spell_known = true;
                    stack.curse_known = true;
                };
                if let Some(stack) = inventory.find_stack_mut(target) {
                    reveal(stack);
                } else if let Some(mut holder) = holder.and_then(|h| bodies.get_mut(h).ok())
                    && let Some(stack) =
                        holder.carried.as_mut().and_then(|c| c.find_stack_mut(target))
                {
                    reveal(stack);
                }
            }
            Ok(report)
        };
        news.write(match cast() {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}
