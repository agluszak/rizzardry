//! A level's particle systems as the player sees them: births on their
//! emitters' schedules, each particle's motion and expiry, and its sprite.
//! The game keeps only whether each emitter emits ([`Emitter`]); retail
//! likewise counts births and moves particles in its renderer
//! (`stParticle::Update`). A spent emission limit stops births
//! (`ActivateParticle`), and the view reports it to the game, which switches
//! the emitter off (`SubmitToRenderer`).
use super::{PresentationRng, billboards::FaceCamera, environment::LevelAppearance};
use crate::{
    content::{
        level::{LevelDefinition, LevelGeometry},
        particles::*,
    },
    game::{
        party::body::PartyBody,
        world::{
            level::{Level, LevelOwned},
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
    levels: &Query<&Level>,
    geometry: &'a Assets<LevelDefinition>,
    owner: &LevelOwned,
    emitter: &Emitter,
) -> Option<&'a ParticleConfig> {
    geometry.get(&levels.get(owner.0).ok()?.definition)?.particles.get(emitter.definition())
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
        let Ok((owner, emitter)) = emitters.get(emitter.parent()) else { continue };
        let Some(config) = config(&levels, &geometry, owner, emitter) else {
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
    geometry: Res<Assets<LevelDefinition>>,
    levels: Query<&Level>,
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
        let Some(config) = config(&levels, &geometry, owner, emitter) else { continue };
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
    levels: Query<(&Level, &LevelAppearance)>,
    definitions: Res<Assets<LevelDefinition>>,
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
            .and_then(|(_, appearance)| geometry.get(&appearance.0))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
        let Some(config) = levels
            .get(parent.0)
            .ok()
            .and_then(|(level, _)| definitions.get(&level.definition))
            .and_then(|level| level.particles.get(emitter.definition()))
        else {
            continue;
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::surface::{Surface, TextureFrames},
        game::world::triggers::tests::Scenario,
    };

    /// The game's emitter scene, seen: its emitters count their births.
    fn scene() -> Scenario {
        let mut scene = crate::game::world::particles::tests::scene();
        scene.world.init_resource::<Time<Virtual>>();
        scene.world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        scene.world.insert_resource(PresentationRng(rand::SeedableRng::seed_from_u64(3)));
        scene.world.add_observer(track_emission);
        scene.world.init_resource::<Messages<EmissionSpent>>();
        scene.visit(None);
        scene
    }

    fn step(scene: &mut Scenario, ms: u64) {
        scene.world.resource_mut::<Time<Virtual>>().advance_by(Duration::from_millis(ms));
        scene.world.run_system_cached(move_particles).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(emit_particles).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(crate::game::world::particles::switch_off_spent).unwrap();
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
        // The view sees it switched on, as on its next frame.
        step(&mut scene, 0);
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

    /// Retail's limit stops births and then switches the emitter off
    /// (`ActivateParticle`, `SubmitToRenderer`); one a trigger set is saved
    /// off, and switched on again on a later visit it counts afresh. The
    /// game's RNG is untouched.
    #[test]
    fn a_spent_limit_switches_a_triggered_emitter_off_across_visits() {
        let mut scene = scene();
        let rng = scene.world.resource::<crate::game::rng::GameRng>().clone();
        assert!(scene.run("ON").0);
        // The view sees it switched on, as on its next frame.
        step(&mut scene, 0);
        step(&mut scene, 100);
        step(&mut scene, 100);
        assert_eq!(count(&mut scene), 2);
        assert!(!active(&mut scene), "the final birth switches it off at once");
        // Leaving now remembers it off, with no further update.
        let memory = scene.remember();
        scene.visit(Some(memory));
        assert!(!active(&mut scene), "remembered off");
        assert!(scene.run("ON").0);
        step(&mut scene, 0);
        step(&mut scene, 100);
        assert_eq!(count(&mut scene), 1, "a new visit's births");
        assert_eq!(scene.world.resource::<crate::game::rng::GameRng>(), &rng);
    }

    #[test]
    fn stopping_leaves_existing_particles_to_move_and_expire_and_restart_has_no_backlog() {
        let mut scene = scene();
        assert!(scene.run("ON").0);
        // The view sees it switched on, as on its next frame.
        step(&mut scene, 0);
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
        // The view sees it switched on, as on its next frame.
        step(&mut scene, 0);
        step(&mut scene, 50);
        assert_eq!(count(&mut scene), 0);
        step(&mut scene, 50);
        assert_eq!(count(&mut scene), 1, "no backlog from the time it was off");
    }

    #[test]
    fn rebuilding_particle_views_preserves_birth_motion_and_independent_texture_phase() {
        let mut scene = scene();
        scene.world.init_resource::<Assets<StandardMaterial>>();
        scene.world.init_resource::<Assets<LevelGeometry>>();
        scene.world.init_resource::<Assets<Image>>();
        scene.world.insert_resource(ParticleMesh(default()));
        let source = scene.world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.5),
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let frames: Vec<_> = (0..2)
            .map(|_| scene.world.resource_mut::<Assets<Image>>().add(Image::default()))
            .collect();
        let appearance = scene.world.resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
            particles: vec![Surface {
                material: source.clone(),
                ambient: Vec3::ONE,
                emission: 0.0,
                animation: TextureFrames {
                    frames: frames.clone(),
                    frames_per_second: 10.0,
                    ..default()
                },
            }],
            ..default()
        });
        scene.world.entity_mut(scene.root).insert(LevelAppearance(appearance));
        assert!(scene.run("ON").0);
        // The view sees it switched on, as on its next frame.
        step(&mut scene, 0);
        step(&mut scene, 100);
        scene.world.run_system_cached(attach_sprites).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(animate_particles).unwrap();
        let (sprite, parent) = scene
            .world
            .query_filtered::<(Entity, &ChildOf), With<ParticleSprite>>()
            .single(&scene.world)
            .unwrap();
        let particle = parent.parent();
        let birth = scene.world.get::<Particle>(particle).unwrap().born;
        let handle = scene.world.get::<MeshMaterial3d<StandardMaterial>>(sprite).unwrap().0.clone();
        assert_eq!(
            scene
                .world
                .resource::<Assets<StandardMaterial>>()
                .get(&handle)
                .unwrap()
                .base_color
                .alpha(),
            0.5
        );
        step(&mut scene, 100);
        scene.world.run_system_cached(animate_particles).unwrap();
        scene.world.despawn(sprite);
        scene.world.run_system_cached(attach_sprites).unwrap();
        scene.world.flush();
        scene.world.run_system_cached(animate_particles).unwrap();
        let child = scene.world.get::<Children>(particle).unwrap()[0];
        let handle = &scene.world.get::<MeshMaterial3d<StandardMaterial>>(child).unwrap().0;
        let material = scene.world.resource::<Assets<StandardMaterial>>().get(handle).unwrap();
        assert!((material.base_color.alpha() - 0.2).abs() < 1e-6);
        assert_eq!(material.base_color_texture.as_ref(), Some(&frames[1]));
        assert_eq!(scene.world.get::<Particle>(particle).unwrap().born, birth);
        assert!((scene.world.get::<Transform>(particle).unwrap().translation.y - 0.1).abs() < 1e-6);
        assert_eq!(
            scene
                .world
                .resource::<Assets<StandardMaterial>>()
                .get(&source)
                .unwrap()
                .base_color
                .alpha(),
            0.5
        );
        scene.visit(None);
        assert_eq!(
            scene.world.query_filtered::<Entity, With<ParticleSprite>>().iter(&scene.world).count(),
            0
        );
    }
}
