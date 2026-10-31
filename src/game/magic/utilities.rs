//! Utility targets are validated before the shared cast/payment operation.
use super::casting::{Aim, MagicChoice, MagicNews, Refusal, Source, UseContext};
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::party::roster::{PartyRoster, the_party};
use bevy::prelude::*;

pub(crate) fn spell(id: SpellId) -> bool {
    super::portals::spell(id)
        || super::dialogue::spell(id)
        || super::pointcasts::spell(id)
        || id == SpellId::IDENTIFY_ITEM
}

pub(crate) fn source_spell(
    world: &mut World,
    caster: Entity,
    source: Source,
) -> Result<SpellId, Refusal> {
    let party = the_party(world);
    let inventory = world.get::<PartyInventory>(party).unwrap();
    let items = &world.resource::<Catalogs>().items;
    source.spell_id(world.get::<Carried>(caster).unwrap(), inventory, items).ok_or(Refusal::NoItem)
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
    let score = u32::from(strength.power) * 3;
    score + score * strength.boost / 100 >= u32::from(difficulty)
}

/// Identify Item, outside a fight and while the party is free: success
/// reveals the target's identity, spell and binding, with no item roll.
pub(crate) fn identify_items(
    mut casts: MessageReader<UtilityCast>,
    freedom: crate::game::freedom::Freedom,
    battles: Query<(), With<crate::game::combat::Battle>>,
    roster: Single<&PartyRoster>,
    mut payment: super::casting::Payment,
    mut news: MessageWriter<MagicNews>,
) {
    for &UtilityCast { caster, choice, spell } in casts.read() {
        if spell != SpellId::IDENTIFY_ITEM {
            continue;
        }
        let identified = if !freedom.unoccupied() || !battles.is_empty() {
            Err(Refusal::WrongTime)
        } else {
            identify(&roster, &mut payment, caster, choice)
        };
        news.write(match identified {
            Ok(report) => MagicNews::Done(report),
            Err(refusal) => MagicNews::Refused { caster, refusal },
        });
    }
}

fn identify(
    roster: &PartyRoster,
    payment: &mut super::casting::Payment,
    caster: Entity,
    choice: MagicChoice,
) -> Result<super::casting::Report, Refusal> {
    let Aim::Item(target_id) = choice.aim else { return Err(Refusal::NoTarget) };
    // Search only the current roster and party inventory. Stable stack IDs
    // allow a target to move between those owners without retargeting a
    // different item.
    let target = |payment: &super::casting::Payment| {
        payment.inventory.find_stack(target_id).cloned().or_else(|| {
            roster
                .iter()
                .find_map(|m| payment.members.get(m).unwrap().2.find_stack(target_id).cloned())
        })
    };
    let stack = target(payment).ok_or(Refusal::NoTarget)?;
    let difficulty = payment.content.items[stack.item].identify_difficulty;
    // The source cannot consume its own target and then silently identify a
    // replacement. A fully known stack likewise needs no paid attempt.
    if matches!(choice.source, Source::Item(s) | Source::PoolItem(s) if s == target_id)
        || stack.identified && stack.spell_known && stack.curse_known
    {
        return Err(Refusal::NoTarget);
    }
    let source = if let Source::Spell { spell, power: super::casting::AUTO_POWER } = choice.source {
        let definition = &payment.content.spells[spell];
        let profile = payment.members.get(caster).unwrap().0;
        let power = (1..=crate::content::spells::MAX_POWER)
            .find(|&power| identifies(super::Strength::of(profile, definition, power), difficulty))
            .unwrap_or(crate::content::spells::MAX_POWER);
        Source::Spell { spell, power }
    } else {
        choice.source
    };
    let (report, strength) = payment.pay(caster, source, UseContext::World)?;
    if report.outcome == super::Outcome::Cast && identifies(strength, difficulty) {
        let stack = match payment.inventory.find_stack_mut(target_id) {
            Some(stack) => stack,
            None => {
                let owner = roster
                    .iter()
                    .find(|&m| payment.members.get(m).unwrap().2.find_stack(target_id).is_some())
                    .unwrap();
                payment
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
