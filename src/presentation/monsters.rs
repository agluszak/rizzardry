//! Showing and animating monster models.
//!
//! Each [`Monster`] gets its model's cycles as children, one visible at a
//! time. Each instance animates its own copy of the cycle
//! meshes, interpolating vertex positions between frames as retail does with
//! smooth animation on.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::Stopwatch;

use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};

use crate::content::level::LevelGeometry;
use crate::content::monster::MonsterModel;
use crate::content::species::MonsterCatalog;
use crate::game::data::GameData;
use crate::game::time::GameClock;
use crate::game::world::level::Level;
use crate::game::world::navigation::Walker;
use crate::game::world::population::Monster;

/// Seconds into its first cycle a monster starts at. Retail starts each
/// idle at a random frame; here it varies by location.
fn first_phase(monster: &Monster) -> f32 {
    (monster.location.unsigned_abs() % 97) as f32 * 0.37
}

/// The cycle an actor should play. Gameplay sets it; a changed request
/// restarts playback.
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
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
    assets: Res<AssetServer>,
) {
    let Some(catalog) = data.and_then(|d| species.get(&d.species)) else { return };
    for (entity, monster) in &actors {
        let Some(kind) = catalog.0.get(monster.species as usize) else { continue };
        let model = assets.load(format!("retail://data/monsters/{}.mls", kind.model));
        commands.entity(entity).insert(MonsterVisual(model));
    }
}

/// The built model of one actor.
#[derive(Component, Debug)]
pub struct MonsterInstance {
    /// The cycle shown, and how long it has played.
    playing: Option<(CycleRequest, Stopwatch)>,
    /// How much sky light reaches the actor: retail fades it between 0.75
    /// in sight of the level's `SUN` light and 0 out of it.
    sky_scale: f32,
}

/// The hierarchy itself records which visual entities were successfully built.
#[derive(Component, Debug)]
pub struct ModelRoot;

#[derive(Component, Debug)]
pub struct ModelCycle(String);

#[derive(Component, Debug)]
pub struct ModelPart(usize);

pub fn build_monster_instances(
    mut commands: Commands,
    actors: Query<(Entity, &Monster, &MonsterVisual, Option<&Walker>), Without<MonsterInstance>>,
    models: Res<Assets<MonsterModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
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
        for (name, cycle) in &model.cycles {
            let cycle_root = commands
                .spawn((
                    Name::new(name.clone()),
                    ModelCycle(name.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(model_root),
                ))
                .id();
            for (index, part) in cycle.parts.iter().enumerate() {
                let Some(mesh) = meshes.get(&part.mesh).cloned() else { continue };
                let copy = meshes.add(mesh);
                commands.spawn((
                    ModelPart(index),
                    Mesh3d(copy),
                    MeshMaterial3d(part.material.clone()),
                    ChildOf(cycle_root),
                ));
            }
        }
        commands
            .entity(actor)
            .insert((MonsterInstance { playing: None, sky_scale: SUNLIT }, Visibility::default()));
    }
}

