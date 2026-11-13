//! What every actor is, a party member or a monster alike
//! (`docs/rules/actors.md`).

pub mod arms;
pub mod figures;
pub mod fitness;
pub mod modifiers;
pub mod place;
pub mod sources;
pub mod spells;
mod stats;
pub mod time;

pub use fitness::{CanAct, CanFight, CanGuard, Lucid, Settle, Written, settle};
pub use modifiers::Modifiers;
pub use place::{Eyes, Hidden, Place, Places};
pub use sources::*;
pub use spells::{Enchantment, Laid, Laying};
pub use stats::*;
pub use time::{Mending, Swallowed};

use bevy::prelude::*;

pub(crate) fn plugin(app: &mut App) {
    app.add_plugins((fitness::plugin, time::plugin, modifiers::plugin))
        .add_observer(spells::age)
        .add_observer(spells::age_the_woken)
        .add_message::<crate::game::characters::Improved>()
        .add_observer(crate::game::characters::practise)
        .add_observer(crate::game::characters::fall)
        .add_observer(crate::game::party::fall)
        .add_observer(spells::lift_from_the_dead)
        .add_observer(spells::end_fight);
}
