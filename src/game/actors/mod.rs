//! What every actor is, a party member or a monster alike
//! (`docs/rules/actors.md`).

pub mod arm;
pub mod armour;

pub mod figures;
pub mod fitness;
pub mod modifiers;
pub mod place;
pub mod sources;
pub mod spells;
mod stats;
pub mod time;

pub use armour::{
    Against, Armour, ArmourPart, ArmourParts, Block, HitPlace, HitPlaces, Unaware, Unseated,
    VulnerableFromBehind,
};
pub use fitness::{CanAct, CanFight, CanGuard, Lucid, Settle, Written, settle};
pub use modifiers::Modifiers;
pub use place::{Eyes, Hidden, Place, Places};
pub use sources::*;
pub use spells::{Enchantment, Laid, Laying};
pub use stats::*;
pub use time::{Mending, Swallowed};

use bevy::prelude::*;

pub fn plugin(app: &mut App) {
    // The spells laid on a body are set aside with it.
    crate::game::world::aside::follows::<spells::LaidOn>(app);
    app.add_plugins((
        fitness::plugin,
        time::plugin,
        modifiers::plugin,
        armour::plugin,
        arm::plugin,
    ))
    .add_observer(spells::age)
    .add_observer(spells::age_the_woken)
    .add_message::<crate::game::characters::Improved>()
    .add_observer(crate::game::characters::practise)
    .add_observer(crate::game::characters::fall)
    .add_plugins(crate::game::characters::draft::plugin)
    .add_observer(crate::game::party::fall)
    .add_observer(spells::lift_from_the_dead)
    .add_observer(spells::end_fight);
}
