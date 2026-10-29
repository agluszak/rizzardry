//! Burden is historical during combat: retail keeps the loads a fight began
//! with. Other derived member statistics are calculated where they are read.

use bevy::prelude::*;

use crate::game::GameplaySet;
use crate::game::combat::Battle;
use crate::game::inventory::distribute_burden;
use crate::game::world::level::level_active;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        distribute_burden
            .run_if(not(any_with_component::<Battle>))
            .in_set(GameplaySet::Stats)
            .run_if(level_active),
    );
}
