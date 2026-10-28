//! Members' derived state that is kept rather than worked out where it is
//! used: their load (`Burden`, frozen while a fight is on) and their
//! spell-point maximum. Everything else a member's profile and belongings
//! imply (main hand, resistances, armour) is calculated where it is read.

use bevy::prelude::*;

use crate::game::GameplaySet;
use crate::game::combat::Battle;
use crate::game::inventory::distribute_burden;
use crate::game::magic::sync_spell_points;
use crate::game::world::level::level_active;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            // A fight keeps the loads it began with (retail
            // `RedistributePartyEncumbrance` defers while in combat).
            distribute_burden.run_if(not(any_with_component::<Battle>)),
            sync_spell_points,
        )
            .chain()
            .in_set(GameplaySet::Stats)
            .run_if(level_active),
    );
}
