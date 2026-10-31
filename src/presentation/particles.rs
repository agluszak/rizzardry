//! A level's particle systems as the player sees them: births on their
//! emitters' schedules, each particle's motion and expiry, and its sprite.
//! The game keeps only whether each emitter emits ([`Emitter`]); retail
//! likewise counts births and moves particles in its renderer
//! (`stParticle::Update`). A spent emission limit stops births
//! (`ActivateParticle`), and the view reports it to the game, which switches
//! the emitter off (`SubmitToRenderer`).
use super::{PresentationRng, billboards::FaceCamera, environment::LevelAppearance};
use crate::{
    content::{level::LevelGeometry, particles::*},
    game::{
        party::body::PartyBody,
        world::{
            level::{LevelOwned, OwnedDefinitions},
            obstacles::WorldObstacles,
            particles::{EmissionSpent, Emitter},
        },
    },
};
use bevy::prelude::*;
use rand::Rng;
use std::time::Duration;

/// An emitter's births so far this visit (retail `emission_count`,
/// `last_emission_tick`).
#[derive(Component, Debug, Default)]
pub struct Emission {
    last: Option<Duration>,
    emitted: u32,
}

/// One particle of an emitter, its child.
#[derive(Component, Debug)]
pub struct Particle {
    pub born: Duration,
    integrated: Duration,
    pub velocity: Vec3,
    pub flutter_angle: f32,
}

#[derive(Component)]
struct ParticleSprite;
#[derive(Resource)]
struct ParticleMesh(Handle<Mesh>);
pub(super) fn plugin(app: &mut App) {
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(1.0, 1.0));
    app.insert_resource(ParticleMesh(mesh)).add_observer(track_emission).add_systems(
        Update,
        (
            move_particles,
            // Settled before a departing level is remembered.
            emit_particles.before(crate::game::world::particles::switch_off_spent),
            attach_sprites,
            animate_particles,
        )
            .chain(),
    );
}

/// An emitter's births are counted from its arrival.
fn track_emission(add: On<Add<Emitter>>, mut commands: Commands) {
    commands.entity(add.entity).insert(Emission::default());
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
    if config.follows_eye {
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

/// An emitter's authored definition, through its level.
fn config<'a>(
    definitions: &'a OwnedDefinitions,
    owner: &LevelOwned,
    emitter: &Emitter,
) -> Option<&'a ParticleConfig> {
    definitions.of(owner)?.particles.get(emitter.definition())
}

fn move_particles(
    time: Res<Time<Virtual>>,
    definitions: OwnedDefinitions,
    emitters: Query<(&LevelOwned, &Emitter)>,
    party: Query<&avian3d::prelude::Position, With<PartyBody>>,
    mut particles: Query<(Entity, &ChildOf, &mut Particle, &mut Transform)>,
    obstacles: WorldObstacles,
    mut commands: Commands,
) {
    let eye = party.single().ok().map(PartyBody::eye);
    for (entity, emitter, mut particle, mut transform) in &mut particles {
        let Ok((owner, emitter)) = emitters.get(emitter.parent()) else { continue };
        let Some(config) = config(&definitions, owner, emitter) else {
            commands.entity(entity).despawn();
            continue;
        };
        let age = time.elapsed().saturating_sub(particle.born);
        if expired(&config.lifetime, age) {
            commands.entity(entity).despawn();
            continue;
        }
        let elapsed = time.elapsed().saturating_sub(particle.integrated).as_secs_f32();
        particle.integrated = time.elapsed();
        particle.velocity += config.acceleration * elapsed;
        let next = transform.translation + particle.velocity * elapsed;
        let blocked = config.stops_at_scenery && !obstacles.clear(transform.translation, next);
        if blocked || !inside(&config.bounds, next, emitter_pose(config, eye)) {
            commands.entity(entity).despawn();
        } else if transform.translation != next {
            transform.translation = next;
        }
    }
}