/// Keep the model's feet on the ground when a monster gains a walking body
/// that floats over them.
pub fn follow_float_height(
    actors: Query<&Walker>,
    mut roots: Query<(&ChildOf, &mut Transform), With<ModelRoot>>,
) {
    for (owner, mut transform) in &mut roots {
        if let Ok(walker) = actors.get(owner.parent()) {
            transform.translation = Vec3::NEG_Y * walker.float_height;
        }
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
        &mut MonsterInstance,
        Option<&CycleRequest>,
        &Children,
    )>,
    models: Res<Assets<MonsterModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
    roots: Query<&Children, With<ModelRoot>>,
    mut cycles: Query<(&ModelCycle, &Children, &mut Visibility)>,
    parts: Query<(&ModelPart, &Mesh3d)>,
) {
    for (monster, visual, mut instance, request, children) in &mut actors {
        let Some(model) = models.get(&visual.0) else { continue };
        let request = request.cloned().unwrap_or_else(|| CycleRequest::looping("IDLE"));
        let name = if model.cycles.contains_key(&request.cycle) { &request.cycle } else { "IDLE" };
        let Some(cycle) = model.cycles.get(name) else { continue };
        if instance.playing.as_ref().is_none_or(|(playing, _)| *playing != request) {
            // The first cycle starts part-way in.
            let mut played = Stopwatch::new();
            if instance.playing.is_none() {
                played.set_elapsed(Duration::from_secs_f32(first_phase(monster)));
            }
            instance.playing = Some((request.clone(), played));
        }
        let Some((_, played)) = &mut instance.playing else { continue };
        let frames = cycle.frame_count() as f32;
        let elapsed = played.tick(time.delta()).elapsed_secs() * cycle.frames_per_second;
        let frame = if request.once { elapsed.min(frames - 1.0) } else { elapsed % frames };
        let positions = cycle.positions_at(frame);
        for root in children.iter().filter_map(|e| roots.get(e).ok()) {
            for child in root.iter() {
                let Ok((cycle_name, children, mut visibility)) = cycles.get_mut(child) else {
                    continue;
                };
                let active = cycle_name.0 == name;
                visibility.set_if_neq(if active {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
                if !active {
                    continue;
                }
                for entity in children.iter() {
                    let Ok((index, copy)) = parts.get(entity) else { continue };
                    let Some(part) = cycle.parts.get(index.0) else { continue };
                    if let Some(mut mesh) = meshes.get_mut(&copy.0) {
                        let moved: Vec<[f32; 3]> =
                            part.source.iter().map(|&v| positions[v as usize].to_array()).collect();
                        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, moved);
                    }
                }
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
    levels: Query<(Entity, &Level)>,
    geometry: Res<Assets<LevelGeometry>>,
    models: Res<Assets<MonsterModel>>,
    spatial: SpatialQuery,
    mut actors: Query<(&GlobalTransform, &MonsterVisual, &mut MonsterInstance, &Children)>,
    roots: Query<&Children, With<ModelRoot>>,
    cycles: Query<(&ModelCycle, &Children, &Visibility)>,
    parts: Query<(&ModelPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some((level_entity, level)) =
        levels.iter().find_map(|(e, l)| geometry.get(&l.geometry).map(|g| (e, g)))
    else {
        return;
    };
    let sky = level.environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(clock.phase()));
    for (transform, visual, mut instance, children) in &mut actors {
        let Some(model) = models.get(&visual.0) else { continue };
        let centre = transform.translation();
        let target = if sunlit(&spatial, level_entity, level, centre) { SUNLIT } else { 0.0 };
        let step = SKY_SCALE_PER_SECOND * time.delta_secs();
        instance.sky_scale += (target - instance.sky_scale).clamp(-step, step);
        let fixed = level.lighting.at(centre, None);
        for root in children.iter().filter_map(|e| roots.get(e).ok()) {
            for child in root.iter() {
                let Ok((cycle_name, children, visibility)) = cycles.get(child) else { continue };
                if *visibility == Visibility::Hidden {
                    continue;
                }
                let Some(cycle) = model.cycles.get(&cycle_name.0) else { continue };
                for entity in children.iter() {
                    let Ok((index, copy)) = parts.get(entity) else { continue };
                    let Some(part) = cycle.parts.get(index.0) else { continue };
                    let light = sky * instance.sky_scale * part.ambient
                        + fixed
                        + Vec3::splat(part.emission);
                    let colour = light.extend(1.0).to_array();
                    let Some(mut mesh) = meshes.get_mut(&copy.0) else { continue };
                    let count = mesh.count_vertices();
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![colour; count]);
                }
            }
        }
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
    use crate::content::monster::{MonsterCycleModel, MonsterPart};
    use crate::game::world::population::ScaledBody;
    use bevy::mesh::VertexAttributeValues;
    use bevy::platform::collections::HashMap;

    #[test]
    fn sparse_render_parts_keep_their_source_identity_and_cycle_fallback() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<MonsterModel>>()
            .add_systems(Update, (build_monster_instances, animate_monsters).chain());
        let mesh = Mesh::from(Cuboid::default());
        let count = mesh.count_vertices();
        let good_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let parts = vec![
            MonsterPart {
                mesh: default(),
                material: default(),
                ambient: Vec3::ZERO,
                emission: 0.0,
                source: vec![0],
            },
            MonsterPart {
                mesh: good_mesh,
                material: default(),
                ambient: Vec3::ONE,
                emission: 0.0,
                source: vec![1; count],
            },
        ];
        let mut cycles = HashMap::new();
        cycles.insert(
            "IDLE".into(),
            MonsterCycleModel {
                frames_per_second: 1.0,
                parts,
                frames: vec![vec![Vec3::ZERO, Vec3::new(7.0, 8.0, 9.0)]],
            },
        );
        let model =
            app.world_mut().resource_mut::<Assets<MonsterModel>>().add(MonsterModel { cycles });
        let placement = Transform::from_xyz(10.0, 2.0, 5.0);
        let actor = app
            .world_mut()
            .spawn((
                Monster {
                    location: 0,
                    species: 0,
                    hp: 1,
                    max_hp: 1,
                    stamina: 1,
                    max_stamina: 1,
                    conditions: vec![],
                    body: ScaledBody { scale: 2.0, radius: 1.0, eye_height: 1.0, speed: 1.0 },
                },
                placement,
                MonsterVisual(model),
                CycleRequest::looping("MISSING"),
            ))
            .id();
        app.update();
        assert_eq!(*app.world().get::<Transform>(actor).unwrap(), placement);
        let (part_entity, source, copy) =
            app.world_mut().query::<(Entity, &ModelPart, &Mesh3d)>().single(app.world()).unwrap();
        assert_eq!(source.0, 1, "missing parts cannot shift the remaining source correspondence");
        let mesh = app.world().resource::<Assets<Mesh>>().get(&copy.0).unwrap();
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions")
        };
        assert_eq!(positions, &vec![[7.0, 8.0, 9.0]; count]);
        let (_, visibility) =
            app.world_mut().query::<(&ModelCycle, &Visibility)>().single(app.world()).unwrap();
        assert_eq!(*visibility, Visibility::Inherited);
        app.world_mut().despawn(part_entity);
        app.update();
        assert_eq!(app.world_mut().query::<&ModelPart>().iter(app.world()).count(), 0);
    }
}
