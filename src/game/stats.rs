//! Burden is accepted when belongings are distributed and remains frozen
//! during combat. Other character stats are calculated from their live owners.
use crate::game::{
    GameplaySet, combat::Battle, inventory::distribute_burden, world::level::level_active,
};
use bevy::prelude::*;
pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        distribute_burden
            .run_if(not(any_with_component::<Battle>))
            .in_set(GameplaySet::Stats)
            .run_if(level_active),
    );
}
