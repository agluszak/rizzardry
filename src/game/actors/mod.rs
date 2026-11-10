//! What every actor is, a party member or a monster alike
//! (`docs/rules/actors.md`).

pub mod arms;
pub mod place;
pub mod spells;
mod stats;

pub use place::{Eyes, Hidden, Place, Places};
pub use spells::{Enchantment, Laid, Laying};
pub use stats::*;

use bevy::prelude::*;

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(stats::refit)
        .add_observer(stats::refit_made::<crate::game::characters::Character>)
        .add_observer(stats::refit_made::<crate::game::world::population::Monster>)
        .add_observer(crate::game::party::recovery::pass_time)
        .add_observer(spells::age)
        .add_observer(age_the_woken)
        .add_message::<crate::game::characters::Improved>()
        .add_observer(crate::game::characters::practise)
        .add_observer(crate::game::characters::fall)
        .add_observer(stats::refit_joined)
        .add_observer(stats::refit_left)
        .add_observer(spells::laid)
        .add_observer(spells::lifted)
        .add_observer(spells::lift_from_the_dead)
        .add_observer(spells::end_fight);
}

/// A level that was away owes those in it the time that passed: what ails
/// them runs its course and the spells laid on them wear, by the aging ticks
/// that went by.
pub(crate) fn age_the_woken(
    woke: On<crate::game::world::level::Woke>,
    belongings: crate::game::world::level::Belongings,
    mut actors: Query<&mut crate::game::characters::Vitals>,
    mut laid: Query<&mut spells::Turns>,
    mut rng: ResMut<crate::game::rng::GameRng>,
    mut commands: Commands,
) {
    let ticks = (woke.away_ms / crate::game::time::AGING_TICK_MS) as u32;
    if ticks == 0 {
        return;
    }
    for entity in belongings.of(woke.level) {
        if let Ok(mut vitals) = actors.get_mut(entity)
            && vitals.conditions.any_timed()
        {
            vitals.age_conditions(ticks, &mut rng);
        }
        if let Ok(mut turns) = laid.get_mut(entity) {
            turns.wear(entity, ticks, &mut commands);
        }
    }
}
