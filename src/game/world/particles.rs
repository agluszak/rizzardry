//! A level's particle systems as the game keeps them: whether each emits,
//! and whether a trigger has set it, which a dormant record keeps (retail
//! `SaveParticleStates`). Births, motion and drawing are presentation's
//! (`presentation::particles`); only an emitter's spent birth limit, which
//! switches it off, comes back to play.
use crate::{content::level::LevelDefinition, game::world::level::LevelOwned};
use bevy::prelude::*;

#[derive(Component, Debug)]
pub struct Emitter {
    definition: usize,
    emitting: bool,
    persisted: bool,
}
impl Emitter {
    pub fn definition(&self) -> usize {
        self.definition
    }
    pub fn emitting(&self) -> bool {
        self.emitting
    }
    /// A trigger turns it on or off (retail `stParticle::SetActive`); from
    /// then on the level remembers it.
    pub fn set_active(&mut self, active: bool) {
        self.emitting = active;
        self.persisted = true;
    }
}

/// An emitter's births have reached its limit, as the view that births them
/// counts them. Retail switches it off then (`stParticle::SubmitToRenderer`)
/// without marking it set, so a level remembers it off only if a trigger
/// set it; headless play births nothing and spends no limit.
#[derive(Message, Clone, Copy, Debug)]
pub struct EmissionSpent(pub Entity);

pub fn switch_off_spent(
    mut spent: MessageReader<EmissionSpent>,
    mut emitters: Query<&mut Emitter>,
) {
    for EmissionSpent(entity) in spent.read() {
        if let Ok(mut emitter) = emitters.get_mut(*entity) {
            emitter.emitting = false;
        }
    }
}

#[derive(Debug, Default)]
pub struct ParticleMemory(Vec<(usize, bool)>);
pub fn remember<'a>(emitters: impl Iterator<Item = &'a Emitter>) -> ParticleMemory {
    ParticleMemory(
        emitters
            .filter(|emitter| emitter.persisted)
            .map(|emitter| (emitter.definition, emitter.emitting))
            .collect(),
    )
}
pub fn spawn_emitters(
    commands: &mut Commands,
    root: Entity,
    definition: &LevelDefinition,
    memory: ParticleMemory,
) {
    for (index, config) in definition.particles.iter().enumerate() {
        let previous = memory.0.iter().find(|(definition, _)| *definition == index);
        commands.spawn((
            Emitter {
                definition: index,
                emitting: previous.map_or(config.initially_active, |(_, active)| *active),
                persisted: previous.is_some(),
            },
            Transform::default(),
            LevelOwned(root),
        ));
    }
}
