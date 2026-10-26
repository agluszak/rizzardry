//! Whether the party is free to act in the world. The screens themselves
//! are presentation; which one is up, and what that holds, is a gameplay
//! fact that presentation routes input by but does not decide.
//!
//! While a full screen (the automap, the inventory), a camp or a travel
//! question is up, the party does not move, use, take or rest. Time,
//! monsters and fights go on: no screen pauses the simulation.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::world::level::{Departing, Level, TravelQuestion};

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
    levels: Query<'w, 's, (), (With<Level>, Without<Departing>)>,
    screen: Res<'w, State<Screen>>,
    camp: Option<Res<'w, Camp>>,
    questions: Query<'w, 's, (), With<TravelQuestion>>,
}

impl Freedom<'_, '_> {
    pub fn free(&self) -> bool {
        !self.levels.is_empty()
            && *self.screen.get() == Screen::World
            && self.camp.is_none()
            && self.questions.is_empty()
    }
}

/// Run condition: the party may move, use, take and rest.
pub fn party_free(freedom: Freedom) -> bool {
    freedom.free()
}
