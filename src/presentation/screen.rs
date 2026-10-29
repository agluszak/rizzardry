//! The full screen that is up. Which one is drawn routes input: the world
//! takes it only while no other screen is up or about to be, and the game
//! itself leaves the party free ([`Freedom`]). No screen pauses the game.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::freedom::Freedom;

/// The full screen that is up; they are mutually exclusive.
#[derive(States, Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    World,
    Automap,
    Inventory,
    Journal,
    Options,
}

/// Whether the world takes the player's input: the world screen is up and
/// staying up, and the party is free.
#[derive(SystemParam)]
pub struct WorldInput<'w, 's> {
    freedom: Freedom<'w, 's>,
    screen: Res<'w, State<Screen>>,
    next: Res<'w, NextState<Screen>>,
}

impl WorldInput<'_, '_> {
    pub fn free(&self) -> bool {
        *self.screen.get() == Screen::World
            && matches!(*self.next, NextState::Unchanged | NextState::Pending(Screen::World))
            && self.freedom.free()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::world::level::Level;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn an_accepted_screen_change_blocks_world_input_before_state_transition() {
        let mut world = World::new();
        world.insert_resource(State::new(Screen::World));
        world.init_resource::<NextState<Screen>>();
        world.spawn(Level::test(default()));
        let free = |input: WorldInput| input.free();
        assert!(world.run_system_once(free).unwrap());
        world.resource_mut::<NextState<Screen>>().set(Screen::Inventory);
        assert_eq!(*world.resource::<State<Screen>>().get(), Screen::World);
        assert!(!world.run_system_once(free).unwrap());
        assert!(
            world.run_system_once(crate::game::freedom::party_free).unwrap(),
            "the game itself does not care which screen is up"
        );
    }
}
