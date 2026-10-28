//! Showing and animating monster models.
//!
//! Each [`Monster`] gets its model's cycles as children, one visible at a
//! time. Instances share immutable morph meshes and own their playback weights;
//! lighting and texture animation live on their materials.

use std::time::Duration;

use super::surfaces::ActorLight;
use bevy::camera::visibility::NoAutoAabb;
use bevy::mesh::morph::{MeshMorphWeights, MorphWeights};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::time::Stopwatch;

use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};

use super::environment::LevelAppearance;
use crate::content::level::LevelGeometry;
use crate::content::monster::MonsterModel;
use crate::game::time::GameClock;
use crate::game::world::navigation::Walker;
use crate::game::world::population::Monster;
use crate::presentation::screen_art::visible_if;

/// Seconds into its first cycle a monster starts at. Retail starts each
/// idle at a random frame; here it varies by location.
fn first_phase(monster: &Monster) -> f32 {
    (monster.location.unsigned_abs() % 97) as f32 * 0.37
}

/// The cycle an actor should play. Gameplay sets it; a different request
/// restarts playback, an identical one continues it.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct CycleRequest {
    pub cycle: String,
    /// Play once and hold the last frame (attacks, death), rather than loop.
    pub once: bool,
}

impl CycleRequest {
    pub fn looping(cycle: &str) -> Self {
        CycleRequest { cycle: cycle.to_owned(), once: false }
    }
}

/// Optional visual content retained while its model loads and plays. It is
/// never captured in a level record or used to size an actor's gameplay body.
#[derive(Component, Debug)]
pub struct MonsterVisual(pub Handle<MonsterModel>);

pub fn request_monster_visuals(
    mut commands: Commands,
    actors: Query<(Entity, &Monster), Without<MonsterVisual>>,
    content: crate::game::data::Content,
    assets: Res<AssetServer>,
) {
    let Some(catalog) = content.species() else { return };
    for (entity, monster) in &actors {
        let Some(kind) = catalog.kind(monster.species) else { continue };
        let model = assets.load(crate::content::paths::monster(&kind.model));
        commands.entity(entity).insert(MonsterVisual(model));
    }
}

/// The playback of one actor's built model.
#[derive(Component, Debug)]
pub struct MonsterPlayback {
    /// The cycle shown, and how long it has played.
    playing: Option<(CycleRequest, Stopwatch)>,
}

/// The lighting of one actor's built model, independent of its playback: a
/// settled corpse is still relit.
#[derive(Component, Debug)]
pub struct MonsterLight {
    /// How much sky light reaches the actor: retail fades it between 0.75
    /// in sight of the level's `SUN` light and 0 out of it.
    sky_scale: f32,
    /// Where the sunlight target and fixed light were last decided, and
    /// their values; only moving re-decides them.
    lit_at: Option<(Vec3, f32, Vec3)>,
}

/// The hierarchy itself records which visual entities were successfully built.
#[derive(Component, Debug)]
pub struct ModelRoot;

#[derive(Component, Debug)]
pub struct ModelCycle(String);

pub fn build_monster_instances(
    mut commands: Commands,
    actors: Query<(Entity, &Monster, &MonsterVisual, Option<&Walker>), Without<MonsterPlayback>>,
    models: Res<Assets<MonsterModel>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (actor, monster, visual, walker) in &actors {
        let Some(model) = models.get(&visual.0) else { continue };
        // Retail models face -Z; after the Z mirror that is +Z, so turn them
        // to face the actor's forward (-Z). Feet are at the model's origin.
        let model_root = commands
            .spawn((
                ModelRoot,
                Transform {
                    translation: Vec3::NEG_Y * walker.map_or(0.0, |w| w.float_height),
                    rotation: Quat::from_rotation_y(std::f32::consts::PI),
                    scale: Vec3::splat(monster.body.scale),
                },
                Visibility::default(),
                ChildOf(actor),
            ))
            .id();
        let mut instances = HashMap::new();
        for (name, cycle) in &model.cycles {
            let cycle_root = commands
                .spawn((
                    Name::new(name.clone()),
                    ModelCycle(name.clone()),
                    MorphWeights::new(
                        vec![0.0; cycle.frame_count],
                        cycle.parts.first().map(|p| p.mesh.clone()),
                    )
                    .expect("loader validated morph count"),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(model_root),
                ))
                .id();
            for part in &cycle.parts {
                let Some(surface) =
                    part.surface.lit_instance(actor, &mut instances, &mut materials)
                else {
                    continue;
                };
                commands.spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMorphWeights::Reference(cycle_root),
                    part.bounds,
                    NoAutoAabb,
                    surface,
                    ChildOf(cycle_root),
                ));
            }
        }
        commands.entity(actor).insert((
            MonsterPlayback { playing: None },
            MonsterLight { sky_scale: SUNLIT, lit_at: None },
            ActorLight::default(),
            Visibility::default(),
        ));
    }
}

