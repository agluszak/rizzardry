//! Walking into positioned triggers, and moving the party where teleports
//! send it.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::body::{EYE_ABOVE_CENTRE, PartyBody};
use crate::content::level::Arrival;
use crate::game::world::level::Travel;
use crate::game::world::triggers::LevelTriggers;

pub fn run_walk_in_triggers(party: Query<&Position, With<PartyBody>>, mut triggers: LevelTriggers) {
    let Ok(position) = party.single() else { return };
    let eye = position.0 + Vec3::Y * EYE_ABOVE_CENTRE;
    triggers.with(|triggers| triggers.walk(eye));
}

/// Teleport the party within the level, or travel to another level's
/// entrance.
pub fn move_party(
    mut party: Query<(&mut PartyBody, &mut Position, &mut Transform)>,
    mut travel: MessageWriter<Travel>,
    mut triggers: LevelTriggers,
) {
    let Some(arrival) = triggers.take_party_move() else { return };
    match arrival {
        Arrival::Here { eye, yaw } => {
            let Ok((mut body, mut position, mut transform)) = party.single_mut() else { return };
            // Same-level destinations place the eye at the trigger, unprobed.
            position.0 = eye - Vec3::Y * EYE_ABOVE_CENTRE;
            transform.translation = position.0;
            body.yaw = yaw;
            info!("teleport to {eye:.2}");
            triggers.with(|triggers| triggers.arrive(eye));
        }
        Arrival::Entrance(entrance) => {
            travel.write(Travel { entrance });
        }
    }
}
