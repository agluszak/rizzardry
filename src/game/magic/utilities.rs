//! Utility targets are validated before the shared cast/payment operation.
use super::casting::{Aim, MagicChoice, MagicNews, Refusal, Source, UseContext};
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::inventory::{Carried, ItemStack, PartyInventory, StackId};
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

/// Search only the current roster and party inventory. Stable stack IDs allow
/// a target to move between those owners without retargeting a different item.
fn target(world: &World, party: Entity, id: StackId) -> Option<&ItemStack> {
    world.get::<PartyInventory>(party).unwrap().find_stack(id).or_else(|| {
        world
            .get::<PartyRoster>(party)
            .unwrap()
            .iter()
            .find_map(|member| world.get::<Carried>(member).unwrap().find_stack(id))
    })
}

/// Whether an identify at `strength` reveals an item this hard (retail
/// `ApplyIdentifyAttempt`: power × 3, raised by Power Cast).
fn identifies(strength: super::Strength, difficulty: u8) -> bool {
    let score = strength.power * 3;
    score + score * strength.boost / 100 >= u32::from(difficulty)
}

pub(crate) fn identify(
    world: &mut World,
    caster: Entity,
    choice: MagicChoice,
) -> Result<(), Refusal> {
    if !world.run_system_cached(crate::game::freedom::unoccupied).unwrap_or(false)
        || world.query::<&crate::game::combat::Battle>().iter(world).next().is_some()
    {
        return Err(Refusal::WrongTime);
    }
    let Aim::Item(target_id) = choice.aim else { return Err(Refusal::NoTarget) };
    let party = the_party(world);
    let stack = target(world, party, target_id).ok_or(Refusal::NoTarget)?;
    let definition = &world.resource::<Catalogs>().items[stack.item];
    let difficulty = definition.identify_difficulty;
    // The source cannot consume its own target and then silently identify a
    // replacement. A fully known stack likewise needs no paid attempt.
    if matches!(choice.source, Source::Item(s) | Source::PoolItem(s) if s == target_id)
        || stack.identified && stack.spell_known && stack.curse_known
    {
        return Err(Refusal::NoTarget);
    }
    let source = if let Source::Spell { spell, power: super::casting::AUTO_POWER } = choice.source {
        let spells = &world.resource::<Catalogs>().spells;
        let definition = &spells[spell];
        let profile = world.get::<crate::game::characters::Character>(caster).unwrap();
        let power = (1..=crate::content::spells::MAX_POWER)
            .find(|&power| identifies(super::Strength::of(profile, definition, power), difficulty))
            .unwrap_or(crate::content::spells::MAX_POWER);
        Source::Spell { spell, power }
    } else {
        choice.source
    };
    let (report, strength) = super::casting::cast_source(world, caster, source, UseContext::World)?;
    // No item roll or extra RNG draw; success reveals identity, spell and
    // binding.
    if report.outcome == super::Outcome::Cast && identifies(strength, difficulty) {
        let reveal = |stack: &mut ItemStack| {
            stack.identified = true;
            stack.spell_known = true;
            stack.curse_known = true;
        };
        let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
        if let Some(stack) = inventory.find_stack_mut(target_id) {
            reveal(stack);
        } else {
            let owner = world
                .get::<PartyRoster>(party)
                .unwrap()
                .iter()
                .find(|&member| {
                    world.get::<Carried>(member).unwrap().find_stack(target_id).is_some()
                })
                .unwrap();
            reveal(world.get_mut::<Carried>(owner).unwrap().find_stack_mut(target_id).unwrap());
        }
    }
    world.write_message(MagicNews::Done(report));
    Ok(())
}
