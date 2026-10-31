//! Indirect trap spells: preflight content, capture real party actors and
//! settle consequences synchronously, before the device may unlock.

use super::{
    LearnedSpells, MagicContext, SpellPoints, Strength, Victim, casting::MagicNews, effects,
};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::party::Members;
use crate::game::world::{damage::MonsterDamage, interaction::Unable};
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
    if !effects::modelled(id) {
        return Err(Unable::UnsupportedTrap);
    }
    Ok(id)
}

/// No attempted inspection/disarm may roll if its failure cannot be resolved.
pub(crate) fn validate(trap: u8) -> Result<(), Unable> {
    trap_spell(trap).map(|_| ())
}

/// Returns whether this discharge destroys the trap. The caller still owns
/// the guard and the trigger; an effect report never substitutes for execution.
pub(crate) fn discharge(
    world: &mut World,
    trap: u8,
    difficulty: u8,
    selected: Option<Entity>,
) -> Result<bool, Unable> {
    trap_spell(trap)?;
    world
        .run_system_once(
            move |(mut members, mut monsters): (Members, MonsterDamage),
                  mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
                  mut context: MagicContext,
                  mut rng: ResMut<crate::game::rng::GameRng>,
                  clock: Res<crate::game::time::GameClock>|
                  -> Result<bool, Unable> {
                let mut targets: Vec<Entity> = context
                    .roster
                    .iter()
                    .filter(|&actor| members.get(actor).is_ok_and(|(_, v, ..)| !v.is_dead()))
                    .collect();
                let (spell, effects, improved, destroyed) =
                    context.resolve(clock.ms, &mut members, &mut monsters, |field| {
                        let spell: &SpellDefinition = &field.spells[SPELLS[usize::from(trap)]];
                        let difficulty = u32::from(difficulty.clamp(1, 7));
                        let destroyed = rng.below(2) == 0;
                        let mut count = difficulty - rng.below(difficulty / 2);
                        let extra = (difficulty as i32
                            - crate::game::world::interaction::TRAP_GRADE[trap as usize])
                            .max(0) as u32;
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
                        let victims: Vec<_> = targets.into_iter().map(Victim::Member).collect();
                        let effects = effects::resolve(
                            None,
                            false,
                            spell,
                            Strength::at(0, spell, power, 0),
                            &victims,
                            &mut rng,
                            field,
                            &mut points,
                        );
                        (spell.id, effects, std::mem::take(&mut field.improved), destroyed)
                    });
                context.news.write(MagicNews::Triggered { spell, effects, improved });
                Ok(destroyed)
            },
        )
        .unwrap()
}