/// A monster that dies plays its death once and stays on its last frame.
pub fn show_deaths(
    mut commands: Commands,
    monsters: Query<(Entity, &Monster, Option<&CycleRequest>), Changed<Monster>>,
) {
    for (entity, monster, request) in &monsters {
        if monster.is_dead() && request.is_none_or(|r| r.cycle != "DIE") {
            commands.entity(entity).insert(CycleRequest { cycle: "DIE".into(), once: true });
        }
    }
}

/// Show each actor's requested cycle at its current frame.
pub fn animate_monsters(
    time: Res<Time<Virtual>>,
    mut actors: Query<(
        &Monster,
        &MonsterVisual,
        &mut MonsterPlayback,
        Option<&CycleRequest>,
        &Children,
    )>,
    models: Res<Assets<MonsterModel>>,
    roots: Query<&Children, With<ModelRoot>>,
    mut cycles: Query<(&ModelCycle, &mut MorphWeights, &mut Visibility)>,
) {
    for (monster, visual, mut playback, request, children) in &mut actors {
        let Some(model) = models.get(&visual.0) else { continue };
        let request = request.cloned().unwrap_or_else(|| CycleRequest::looping("IDLE"));
        let current = playback.playing.as_ref().is_some_and(|(playing, _)| *playing == request);
        let name = if model.cycles.contains_key(&request.cycle) { &request.cycle } else { "IDLE" };
        let Some(cycle) = model.cycles.get(name) else { continue };
        // The previous tick already applied a completed one-shot's final pose.
        // Derive this from its clock rather than storing another completion flag.
        if current
            && request.once
            && playback.playing.as_ref().is_some_and(|(_, played)| {
                played.elapsed_secs() * cycle.frames_per_second >= cycle.frame_count as f32 - 1.0
            })
        {
            continue;
        }
        if !current {
            // The first cycle starts part-way in.
            let mut played = Stopwatch::new();
            if playback.playing.is_none() {
                played.set_elapsed(Duration::from_secs_f32(first_phase(monster)));
            }
            playback.playing = Some((request.clone(), played));
        }
        let Some((_, played)) = &mut playback.playing else { continue };
        let frames = cycle.frame_count as f32;
        let elapsed = played.tick(time.delta()).elapsed_secs() * cycle.frames_per_second;
        let frame = if request.once { elapsed.min(frames - 1.0) } else { elapsed % frames };
        for root in roots.iter_many(children).matched() {
            for child in root.iter() {
                let Ok((cycle_name, mut weights, mut visibility)) = cycles.get_mut(child) else {
                    continue;
                };
                let active = cycle_name.0 == name;
                visibility.set_if_neq(visible_if(active));
                if !active {
                    continue;
                }
                cycle.write_weights(frame, weights.weights_mut());
            }
        }
    }
}

/// The sky light an actor in sight of the sun receives (Whizzardry
/// `Monster::Update`).
const SUNLIT: f32 = 0.75;
/// Retail steps the sky scale by 0.05 every 25 ms.
const SKY_SCALE_PER_SECOND: f32 = 2.0;

/// Light actors as retail does models without baked light: sky light while
/// they can see the `SUN` light, scaled by each material's ambient, plus the
/// level's offset, nearby lights and the material's emission. Retail adds
/// point lights per vertex with N·L; here they light the whole actor by
/// their falloff at its centre.
pub fn light_monsters(
    time: Res<Time>,
    clock: Res<GameClock>,
    levels: Query<(Entity, &LevelAppearance)>,
    geometry: Res<Assets<LevelGeometry>>,
    spatial: SpatialQuery,
    mut actors: Query<(&GlobalTransform, &mut MonsterLight, &mut ActorLight)>,
) {
    let Some((level_entity, level)) =
        levels.iter().find_map(|(e, l)| geometry.get(&l.0).map(|g| (e, g)))
    else {
        return;
    };
    let sky = level.environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(clock.phase()));
    for (transform, mut sampled, mut light) in &mut actors {
        let centre = transform.translation();
        let (target, fixed) = match sampled.lit_at {
            Some((at, target, fixed)) if at == centre && !geometry.is_changed() => (target, fixed),
            _ => {
                let target =
                    if sunlit(&spatial, level_entity, level, centre) { SUNLIT } else { 0.0 };
                let fixed = level.lighting.at(centre, None);
                sampled.lit_at = Some((centre, target, fixed));
                (target, fixed)
            }
        };
        if sampled.sky_scale != target {
            let step = SKY_SCALE_PER_SECOND * time.delta_secs();
            sampled.sky_scale += (target - sampled.sky_scale).clamp(-step, step);
        }
        light.set_if_neq(ActorLight { sky: sky * sampled.sky_scale, fixed });
    }
}

/// Whether the level's `SUN` light is in sight from `at`, past the level's
/// own geometry. A level without one has no sunlight.
pub fn sunlit(
    spatial: &SpatialQuery,
    level_entity: Entity,
    level: &LevelGeometry,
    at: Vec3,
) -> bool {
    level.lighting.sun.is_some_and(|sun| {
        let Ok(direction) = Dir3::new(sun - at) else { return true };
        spatial
            .cast_ray_predicate(
                at,
                direction,
                at.distance(sun),
                true,
                &SpatialQueryFilter::default(),
                &|hit| hit == level_entity,
            )
            .is_none()
    })
}

