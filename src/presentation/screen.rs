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
