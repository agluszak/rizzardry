//! Whether the party is free to act in the world: in a played level, not
//! entering another, and held by no camp, conversation, travel question,
//! path flight or work on a device. Then the party does not move, use, take
//! or rest. Time, monsters and fights go on regardless.
//!
//! Which screen is drawn is presentation's to route input by
//! (`presentation::screen`); it is not a rule of the game.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::world::level::{Destination, Level, TravelQuestion};

/// What decides whether the party is free to act.
#[derive(SystemParam)]
pub struct Freedom<'w, 's> {
    levels: Query<'w, 's, (), With<Level>>,
    entering: Query<'w, 's, (), With<Destination>>,
    camp: Option<Res<'w, Camp>>,
    conversations: Query<
        'w,
        's,
        (),
        (
            With<crate::game::npcs::conversation::Conversation>,
            Without<crate::game::npcs::conversation::Closing>,
        ),
    >,
    questions: Query<'w, 's, (), With<TravelQuestion>>,
    flights: Query<'w, 's, (), With<crate::game::party::walk_in::PathFlight>>,
    interaction: Query<'w, 's, (), With<crate::game::world::interaction::DeviceInteraction>>,
}

impl Freedom<'_, '_> {
    pub fn free(&self) -> bool {
        self.unoccupied() && self.camp.is_none()
    }

    /// Entering no level and held by no travel question or conversation:
    /// what views that open over the world wait for.
    pub fn undisturbed(&self) -> bool {
        self.entering.is_empty() && self.conversations.is_empty() && self.questions.is_empty()
    }

    /// Free but perhaps camping: what utility magic needs (a camp does not
    /// stop a portal).
    pub fn unoccupied(&self) -> bool {
        !self.levels.is_empty()
            && self.undisturbed()
            && self.flights.is_empty()
            && self.interaction.is_empty()
    }
}

/// Run condition: the party may move, use, take and rest.
pub fn party_free(freedom: Freedom) -> bool {
    freedom.free()
}

/// Whether the party is free, for operations that hold the whole world.
pub fn free_in(world: &mut World) -> bool {
    world.run_system_cached(party_free).unwrap_or(false)
}

/// Run condition: free but perhaps camping ([`Freedom::unoccupied`]).
pub fn unoccupied(freedom: Freedom) -> bool {
    freedom.unoccupied()
}