fn emit_particles(
    time: Res<Time<Virtual>>,
    definitions: OwnedDefinitions,
    mut emitters: Query<(Entity, &LevelOwned, &Emitter, &mut Emission, Option<&Children>)>,
    particles: Query<&Particle>,
    party: Query<&avian3d::prelude::Position, With<PartyBody>>,
    mut rng: ResMut<PresentationRng>,
    mut spent_emitters: MessageWriter<EmissionSpent>,
    mut commands: Commands,
) {
    let eye = party.single().ok().map(PartyBody::eye);
    let now = time.elapsed();
    for (entity, owner, emitter, mut emission, produced) in &mut emitters {
        let Some(config) = config(&definitions, owner, emitter) else { continue };
        if !emitter.emitting() || matches!(config.emission, ParticleEmission::None) {
            emission.last = None;
            continue;
        }
        let spent = |emission: &Emission| {
            config.emission_limit != 0 && emission.emitted >= config.emission_limit
        };
        // Its births spent, it is switched off (retail `SubmitToRenderer`).
        if spent(&emission) {
            spent_emitters.write(EmissionSpent(entity));
            continue;
        }
        // Turned on: its schedule starts now (retail `SetActive`).
        let Some(last) = emission.last else {
            emission.last = Some(now);
            continue;
        };
        let elapsed = now.saturating_sub(last);
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
            // A spent limit stops births (retail `ActivateParticle`).
            if spent(&emission) {
                emission.last = Some(now);
                break;
            }
            let born = match config.emission {
                ParticleEmission::CatchUp { .. } => emission.last.unwrap() + config.interval,
                ParticleEmission::Single | ParticleEmission::None => now,
            };
            if expired(&config.lifetime, now.saturating_sub(born)) {
                emission.last = Some(born);
                emission.emitted = emission.emitted.saturating_add(1);
                continue;
            }
            if existing.len() >= config.capacity as usize {
                if !config.replace_when_full || existing.is_empty() {
                    emission.last = Some(now);
                    break;
                }
                let (index, _) = existing
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (born, entity))| (*born, *entity))
                    .unwrap();
                commands.entity(existing.swap_remove(index).1).despawn();
            }
            let rng = &mut rng.0;
            let mut velocity = match config.direction {
                ParticleDirection::Stationary => Vec3::ZERO,
                ParticleDirection::Fixed(direction) => direction,
                ParticleDirection::Forward => transform.rotation * Vec3::NEG_Z,
                ParticleDirection::Cone { yaw, pitch } => {
                    transform.rotation
                        * Quat::from_rotation_y((rng.random::<f32>() - 0.5) * yaw)
                        * Quat::from_rotation_x((rng.random::<f32>() - 0.5) * pitch)
                        * Vec3::NEG_Z
                }
                ParticleDirection::Random => Vec3::new(
                    rng.random::<f32>() - 0.5,
                    rng.random::<f32>() - 0.5,
                    rng.random::<f32>() - 0.5,
                )
                .normalize_or(Vec3::Y),
            } * match config.speed {
                ParticleSpeed::Zero => 0.0,
                ParticleSpeed::Fixed(speed) => speed,
                ParticleSpeed::Random { minimum, maximum } => {
                    minimum + (maximum - minimum) * rng.random::<f32>()
                }
            };
            let mut position = transform.translation;
            let lag = if let ParticleEmission::CatchUp { minimum, maximum } = config.emission {
                position += transform.rotation
                    * (minimum
                        + (maximum - minimum)
                            * Vec3::new(rng.random(), rng.random(), rng.random()));
                emission.last = Some(born);
                now.saturating_sub(born).as_secs_f32()
            } else {
                emission.last = Some(now);
                0.0
            };
            velocity += config.acceleration * lag;
            position += velocity * lag;
            let particle = commands
                .spawn((
                    Particle {
                        born,
                        integrated: now,
                        velocity,
                        flutter_angle: rng.random::<f32>() * std::f32::consts::TAU,
                    },
                    Transform::from_translation(position),
                    ChildOf(entity),
                ))
                .id();
            emission.emitted = emission.emitted.saturating_add(1);
            existing.push((born, particle));
        }
        // The final birth spends it in the same update (retail
        // `SubmitToRenderer` checks right after `Update`).
        if spent(&emission) {
            spent_emitters.write(EmissionSpent(entity));
        }
    }
}
fn attach_sprites(
    levels: Query<&LevelAppearance>,
    geometry: Res<Assets<LevelGeometry>>,
    emitters: Query<(&LevelOwned, &Emitter)>,
    particles: Query<(Entity, &ChildOf, Option<&Children>), With<Particle>>,
    sprites: Query<(), With<ParticleSprite>>,
    mesh: Res<ParticleMesh>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (entity, parent, children) in &particles {
        if children.is_some_and(|children| children.iter().any(|child| sprites.contains(child))) {
            continue;
        }
        let Ok((owner, emitter)) = emitters.get(parent.parent()) else { continue };
        let Some(surface) = levels
            .get(owner.0)
            .ok()
            .and_then(|level| geometry.get(&level.0))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
        let Some(mut material) = materials.get(&surface.material).cloned() else { continue };
        material.cull_mode = None;
        if material.alpha_mode == AlphaMode::Opaque {
            material.alpha_mode = AlphaMode::Blend;
        }
        let material = materials.add(material);
        commands.spawn((
            ParticleSprite,
            FaceCamera,
            Mesh3d(mesh.0.clone()),
            MeshMaterial3d(material),
            Transform::default(),
            ChildOf(entity),
        ));
    }
}
fn animate_particles(
    time: Res<Time<Virtual>>,
    geometry: Res<Assets<LevelGeometry>>,
    levels: Query<&LevelAppearance>,
    definitions: OwnedDefinitions,
    emitters: Query<(&LevelOwned, &Emitter)>,
    particles: Query<(&ChildOf, &Particle)>,
    mut sprites: Query<
        (&ChildOf, &MeshMaterial3d<StandardMaterial>, &mut Transform),
        With<ParticleSprite>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (parent, material, mut transform) in &mut sprites {
        let Ok((emitter, particle)) = particles.get(parent.parent()) else { continue };
        let Ok((parent, emitter)) = emitters.get(emitter.parent()) else { continue };
        let Some(visual) = levels
            .get(parent.0)
            .ok()
            .and_then(|appearance| geometry.get(&appearance.0))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
        let Some(config) = config(&definitions, parent, emitter) else { continue };
        let age = time.elapsed().saturating_sub(particle.born);
        let fade = if age.is_zero() {
            1.0
        } else {
            match config.lifetime {
                ParticleLifetime::Timed(life) => {
                    (life.saturating_sub(age).as_secs_f32() / 0.5).min(1.0)
                }
                _ => 1.0,
            }
        };
        let frame = visual.animation.frame_at(age.as_secs_f32());
        let texture = visual.animation.frames.get(frame);
        let alpha =
            materials.get(&visual.material).map_or(1.0, |base| base.base_color.alpha()) * fade;
        if let Some(current) = materials.get(&material.0)
            && (current.base_color.alpha() != alpha
                || texture.is_some_and(|t| current.base_color_texture.as_ref() != Some(t)))
            && let Some(mut current) = materials.get_mut(&material.0)
        {
            current.base_color.set_alpha(alpha);
            if let Some(texture) = texture {
                current.base_color_texture = Some(texture.clone());
            }
        }
        let mut flutter = Vec3::ZERO;
        if let Some((amplitude, period)) = config.flutter
            && !period.is_zero()
            && particle.velocity.y < 0.0
        {
            let phase = (time.elapsed().as_secs_f32() % period.as_secs_f32())
                / period.as_secs_f32()
                * std::f32::consts::TAU;
            let amount = particle.velocity.y.max(-2.0) * -0.001 * amplitude * phase.sin();
            flutter = Quat::from_rotation_y(particle.flutter_angle) * Vec3::new(amount, 0.0, 0.0);
        }
        let scale = Vec3::splat(config.size);
        if transform.translation != flutter || transform.scale != scale {
            transform.translation = flutter;
            transform.scale = scale;
        }
    }
}
