//! Indirect trap spells: a sprung trap's spell lands on members of the
//! party as a deed with no caster.

use super::Strength;
use crate::content::spells::SpellId;
use crate::game::characters::Vitals;
use crate::game::hits::{Act, Deed};
use crate::game::party::roster::PartyRoster;
use crate::game::world::interaction::Unable;
use bevy::{ecs::system::RunSystemOnce, prelude::*};

pub const SPELLS: [SpellId; 15] = [
    SpellId::DAGGER_SCATTER,
    SpellId::POISON_DARTS,
    SpellId::ALARM,
    SpellId::INSANITY,
    SpellId::WHIPPING_ROCKS,
    SpellId::FIREBALL,
    SpellId::FREEZE_FLESH,
    SpellId::SUMMON_ELEMENTAL,
    SpellId::INSTANT_DEATH,
    SpellId::PHILOSOPHERS_BANE,
    SpellId::VAMPIRIC_VORTEX,
    SpellId::HEATWAVE,
    SpellId::PRISMIC_RAY,
    SpellId::ANTI_MAGIC,
    SpellId::CEREBRAL_HEMORRHAGE,
];

/// The spell a trap type discharges, if the game can resolve it.
fn trap_spell(trap: u8) -> Result<SpellId, Unable> {
    let id = *SPELLS.get(usize::from(trap)).ok_or(Unable::UnsupportedTrap)?;
    if !crate::game::hits::spells::modelled(id) {
        return Err(Unable::UnsupportedTrap);
    }
    Ok(id)
}

/// No attempted inspection/disarm may roll if its failure cannot be resolved.
pub(crate) fn validate(trap: u8) -> Result<(), Unable> {
    trap_spell(trap).map(|_| ())
}

/// Discharge a trap's spell at the party: at random members, keeping the
/// one who set it off. Returns whether this discharge destroys the trap.
/// The caller still owns the guard and the trigger.
pub(crate) fn discharge(
    world: &mut World,
    trap: u8,
    difficulty: u8,
    selected: Option<Entity>,
) -> Result<bool, Unable> {
    let id = trap_spell(trap)?;
    let destroyed = world
        .run_system_once(
            move |bodies: Query<&Vitals>,
                  roster: Single<&PartyRoster>,
                  content: If<Res<crate::game::data::Catalogs>>,
                  mut rng: ResMut<crate::game::rng::GameRng>,
                  mut commands: Commands|
                  -> bool {
                let party: Vec<Entity> = roster.iter().collect();
                let mut targets: Vec<Entity> =
                    party.iter().copied().filter(|&m| !bodies.get(m).unwrap().is_dead()).collect();
                let spell = &content.spells[id];
                let difficulty = u32::from(difficulty.clamp(1, 7));
                let destroyed = rng.below(2) == 0;
                let mut count = difficulty - rng.below(difficulty / 2);
                let grade = crate::game::world::interaction::TRAP_GRADE[trap as usize];
                let extra = difficulty.max(grade) - grade;
                let power = (extra + 4).min(7);
                if extra > 3 {
                    count += rng.below(extra - 3);
                }
                // Retail deletes random slots from roster order, retaining the
                // selected living actor. Sleeping/unconscious members can be
                // victims too.
                while targets.len() > count as usize {
                    let index = rng.below(targets.len() as u32) as usize;
                    if Some(targets[index]) != selected {
                        targets.remove(index);
                    }
                }
                let deed = commands.spawn(Deed::new(Act::Triggered { spell: id })).id();
                let strength = Strength::at(0, spell, power, 0);
                crate::game::hits::spells::land(
                    &mut commands,
                    deed,
                    (id, strength, false),
                    &targets,
                    &party,
                );
                destroyed
            },
        )
        .unwrap();
    Ok(destroyed)
}
