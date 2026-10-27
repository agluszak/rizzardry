//! Whether the party is free to act in the world. The screens themselves
//! are presentation; which one is up, and what that holds, is a gameplay
//! fact that presentation routes input by but does not decide.
//!
//! While a full screen (the automap, the inventory), a camp, a travel
//! question or work on a device (a lock, a trap, a container's contents) is
//! up, the party does not move, use, take or rest. Time,
//! monsters and fights go on: no screen pauses the simulation.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::world::level::{Level, TravelQuestion};

/// The full screen that is up; they are mutually exclusive.
#[derive(States, Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    World,
    Automap,
    Inventory,
}

/// What decides whether the party is free to act.
#[derive(SystemParam)]
pub struct Freedom<'w, 's> {
    levels: Query<'w, 's, (), With<Level>>,
    screen: Res<'w, State<Screen>>,
    next: Res<'w, NextState<Screen>>,
    camp: Option<Res<'w, Camp>>,
    questions: Query<'w, 's, (), With<TravelQuestion>>,
    flights: Query<'w, 's, (), With<crate::game::party::walk_in::PathFlight>>,
    interaction: Option<Res<'w, crate::game::world::interaction::DeviceInteraction>>,
}

impl Freedom<'_, '_> {
    pub fn free(&self) -> bool {
        !self.levels.is_empty()
            && *self.screen.get() == Screen::World
            && matches!(*self.next, NextState::Unchanged | NextState::Pending(Screen::World))
            && self.camp.is_none()
            && self.questions.is_empty()
            && self.flights.is_empty()
            && self.interaction.is_none()
    }
}

/// Run condition: the party may move, use, take and rest.
pub fn party_free(freedom: Freedom) -> bool {
    freedom.free()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn an_accepted_screen_change_blocks_world_input_before_state_transition() {
        let mut world = World::new();
        world.insert_resource(State::new(Screen::World));
        world.init_resource::<NextState<Screen>>();
        world.spawn(Level::test(default()));
        assert!(world.run_system_once(party_free).unwrap());
        world.resource_mut::<NextState<Screen>>().set(Screen::Inventory);
        assert_eq!(*world.resource::<State<Screen>>().get(), Screen::World);
        assert!(!world.run_system_once(party_free).unwrap());
    }
}
