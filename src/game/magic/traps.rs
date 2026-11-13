//! Indirect trap spells: a sprung trap's spell lands on members of the
//! party as a deed with no caster.

use super::power;
use super::workings;
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::hits::Deed;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::Strike;
use crate::game::rng::GameRng;
use crate::game::world::interaction::{Unable, trap_kind};
use bevy::prelude::*;

/// The spell a kind of trap discharges, if the game can resolve it.
fn trap_spell(trap: u8, content: &Catalogs) -> Result<SpellId, Unable> {
    let id = trap_kind(trap).spell;
    if content.spell_row(id).is_none() {
        return Err(Unable::UnsupportedTrap);
    }
    Ok(id)
}

/// No attempted inspection/disarm may roll if its failure cannot be resolved.
pub(crate) fn validate(trap: u8, content: &Catalogs) -> Result<(), Unable> {
    trap_spell(trap, content).map(|_| ())
}

/// Discharge a trap's spell at the party: a deed with no caster reaching
/// living members at random, keeping the one who set it off. Whether this
/// discharge destroys the trap, and the deed. The trap is one
/// [`validate`] accepted.
pub(crate) fn discharge(
    (trap, difficulty): (u8, u8),
    (members, selected): (Vec<Entity>, Option<Entity>),
    content: &Catalogs,
    rng: &mut GameRng,
) -> (bool, Deed) {
    let id = trap_spell(trap, content).expect("a validated trap");
    let spell = &content.spells[id];
    let grade = trap_kind(trap).grade;
    let sprung = workings::spring(rng, (difficulty, grade), &members, selected);
    let strike = Strike {
        rows: Rows::Spell(id),
        strength: power::strength(0, sprung.power, 0, spell),
        blow: None,
        by: None,
    };
    (sprung.destroyed, Deed::at(strike, sprung.victims))
}
