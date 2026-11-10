//! A level's particle systems as the game keeps them: whether each emits.
//! Births, motion and drawing are presentation's (`presentation::particles`);
//! only an emitter's spent birth limit, which switches it off, comes back to
//! play.
use crate::{content::level::LevelDefinition, game::world::level::LevelOwned};
use bevy::prelude::*;

#[derive(Component, Debug)]
pub struct Emitter {
    definition: usize,
    emitting: bool,
}
impl Emitter {
    pub fn definition(&self) -> usize {
        self.definition
    }
    pub fn emitting(&self) -> bool {
        self.emitting
    }
    /// A trigger turns it on or off (retail `stParticle::SetActive`).
    pub fn set_active(&mut self, active: bool) {
        self.emitting = active;
    }
}

/// An emitter's births have reached its limit, as the view that births them
/// counts them. Retail switches it off then (`stParticle::SubmitToRenderer`)
/// without marking it set; headless play births nothing and spends no limit.
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

pub fn spawn_emitters(commands: &mut Commands, root: Entity, definition: &LevelDefinition) {
    for (index, config) in definition.particles.iter().enumerate() {
        commands.spawn((
            Emitter { definition: index, emitting: config.initially_active },
            Transform::default(),
            LevelOwned(root),
        ));
    }
}
