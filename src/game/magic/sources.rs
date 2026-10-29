//! Shared source/payment transaction after utility/device target validation.
use super::{
    LearnedSpells, SpellPoints, Strength,
    casting::{self, Refusal, Report, Source, UseContext},
};
use crate::game::{
    data::Content,
    difficulty::Difficulty,
    inventory::PartyInventory,
    party::{Members, roster::PartyRoster},
    rng::GameRng,
};
use bevy::{ecs::system::RunSystemOnce, prelude::*};

pub(crate) fn cast_source(
    world: &mut World,
    caster: Entity,
    source: Source,
    use_context: UseContext,
) -> Result<(Report, Strength), Refusal> {
    world
        .run_system_once(
            move |mut members: Members,
                  mut points: Query<(&LearnedSpells, &mut SpellPoints)>,
                  mut inventory: Single<&mut PartyInventory>,
                  roster: Single<&PartyRoster>,
                  levels: Query<
                &crate::game::world::triggers::LevelState,
                With<crate::game::world::level::Level>,
            >,
                  content: Content,
                  difficulty: Res<Difficulty>,
                  mut rng: ResMut<GameRng>| {
                if !roster.contains(&caster) {
                    return Err(Refusal::CannotAct);
                }
                let (Some(spells), Some(items)) = (content.spells(), content.items()) else {
                    return Err(Refusal::MissingContent);
                };
                casting::perform_utility(
                    caster,
                    source,
                    use_context,
                    spells,
                    &mut points,
                    &mut rng,
                    &mut members,
                    &mut inventory,
                    items,
                    *difficulty,
                    levels.single().is_ok_and(|state| state.camera_sway),
                )
            },
        )
        .map_err(|_| Refusal::MissingContent)?
}
