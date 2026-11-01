//! Utility targets are validated before the shared cast/payment operation.
use super::casting::{Aim, MagicChoice, MagicNews, Refusal, Source, UseContext};
use crate::content::spells::SpellId;
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    super::portals::spell(id)
        || super::dialogue::spell(id)
        || super::pointcasts::spell(id)
        || id == SpellId::IDENTIFY_ITEM
}

/// A member casts a utility spell, carried out by the system for its kind.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct UtilityCast {
    pub caster: Entity,
    pub choice: MagicChoice,
    pub spell: SpellId,
}

/// Whether an identify at `strength` reveals an item this hard (retail
/// `ApplyIdentifyAttempt`: power × 3, raised by Power Cast).
fn identifies(strength: super::Strength, difficulty: u8) -> bool {
    let score = strength.power * 3;
    score + score * strength.boost / 100 >= u32::from(difficulty)
}

/// Identify Item, outside a fight and while the party is free: success
/// reveals the target's identity, spell and binding, with no item roll.
pub(crate) fn identify_items(
    mut casts: MessageReader<UtilityCast>,
    freedom: crate::game::freedom::Freedom,
    mut field: super::Field,
) {
    for &UtilityCast { caster, choice, spell } in casts.read() {
        if spell != SpellId::IDENTIFY_ITEM {
            continue;
        }
        let identified = if !freedom.unoccupied() || freedom.fighting() {
            Err(Refusal::WrongTime)
        } else {
            identify(&mut field, caster, choice)
        };
        field.news.write(match identified {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}

fn identify(
    field: &mut super::Field,
    caster: Entity,
    choice: MagicChoice,
) -> Result<super::casting::Report, Refusal> {
    let Aim::Item(target_id) = choice.aim else { return Err(Refusal::NoTarget) };
    // Search only the current roster and party inventory. Stable stack IDs
    // allow a target to move between those owners without retargeting a
    // different item.
    let stack = field
        .inventory
        .find_stack(target_id)
        .or_else(|| {
            field.party.iter().find_map(|m| field.members.get(m).unwrap().2.find_stack(target_id))
        })
        .ok_or(Refusal::NoTarget)?;
    let difficulty = field.items()[stack.item].identify_difficulty;
    // The source cannot consume its own target and then silently identify a
    // replacement. A fully known stack likewise needs no paid attempt.
    if matches!(choice.source, Source::Item(s) | Source::PoolItem(s) if s == target_id)
        || stack.identified && stack.spell_known && stack.curse_known
    {
        return Err(Refusal::NoTarget);
    }
    let source = if let Source::Spell { spell, power: super::casting::AUTO_POWER } = choice.source {
        let definition = &field.spells()[spell];
        let profile = field.members.get(caster).unwrap().0;
        let power = (1..=crate::content::spells::MAX_POWER)
            .find(|&power| identifies(super::Strength::of(profile, definition, power), difficulty))
            .unwrap_or(crate::content::spells::MAX_POWER);
        Source::Spell { spell, power }
    } else {
        choice.source
    };
    let (report, strength) = field.pay(caster, source, UseContext::World)?;
    if report.outcome == super::Outcome::Cast && identifies(strength, difficulty) {
        let stack = match field.inventory.find_stack_mut(target_id) {
            Some(stack) => stack,
            None => {
                let owner = field
                    .party
                    .iter()
                    .find(|&m| field.members.get(m).unwrap().2.find_stack(target_id).is_some())
                    .unwrap();
                field
                    .members
                    .get_mut(owner)
                    .unwrap()
                    .2
                    .into_inner()
                    .find_stack_mut(target_id)
                    .unwrap()
            }
        };
        stack.identified = true;
        stack.spell_known = true;
        stack.curse_known = true;
    }
    Ok(report)
}
