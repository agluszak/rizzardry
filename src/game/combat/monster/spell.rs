//! A monster choosing a spell (`docs/rules/monsters.md` section 5): which of
//! its ten slots it may cast, at whom each may be aimed, whether it casts,
//! which spell, and how hard.
//!
//! A monster spends nothing to cast: its spell budget stands in for a caster's
//! skill in the failure roll, and the power it picks is the most that budget
//! comfortably covers.

use bevy::prelude::*;

use super::aim::{AimsMade, At, By, Spoiled, Weigh, Worth};
use super::slot::{Breath, Castable, SlotOf, Slots, Weight};
use super::{ClosesIn, Deliberating, PartyGap};
use crate::content::spells::{MAX_POWER, SpellDefinition};
use crate::game::actors::Level;
use crate::game::actors::arm::Furthest;
use crate::game::combat::turn::{Decided, Decision};
use crate::game::data::Catalogs;
use crate::game::magic::casting::{MagicChoice, Source};
use crate::game::magic::reach::Aim;
use crate::game::magic::roll::spell_failure;
use crate::game::rng::{GameRng, weighted};
use crate::game::world::population::{CastChance, SpellBudget};

/// The deliberation casts none of its spells.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NoSpell;

/// A deliberation picks among the slots it weighed.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct PicksSpell {
    #[event_target]
    deliberation: Entity,
}

/// A deliberation that made no special attack weighs each slot that may be
/// cast now (section 5), unless it closes in or never casts; then it picks
/// among them.
pub(super) fn weigh_spells(
    weighed: On<Add<super::special::NoSpecial>>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &ClosesIn)>,
    casters: Query<(&CastChance, &Slots)>,
    castable: Query<(), (With<Castable>, Without<Breath>)>,
) {
    let deliberation = weighed.entity;
    let Ok((&Deliberating(monster), &ClosesIn(closes))) = deliberations.get(deliberation) else {
        return;
    };
    let Ok((chance, slots)) = casters.get(monster) else {
        commands.entity(deliberation).insert(NoSpell);
        return;
    };
    if closes || chance.0 == 0 {
        commands.entity(deliberation).insert(NoSpell);
        return;
    }
    for slot in slots.iter().filter(|&slot| castable.contains(slot)) {
        commands.trigger(Weigh { slot, considering: deliberation });
    }
    commands.trigger(PicksSpell { deliberation });
}

/// Whether it casts, and what (section 5): its chance (the species' while
/// one of its attacks reaches the party, 100 when none does), rolled once if
/// any slot weighed has an aim worth casting that spoils nothing; then one of
/// those slots by their weights, at one of its aims at random, at the power
/// its budget covers.
#[allow(clippy::type_complexity)]
fn pick(
    picks: On<PicksSpell>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &PartyGap, Option<&AimsMade>)>,
    aims: Query<(&By, &At), (With<Worth>, Without<Spoiled>)>,
    slots: Query<(&super::slot::Casts, &Weight), (With<SlotOf>, Without<Breath>)>,
    casters: Query<(&CastChance, &Furthest, &Level, &SpellBudget)>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
) {
    let deliberation = picks.deliberation;
    let Ok((&Deliberating(monster), &PartyGap(gap), made)) = deliberations.get(deliberation) else {
        return;
    };
    let Ok((chance, reach, level, &SpellBudget(budget))) = casters.get(monster) else {
        commands.entity(deliberation).insert(NoSpell);
        return;
    };
    // Each slot with a good aim, in the order its aims were made, which is
    // the order of the slots.
    let mut usable: Vec<(Entity, Vec<Aim>)> = Vec::new();
    for (&By(slot), &At(at)) in
        aims.iter_many(made.into_iter().flat_map(|made| made.iter())).flatten()
    {
        if !slots.contains(slot) {
            continue;
        }
        match usable.iter_mut().find(|(weighed, _)| *weighed == slot) {
            Some((_, aims)) => aims.push(at),
            None => usable.push((slot, vec![at])),
        }
    }
    let chance = if reach.reaches(gap) { u32::from(chance.0) } else { 100 };
    if usable.is_empty() || rng.below(100) >= chance {
        commands.entity(deliberation).insert(NoSpell);
        return;
    }
    let weights = || usable.iter().filter_map(|(slot, _)| slots.get(*slot).ok()).map(|s| s.1.0);
    let which = weighted(rng.below(weights().sum()), weights()).unwrap_or(usable.len() - 1);
    let (slot, ref aims) = usable[which];
    let Ok((&super::slot::Casts(spell), _)) = slots.get(slot) else { return };
    let aim = aims[rng.below(aims.len() as u32) as usize];
    let power = power(&mut rng, budget, &content.spells[spell]);
    let source = Source::Innate { spell, power, budget, level: level.0 as u8 };
    commands.trigger(Decided {
        actor: monster,
        decision: Decision::Magic(MagicChoice { source, aim }),
        own_will: true,
    });
    commands.entity(deliberation).despawn();
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(weigh_spells).add_observer(pick);
}

/// The power a budget comfortably covers: up from 1 while the spell's
/// failure chance at that power with the budget as skill is under 9 percent
/// and the power is under 8; then one less if above 1.
pub fn comfortable(budget: u32, spell: &SpellDefinition) -> u32 {
    let mut power = 1;
    while power < 8 && spell_failure(budget, spell, power) < 9 {
        power += 1;
    }
    if power > 1 { power - 1 } else { power }
}

/// How hard it casts (retail `ChooseMonsterSpellPowerLevel`): the power its
/// budget comfortably covers; then below(4): on 0 one less if above 2, on 1
/// one more if under 7.
pub fn power(rng: &mut GameRng, budget: u32, spell: &SpellDefinition) -> u32 {
    let power = comfortable(budget, spell);
    match rng.below(4) {
        0 if power > 2 => power - 1,
        1 if power < MAX_POWER => power + 1,
        _ => power,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::Reach;
    use crate::content::spells::{SpellId, Targeting};

    fn spell(id: u8, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id: SpellId(id),
            targeting,
            level: 1,
            cost: 2,
            monsters_cast: true,
            reach: Reach::Long,
            ..Default::default()
        }
    }

    // Power: failure of the test spell (cost 2, level 1: band 2, threshold 50) at
    // power p is need = 50 * p / 7 = 7, 14, 21, 28, 35, 42, 50 for 1 to 7, and
    // (need - budget) * 70 / need when the budget is short of it.

    fn powers(budget: u32) -> Vec<u32> {
        let blast = spell(1, Targeting::Enemy);
        let mut rng = GameRng::seeded(4);
        let mut got: Vec<u32> = (0..200).map(|_| power(&mut rng, budget, &blast)).collect();
        got.sort_unstable();
        got.dedup();
        got
    }

    #[test]
    fn a_middling_budget_casts_one_below_the_last_power_it_covers_give_or_take_one() {
        // Budget 30: powers 1-4 need 7, 14, 21, 28, all met; power 5 needs 35:
        // (35 - 30) * 70 / 35 = 10, not under 9. So 5, then 4; roll 0 -> 3,
        // roll 1 -> 5, else 4.
        assert_eq!(powers(30), [3, 4, 5]);
        // Budget 31: (35 - 31) * 70 / 35 = 8 is under 9: power 6 is tried; it
        // needs 42: (42 - 31) * 70 / 42 = 18: stops at 6, then 5; 4, 5 or 6.
        assert_eq!(powers(31), [4, 5, 6]);
    }
}
