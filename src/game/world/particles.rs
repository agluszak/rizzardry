//! A level's particle systems as the game keeps them: whether each emits,
//! and whether a trigger has set it, which a dormant record keeps (retail
//! `SaveParticleStates`). Births, motion and drawing are presentation's
//! (`presentation::particles`); no emission count or particle reaches play.
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
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::content::level::{StateChange, TriggerAction};
    use crate::content::particles::*;
    use crate::game::world::{
        scripts::LevelScript,
        triggers::tests::{Scenario, trigger},
    };
    use std::time::Duration;

    /// A level with one limited steam emitter, off, and triggers to turn it
    /// on and off.
    pub(crate) fn scene() -> Scenario {
        let on =
            trigger("ON", TriggerAction::Particles { sources: vec![0], change: StateChange::Set });
        let off = trigger(
            "OFF",
            TriggerAction::Particles { sources: vec![0], change: StateChange::Clear },
        );
        let mut scene = Scenario::new(vec![on, off], &[], LevelScript::None);
        scene.geometry.particles.push(ParticleConfig {
            name: "STEAM".into(),
            transform: Transform::default(),
            capacity: 2,
            replace_when_full: false,
            bounds: ParticleBounds::None,
            emission: ParticleEmission::Single,
            interval: Duration::from_millis(100),
            direction: ParticleDirection::Fixed(Vec3::Y),
            speed: ParticleSpeed::Fixed(1.0),
            acceleration: Vec3::ZERO,
            size: 0.5,
            lifetime: ParticleLifetime::Timed(Duration::from_millis(300)),
            stops_at_scenery: false,
            follows_eye: false,
            flutter: None,
            initially_active: false,
            emission_limit: 2,
            attachment: None,
            frames: None,
        });
        scene.visit(None);
        scene
    }

    fn active(scene: &mut Scenario) -> bool {
        scene.world.query::<&Emitter>().single(&scene.world).unwrap().emitting()
    }

    /// Only what a trigger set survives a visit; an untouched emitter
    /// starts as authored.
    #[test]
    fn a_trigger_set_activation_is_remembered_across_visits() {
        let mut scene = scene();
        let memory = scene.remember();
        scene.visit(Some(memory));
        assert!(!active(&mut scene));
        assert!(scene.run("ON").0);
        assert!(active(&mut scene));
        let memory = scene.remember();
        scene.visit(Some(memory));
        assert!(active(&mut scene));
        assert!(scene.run("OFF").0);
        let memory = scene.remember();
        scene.visit(Some(memory));
        assert!(!active(&mut scene));
    }
}
