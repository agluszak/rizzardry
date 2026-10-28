//! Members' load (`Burden`): the one thing their belongings imply that is
//! kept rather than worked out where it is used, because a fight keeps the
//! loads it began with. Everything else (main hand, resistances, armour,
//! spell-point maximum) is calculated where it is read.

use bevy::prelude::*;

use crate::game::GameplaySet;
use crate::game::combat::Battle;
use crate::game::inventory::distribute_burden;
use crate::game::world::level::level_active;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        // A fight keeps the loads it began with (retail
        // `RedistributePartyEncumbrance` defers while in combat).
        distribute_burden
            .run_if(not(any_with_component::<Battle>))
            .in_set(GameplaySet::Stats)
            .run_if(level_active),
    );
}
