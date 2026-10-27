//! Shared cast/payment transaction for an already validated device target.
use super::{
    Field, LearnedSpells, MagicContext, Members, SpellPoints, Strength,
    casting::{self, Refusal, Report, Source, UseContext},
};
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
                  mut context: MagicContext| {
                if !context.roster.contains(&caster) {
                    return Err(Refusal::CannotAct);
                }
                let party_level = context.party_level(&members);
                let MagicContext {
                    roster,
                    data,
                    items,
                    species,
                    spells,
                    difficulty,
                    formation,
                    party_damage,
                    rng,
                    ..
                } = &mut context;
                let data = data.as_ref().ok_or(Refusal::MissingContent)?;
                let (Some(items), Some(species), Some(spells)) =
                    (items.get(&data.items), species.get(&data.species), spells.get(&data.spells))
                else {
                    return Err(Refusal::MissingContent);
                };
                let party: Vec<_> = roster.iter().collect();
                let mut field = Field {
                    members: &mut members,
                    monsters: &mut monsters,
                    formation,
                    party_damage,
                    items,
                    species,
                    party_level,
                    difficulty: **difficulty,
                    party: &party,
                    improved: Vec::new(),
                };
                casting::perform_device(
                    caster,
                    source,
                    use_context,
                    spells,
                    &mut points,
                    rng,
                    &mut field,
                )
            },
        )
        .map_err(|_| Refusal::MissingContent)?
}
