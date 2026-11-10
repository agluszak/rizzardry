//! Indirect trap spells: a sprung trap's spell lands on members of the
//! party as a deed with no caster.

use super::power;
use super::workings::{self, Springing};
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::Strike;
use crate::game::hits::{Act, Deed, Release};
use crate::game::rng::GameRng;
use crate::game::world::interaction::{Unable, trap_kind};
use bevy::prelude::*;

/// The spell a kind of trap discharges, if the game can resolve it.
fn trap_spell(trap: u8, definitions: &Definitions) -> Result<SpellId, Unable> {
    let id = trap_kind(trap).spell;
    if definitions.spell(id).is_none() {
        return Err(Unable::UnsupportedTrap);
    }
    Ok(id)
}

/// No attempted inspection/disarm may roll if its failure cannot be resolved.
pub(crate) fn validate(trap: u8, definitions: &Definitions) -> Result<(), Unable> {
    trap_spell(trap, definitions).map(|_| ())
}

/// Discharge a trap's spell at the party: a deed with no caster reaching
/// living members at random, keeping the one who set it off. Whether this
/// discharge destroys the trap, and the deed to release. The trap is one
/// [`validate`] accepted.
pub(crate) fn discharge(
    (trap, difficulty): (u8, u8),
    (members, selected): (Vec<Entity>, Option<Entity>),
    (content, definitions): (&Catalogs, &Definitions),
    rng: &mut GameRng,
) -> (bool, Release) {
    let id = trap_spell(trap, definitions).expect("a validated trap");
    let spell = &content.spells[id];
    let grade = trap_kind(trap).grade;
    let sprung =
        workings::spring(rng, &Springing { difficulty, grade, members: &members, selected });
    let strike = Strike {
        definition: definitions.spell(id).unwrap(),
        strength: power::strength(0, sprung.power, 0, spell),
        blow: None,
    };
    let release = Release {
        deed: Deed::new(Act::Triggered { spell: id }, strike),
        victims: sprung.victims.into_iter().map(|victim| (victim, None)).collect(),
    };
    (sprung.destroyed, release)
}
