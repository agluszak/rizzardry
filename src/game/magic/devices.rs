//! Shared cast/payment transaction for an already validated utility or device target.
use super::{
    LearnedSpells, MagicContext, SpellPoints, Strength,
    casting::{self, Refusal, Report, Source, UseContext},
};
use crate::game::party::Members;
use crate::game::world::damage::MonsterDamage;
use bevy::{ecs::system::RunSystemOnce, prelude::*};

pub(crate) fn cast_source(
    world: &mut World,
    caster: Entity,
    source: Source,
    use_context: UseContext,
) -> Result<(Report, Strength), Refusal> {
    world
        .run_system_once(
            move |(mut members, mut monsters): (Members, MonsterDamage),
                  mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
                  mut context: MagicContext,
                  mut rng: ResMut<crate::game::rng::GameRng>| {
                if !context.roster.contains(&caster) {
                    return Err(Refusal::CannotAct);
                }
                context
                    .resolve(&mut members, &mut monsters, |field, spells| {
                        casting::perform_utility(
                            caster,
                            source,
                            use_context,
                            spells,
                            &mut points,
                            &mut rng,
                            field,
                        )
                    })
                    .and_then(|performed| performed)
            },
        )
        .map_err(|_| Refusal::MissingContent)?
}
