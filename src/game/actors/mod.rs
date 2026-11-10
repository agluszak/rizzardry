//! What every actor is, a party member or a monster alike
//! (`docs/rules/actors.md`).

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
        .add_message::<crate::game::party::recovery::Elapsed>()
        .add_observer(stats::refit_joined)
        .add_observer(stats::refit_left)
        .add_observer(spells::laid)
        .add_observer(spells::lifted)
        .add_observer(spells::lift_from_the_dead)
        .add_observer(spells::end_fight)
        .add_systems(Update, stats::refit_regeared.in_set(crate::game::GameplaySet::Bodies));
}
