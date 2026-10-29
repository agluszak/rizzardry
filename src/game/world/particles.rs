//! Named scene emitters own activation, emission history and particle motion.
//! Rendering observes these entities; artwork cannot decide retirement or saved
//! activation. The separate cosmetic RNG never advances gameplay randomness.
use crate::{
    content::{level::LevelDefinition, particles::*},
    game::{
        party::body::PartyBody,
        world::{
            level::{Level, LevelOwned},
            obstacles::WorldObstacles,
        },
    },
};
use bevy::prelude::*;
use rand::{Rng, SeedableRng};
use std::time::Duration;

#[derive(Component, Debug)]
pub struct Emitter {
    definition: usize,
    emitting: bool,
    persisted: bool,
    last_emission: Option<Duration>,
    emitted: u32,
}
impl Emitter {
    pub fn definition(&self) -> usize {
        self.definition
    }
    pub fn emitting(&self) -> bool {
        self.emitting
    }
    pub fn set_active(&mut self, active: bool, now: Duration) {
        if active && !self.emitting {
            self.last_emission = Some(now);
        }
        self.emitting = active;
        self.persisted = true;
    }
}
#[derive(Component, Debug)]
pub struct Particle {
    pub born: Duration,
    integrated: Duration,
    pub velocity: Vec3,
    pub flutter_angle: f32,
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
                last_emission: None,
                emitted: 0,
            },
            Transform::default(),
            LevelOwned(root),
        ));
    }
}
#[derive(Resource)]
struct ParticleRng(rand::rngs::SmallRng);
impl Default for ParticleRng {
    fn default() -> Self {
        Self(rand::rngs::SmallRng::from_os_rng())
    }
}
pub fn plugin(app: &mut App) {
    app.init_resource::<ParticleRng>().add_systems(
        Update,
        (move_particles, emit_particles).chain().after(crate::game::GameplaySet::LevelActivation),
    );
}
fn expired(lifetime: &ParticleLifetime, age: Duration) -> bool {
    match lifetime {
        ParticleLifetime::Timed(limit) => age > *limit,
        ParticleLifetime::TextureOnce(limit) => age >= *limit,
        ParticleLifetime::TextureLoop => false,
    }
}
fn emitter_pose(config: &ParticleConfig, eye: Option<Vec3>) -> Transform {
    let mut transform = config.transform;
    if config.name.starts_with("CLOUD") {
        transform.translation += eye.unwrap_or_default();
    }
    transform
}

fn inside(bounds: &ParticleBounds, point: Vec3, transform: Transform) -> bool {
    let local = transform.rotation.inverse() * (point - transform.translation);
    match bounds {
        ParticleBounds::None => true,
        ParticleBounds::Box { minimum, maximum } => {
            local.cmpge(*minimum).all() && local.cmple(*maximum).all()
        }
        ParticleBounds::Sphere { centre, radius } => local.distance(*centre) <= *radius,
    }
}

fn move_particles(
    time: Res<Time<Virtual>>,
    geometry: Res<Assets<LevelDefinition>>,
    levels: Query<&Level>,
    emitters: Query<(&LevelOwned, &Emitter)>,
    party: Query<&avian3d::prelude::Position, With<PartyBody>>,
    mut particles: Query<(Entity, &ChildOf, &mut Particle, &mut Transform)>,
    obstacles: WorldObstacles,
    mut commands: Commands,
) {
    let eye = party.single().ok().map(PartyBody::eye);
    for (entity, emitter, mut particle, mut transform) in &mut particles {
        let Ok((parent, emitter)) = emitters.get(emitter.parent()) else { continue };
        let Some(definition) =
            levels.get(parent.0).ok().and_then(|level| geometry.get(&level.definition))
        else {
            continue;
        };
        let Some(visual) = definition.particles.get(emitter.definition) else {
            commands.entity(entity).despawn();
            continue;
        };
        let age = time.elapsed().saturating_sub(particle.born);
        if expired(&visual.lifetime, age) {
            commands.entity(entity).despawn();
            continue;
        }
        let elapsed = time.elapsed().saturating_sub(particle.integrated).as_secs_f32();
        particle.integrated = time.elapsed();
        particle.velocity += visual.acceleration * elapsed;
        let next = transform.translation + particle.velocity * elapsed;
        let blocked = visual.stops_at_scenery && !obstacles.clear(transform.translation, next);
        if blocked || !inside(&visual.bounds, next, emitter_pose(visual, eye)) {
            commands.entity(entity).despawn();
        } else if transform.translation != next {
            transform.translation = next;
        }
    }
}

