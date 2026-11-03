//! Breathing: the racial or spell-granted breath a member may take as their
//! action ([`super::member_turn::start_breath`]).
use crate::content::{characters::Trait, spells::SpellId};
use crate::game::characters::{Character, Vitals};

/// Whether a member can breathe now: able, of a breathing race or under
/// Dracon Breath, with a fifth of their stamina.
pub fn available(profile: &Character, vitals: &Vitals) -> bool {
    vitals.can_fight()
        && (profile.has_trait(Trait::Breathe) || vitals.enchantments.has(SpellId::DRACON_BREATH))
        && vitals.stamina.current >= vitals.stamina.max / 5
}