/// Walkers play WALK while moving and IDLE when still.
pub fn choose_walker_cycles(
    mut commands: Commands,
    walkers: Query<
        (Entity, &avian3d::prelude::LinearVelocity, Option<&CycleRequest>),
        With<Monster>,
    >,
) {
    for (entity, velocity, current) in &walkers {
        // A death or another play-once cycle is not interrupted.
        if current.is_some_and(|c| c.once) {
            continue;
        }
        let cycle = if velocity.0.with_y(0.0).length() > 0.3 { "WALK" } else { "IDLE" };
        if current.is_none_or(|c| c.cycle != cycle) {
            commands.entity(entity).insert(CycleRequest::looping(cycle));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        monster::{MonsterBody, MonsterCycleModel, MonsterPart},
        surface::Surface,
    };
    use crate::game::world::population::ScaledBody;

    #[test]
    fn instances_share_geometry_but_keep_phase_restart_and_final_frame_hold() {
        let mut app = App::new();
        app.init_resource::<Assets<MonsterModel>>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Time<Virtual>>()
            .add_systems(Update, (build_monster_instances, animate_monsters).chain());
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let mesh =
            app.world_mut().resource_mut::<Assets<Mesh>>().add(Mesh::from(Cuboid::default()));
        let mut cycles = HashMap::new();
        for (name, fps) in [("IDLE", 1.), ("DIE", 2.)] {
            cycles.insert(
                name.into(),
                MonsterCycleModel {
                    frames_per_second: fps,
                    frame_count: 3,
                    parts: vec![MonsterPart {
                        mesh: mesh.clone(),
                        bounds: bevy::camera::primitives::Aabb::default(),
                        surface: Surface {
                            material: material.clone(),
                            ambient: Vec3::ONE,
                            emission: 0.,
                            animation: default(),
                        },
                    }],
                },
            );
        }
        let model =
            app.world_mut().resource_mut::<Assets<MonsterModel>>().add(MonsterModel { cycles });
        let actors: Vec<_> = (0..2)
            .map(|location| {
                app.world_mut()
                    .spawn((
                        Monster {
                            enchantments: default(),
                            location,
                            species: 0,
                            hp: 1,
                            max_hp: 1,
                            stamina: 1,
                            max_stamina: 1,
                            conditions: vec![],
                            body: ScaledBody::new(&MonsterBody::default(), 0),
                        },
                        MonsterVisual(model.clone()),
                    ))
                    .id()
            })
            .collect();
        app.update();
        let cycle_entity = |app: &mut App, actor, name: &str| {
            let world = app.world_mut();
            world
                .query::<(Entity, &ModelCycle, &ChildOf)>()
                .iter(world)
                .find(|(_, cycle, parent)| {
                    cycle.0 == name
                        && world
                            .get::<ChildOf>(parent.parent())
                            .is_some_and(|root| root.parent() == actor)
                })
                .unwrap()
                .0
        };
        let a = cycle_entity(&mut app, actors[0], "IDLE");
        let b = cycle_entity(&mut app, actors[1], "IDLE");
        assert_eq!(app.world().get::<MorphWeights>(a).unwrap().weights(), &[1., 0., 0.]);
        assert_ne!(
            app.world().get::<MorphWeights>(a).unwrap().weights(),
            app.world().get::<MorphWeights>(b).unwrap().weights()
        );
        assert!(app.world_mut().query::<&Mesh3d>().iter(app.world()).all(|m| m.0 == mesh));
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 1);
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs_f32(2.5));
        app.update();
        assert_eq!(app.world().get::<MorphWeights>(a).unwrap().weights(), &[0., 0., 1.]);
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs_f32(0.5));
        app.update();
        assert_eq!(app.world().get::<MorphWeights>(a).unwrap().weights(), &[1., 0., 0.]);
        app.world_mut()
            .entity_mut(actors[0])
            .insert(CycleRequest { cycle: "DIE".into(), once: true });
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs_f32(0.25));
        app.update();
        let death = cycle_entity(&mut app, actors[0], "DIE");
        assert_eq!(app.world().get::<MorphWeights>(death).unwrap().weights(), &[0.5, 0.5, 0.]);
        assert_eq!(*app.world().get::<Visibility>(a).unwrap(), Visibility::Hidden);
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs(2));
        app.update();
        assert_eq!(app.world().get::<MorphWeights>(death).unwrap().weights(), &[0., 0., 1.]);
        let changed_at = |world: &mut World| {
            world.query::<Ref<MorphWeights>>().get(world, death).unwrap().last_changed()
        };
        let held_at = changed_at(app.world_mut());
        app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs(1));
        app.update();
        assert_eq!(
            changed_at(app.world_mut()),
            held_at,
            "held poses must not upload weights again"
        );
    }
}