fn emit_particles(
    time: Res<Time<Virtual>>,
    geometry: Res<Assets<LevelDefinition>>,
    levels: Query<&Level>,
    mut emitters: Query<(Entity, &LevelOwned, &mut Emitter, Option<&Children>)>,
    particles: Query<&Particle>,
    party: Query<&avian3d::prelude::Position, With<PartyBody>>,
    mut rng: ResMut<ParticleRng>,
    mut commands: Commands,
) {
    let eye = party.single().ok().map(PartyBody::eye);
    for (entity, parent, mut emitter, produced) in &mut emitters {
        let Ok(level) = levels.get(parent.0) else { continue };
        let Some(definition) = geometry.get(&level.definition) else { continue };
        let Some(visual) = definition.particles.get(emitter.definition) else {
            commands.entity(entity).despawn();
            continue;
        };
        let config = visual;
        if !emitter.emitting || matches!(config.emission, ParticleEmission::None) {
            if emitter.last_emission.is_some() {
                emitter.last_emission = None;
            }
            continue;
        }
        let Some(last) = emitter.last_emission else {
            emitter.last_emission = Some(time.elapsed());
            continue;
        };
        let elapsed = time.elapsed().saturating_sub(last);
        if elapsed < config.interval {
            continue;
        }
        let transform = emitter_pose(config, eye);
        let due = match config.emission {
            ParticleEmission::Single => 1,
            _ => {
                ((elapsed.as_nanos().saturating_sub(1)) / config.interval.as_nanos().max(1)) as u64
            }
        };
        let mut existing: Vec<_> = produced
            .into_iter()
            .flat_map(|p| p.iter())
            .filter_map(|e| particles.get(e).ok().map(|p| (p.born, e)))
            .collect();
        for _ in 0..due {
            if config.emission_limit != 0 && emitter.emitted >= config.emission_limit {
                emitter.last_emission = Some(time.elapsed());
                emitter.emitting = false;
                break;
            }
            let born = match config.emission {
                ParticleEmission::CatchUp { .. } => {
                    emitter.last_emission.unwrap() + config.interval
                }
                ParticleEmission::Single | ParticleEmission::None => time.elapsed(),
            };
            if expired(&config.lifetime, time.elapsed().saturating_sub(born)) {
                emitter.last_emission = Some(born);
                emitter.emitted = emitter.emitted.saturating_add(1);
                if config.emission_limit != 0 && emitter.emitted >= config.emission_limit {
                    emitter.emitting = false;
                }
                continue;
            }
            if existing.len() >= config.capacity as usize {
                if !config.replace_when_full || existing.is_empty() {
                    emitter.last_emission = Some(time.elapsed());
                    break;
                }
                let (index, _) = existing
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (born, entity))| (*born, *entity))
                    .unwrap();
                commands.entity(existing.swap_remove(index).1).despawn();
            }
            let mut velocity = match config.direction {
                ParticleDirection::Stationary => Vec3::ZERO,
                ParticleDirection::Fixed(direction) => direction,
                ParticleDirection::Forward => transform.rotation * Vec3::NEG_Z,
                ParticleDirection::Cone { yaw, pitch } => {
                    transform.rotation
                        * Quat::from_rotation_y((rng.0.random::<f32>() - 0.5) * yaw)
                        * Quat::from_rotation_x((rng.0.random::<f32>() - 0.5) * pitch)
                        * Vec3::NEG_Z
                }
                ParticleDirection::Random => Vec3::new(
                    rng.0.random::<f32>() - 0.5,
                    rng.0.random::<f32>() - 0.5,
                    rng.0.random::<f32>() - 0.5,
                )
                .normalize_or(Vec3::Y),
            } * match config.speed {
                ParticleSpeed::Zero => 0.0,
                ParticleSpeed::Fixed(speed) => speed,
                ParticleSpeed::Random { minimum, maximum } => {
                    minimum + (maximum - minimum) * rng.0.random::<f32>()
                }
            };
            let mut position = transform.translation;
            let lag = if let ParticleEmission::CatchUp { minimum, maximum } = config.emission {
                position += transform.rotation
                    * (minimum
                        + (maximum - minimum)
                            * Vec3::new(rng.0.random(), rng.0.random(), rng.0.random()));
                emitter.last_emission = Some(born);
                time.elapsed().saturating_sub(born).as_secs_f32()
            } else {
                emitter.last_emission = Some(time.elapsed());
                0.0
            };
            velocity += config.acceleration * lag;
            position += velocity * lag;
            let particle = commands
                .spawn((
                    Particle {
                        born,
                        integrated: time.elapsed(),
                        velocity,
                        flutter_angle: rng.0.random::<f32>() * std::f32::consts::TAU,
                    },
                    Transform::from_translation(position),
                    ChildOf(entity),
                ))
                .id();
            emitter.emitted = emitter.emitted.saturating_add(1);
            if config.emission_limit != 0 && emitter.emitted >= config.emission_limit {
                emitter.emitting = false;
            }
            existing.push((born, particle));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        content::level::{StateChange, TriggerAction},
        game::{
            rng::GameRng,
            world::{
                scripts::LevelScript,
                triggers::tests::{Scenario, trigger},
            },
        },
    };

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
            flutter: None,
            initially_active: false,
            emission_limit: 2,
            attachment: None,
            frames: None,
        });
        scene.visit(None);
        scene.world.init_resource::<Time<Virtual>>();
        scene.world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        scene.world.insert_resource(ParticleRng(rand::rngs::SmallRng::seed_from_u64(3)));
        scene
    }
    pub(crate) fn step(scene: &mut Scenario, ms: u64) {
        scene.world.resource_mut::<Time<Virtual>>().advance_by(Duration::from_millis(ms));
        scene.world.run_system_cached(move_particles).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(emit_particles).unwrap();
        scene.world.flush();
    }
    fn count(scene: &mut Scenario) -> usize {
        scene.world.query::<&Particle>().iter(&scene.world).count()
    }
    fn active(scene: &mut Scenario) -> bool {
        scene.world.query::<&Emitter>().single(&scene.world).unwrap().emitting()
    }

    #[test]
    fn catch_up_keeps_scheduled_births_and_retires_old_texture_lifetimes() {
        let mut scene = scene();
        {
            let handle = scene.world.get::<Level>(scene.root).unwrap().definition.clone();
            let mut definitions = scene.world.resource_mut::<Assets<LevelDefinition>>();
            let config = &mut definitions.get_mut(&handle).unwrap().particles[0];
            config.emission =
                ParticleEmission::CatchUp { minimum: Vec3::ZERO, maximum: Vec3::ZERO };
            config.capacity = 20;
            config.emission_limit = 0;
            config.lifetime = ParticleLifetime::TextureOnce(Duration::from_millis(250));
        }
        assert!(scene.run("ON").0);
        step(&mut scene, 1000);
        let born: Vec<_> = scene
            .world
            .query::<&Particle>()
            .iter(&scene.world)
            .map(|particle| particle.born)
            .collect();
        assert_eq!(born, [Duration::from_millis(800), Duration::from_millis(900)]);
        let positions: Vec<_> = scene
            .world
            .query::<&Transform>()
            .iter(&scene.world)
            .map(|t| t.translation.y)
            .filter(|y| *y > 0.0)
            .collect();
        assert!(positions.iter().any(|&y| (y - 0.2).abs() < 0.0001));
        assert!(positions.iter().any(|&y| (y - 0.1).abs() < 0.0001));
    }

    #[test]
    fn finite_emission_retires_and_restores_actual_activation_without_artwork_or_gameplay_rng() {
        let mut scene = scene();
        let rng = scene.world.resource::<GameRng>().clone();
        assert!(scene.run("ON").0);
        step(&mut scene, 100);
        assert_eq!(count(&mut scene), 1);
        assert!(active(&mut scene));
        step(&mut scene, 100);
        assert_eq!(count(&mut scene), 2);
        assert!(!active(&mut scene), "retirement commits with the last birth");
        let memory = scene.remember();
        let member = scene.members[0];
        scene.visit(Some(memory));
        assert_eq!(count(&mut scene), 0, "old scene particles are owned by their emitter");
        assert!(scene.world.get_entity(member).is_ok());
        assert!(!active(&mut scene));
        step(&mut scene, 500);
        assert_eq!(count(&mut scene), 0);
        // Retail preserves the switched-off fact, but starts a new visit's budget.
        assert!(scene.run("ON").0);
        step(&mut scene, 100);
        assert_eq!(count(&mut scene), 1);
        assert!(active(&mut scene));
        assert_eq!(scene.world.resource::<GameRng>(), &rng);
    }

    #[test]
    fn stopping_leaves_existing_particles_to_move_and_expire_and_restart_has_no_backlog() {
        let mut scene = scene();
        assert!(scene.run("ON").0);
        step(&mut scene, 100);
        let entity =
            scene.world.query_filtered::<Entity, With<Particle>>().single(&scene.world).unwrap();
        assert!(scene.run("OFF").0);
        step(&mut scene, 100);
        assert_eq!(count(&mut scene), 1);
        assert!((scene.world.get::<Transform>(entity).unwrap().translation.y - 0.1).abs() < 1e-6);
        step(&mut scene, 201);
        assert_eq!(count(&mut scene), 0);
        assert!(scene.run("ON").0);
        step(&mut scene, 50);
        assert_eq!(count(&mut scene), 0);
        step(&mut scene, 50);
        assert_eq!(count(&mut scene), 1);
        assert!(!active(&mut scene), "stopping did not refill this visit's finite budget");
    }
}
