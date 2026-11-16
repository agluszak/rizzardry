//! A member's breath: of a breathing race, or under Dracon Breath.
use bevy::prelude::*;

use crate::content::characters::Trait;
use crate::game::actors::{Modifiers, Traits};
use crate::game::characters::Vitals;

/// It has a breath to take now: of a breathing race or under Dracon Breath,
/// with a fifth of its stamina.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CanBreathe;

/// Having a breath to take follows the race, the breath laid and stamina.
pub fn ready(
    members: Query<(Entity, &Traits, &Modifiers, &Vitals, Has<CanBreathe>)>,
    mut commands: Commands,
) {
    for (member, traits, modifiers, vitals, was) in &members {
        let now = (traits.has(Trait::Breathe) || modifiers.laid.breath)
            && vitals.stamina.current >= vitals.stamina.max / 5;
        if now && !was {
            commands.entity(member).insert(CanBreathe);
        } else if !now && was {
            commands.entity(member).remove::<CanBreathe>();
        }
    }
}
