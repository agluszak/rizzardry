//! Breathing: the racial or spell-granted breath a member may take as their
//! action ([`super::member_turn::start_breath`]).
use crate::content::characters::Trait;
use crate::game::actors::Laid;
use crate::game::actors::Traits;
use crate::game::characters::Vitals;
use bevy::prelude::Entity;

/// Whether a member who can fight has a breath to take now: of a breathing
/// race or under Dracon Breath, with a fifth of their stamina.
pub fn available(member: Entity, traits: &Traits, vitals: &Vitals, laid: &Laid) -> bool {
    (traits.has(Trait::Breathe) || laid.bonus(member).breath)
        && vitals.stamina.current >= vitals.stamina.max / 5
}
