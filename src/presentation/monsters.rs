//! Showing and animating monster models.
//!
//! An actor with [`MonsterAppearance`] gets the model's cycles as children,
//! one visible at a time. Each instance animates its own copy of the cycle
//! meshes, interpolating vertex positions between frames as retail does with
//! smooth animation on.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};

use crate::content::level::LevelGeometry;
use crate::content::monster::MonsterModel;
use crate::game::time::GameClock;
use crate::game::world::level::Level;
use crate::game::world::population::Monster;

/// What an actor looks like, and how far its origin floats above its feet.
#[derive(Component, Debug)]
#[require(Visibility)]
pub struct MonsterAppearance {
    pub model: Handle<MonsterModel>,
    pub float_height: f32,
    /// Seconds into its first cycle it starts at.
    pub phase: f32,
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

/// The built model of one actor.
#[derive(Component, Debug)]
pub struct MonsterInstance {
    /// Per cycle: its root entity and this instance's mesh copies.
    cycles: HashMap<String, (Entity, Vec<Handle<Mesh>>)>,
    playing: Option<(CycleRequest, f32)>,
    /// How much sky light reaches the actor: retail fades it between 0.75
    /// in sight of the level's `SUN` light and 0 out of it.
    sky_scale: f32,
}

impl MonsterInstance {
    /// Whether a play-once cycle has reached its last frame (retail
    /// `CYCLE_COMPLETE`).
    pub fn is_complete(&self, model: &MonsterModel, now: f32) -> bool {
        let Some((request, started)) = &self.playing else { return false };
        let Some(cycle) = model.cycle(&request.cycle) else { return true };
        request.once
            && (now - started) * cycle.frames_per_second >= (cycle.frame_count() - 1) as f32
    }
}

pub fn build_monster_instances(
    mut commands: Commands,
    actors: Query<(Entity, &MonsterAppearance), Without<MonsterInstance>>,
    models: Res<Assets<MonsterModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (actor, appearance) in &actors {
        let Some(model) = models.get(&appearance.model) else { continue };
        // Retail models face -Z; after the Z mirror that is +Z, so turn them
        // to face the actor's forward (-Z). Feet are at the model's origin.
        let model_root = commands
            .spawn((
                Transform {
                    translation: Vec3::NEG_Y * appearance.float_height,
                    rotation: Quat::from_rotation_y(std::f32::consts::PI),
                    scale: Vec3::splat(model.scale),
                },
                Visibility::default(),
                ChildOf(actor),
            ))
            .id();
        let mut cycles = HashMap::new();
        for (name, cycle) in &model.cycles {
            let cycle_root = commands
                .spawn((
                    Name::new(name.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(model_root),
                ))
                .id();
            let mut copies = Vec::new();
            for part in &cycle.parts {
                let Some(mesh) = meshes.get(&part.mesh).cloned() else { continue };
                let copy = meshes.add(mesh);
                commands.spawn((
                    Mesh3d(copy.clone()),
                    MeshMaterial3d(part.material.clone()),
                    ChildOf(cycle_root),
                ));
                copies.push(copy);
            }
            cycles.insert(name.clone(), (cycle_root, copies));
        }
        commands.entity(actor).insert(MonsterInstance { cycles, playing: None, sky_scale: SUNLIT });
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
    mut actors: Query<(&MonsterAppearance, &mut MonsterInstance, Option<&CycleRequest>)>,
    models: Res<Assets<MonsterModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut visibility: Query<&mut Visibility>,
) {
    let now = time.elapsed_secs();
    for (appearance, mut instance, request) in &mut actors {
        let Some(model) = models.get(&appearance.model) else { continue };
        let request = request.cloned().unwrap_or_else(|| CycleRequest::looping("IDLE"));
        let name = if model.cycles.contains_key(&request.cycle) { &request.cycle } else { "IDLE" };
        let Some(cycle) = model.cycles.get(name) else { continue };
        if instance.playing.as_ref().is_none_or(|(playing, _)| *playing != request) {
            for (cycle_name, (root, _)) in &instance.cycles {
                if let Ok(mut shown) = visibility.get_mut(*root) {
                    *shown =
                        if cycle_name == name { Visibility::Inherited } else { Visibility::Hidden };
                }
            }
            // The first cycle starts part-way in.
            let started = if instance.playing.is_none() { now - appearance.phase } else { now };
            instance.playing = Some((request.clone(), started));
        }
        let started = instance.playing.as_ref().map_or(now, |(_, started)| *started);
        let frames = cycle.frame_count() as f32;
        let elapsed = (now - started) * cycle.frames_per_second;
        let frame = if request.once { elapsed.min(frames - 1.0) } else { elapsed % frames };
        let positions = cycle.positions_at(frame);
        let Some((_, copies)) = instance.cycles.get(name) else { continue };
        for (part, copy) in cycle.parts.iter().zip(copies) {
            if let Some(mut mesh) = meshes.get_mut(copy) {
                let moved: Vec<[f32; 3]> =
                    part.source.iter().map(|&v| positions[v as usize].to_array()).collect();
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, moved);
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
    mut actors: Query<(&GlobalTransform, &MonsterAppearance, &mut MonsterInstance)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some((level_entity, level)) =
        levels.iter().find_map(|(e, l)| geometry.get(&l.geometry).map(|g| (e, g)))
    else {
        return;
    };
    let sky = level.environment.as_ref().map_or(Vec3::ZERO, |e| e.sky_light(clock.phase()));
    for (transform, appearance, mut instance) in &mut actors {
        let Some(model) = models.get(&appearance.model) else { continue };
        let centre = transform.translation();
        let sunlit = level.lighting.sun.is_some_and(|sun| {
            let Ok(direction) = Dir3::new(sun - centre) else { return true };
            spatial
                .cast_ray_predicate(
                    centre,
                    direction,
                    centre.distance(sun),
                    true,
                    &SpatialQueryFilter::default(),
                    &|hit| hit == level_entity,
                )
                .is_none()
        });
        let target = if sunlit { SUNLIT } else { 0.0 };
        let step = SKY_SCALE_PER_SECOND * time.delta_secs();
        instance.sky_scale += (target - instance.sky_scale).clamp(-step, step);
        let fixed = level.lighting.at(centre, None);
        let Some(playing) = instance.playing.as_ref().map(|(request, _)| request.cycle.clone())
        else {
            continue;
        };
        let Some(cycle) = model.cycle(&playing) else { continue };
        let Some((_, copies)) =
            instance.cycles.get(&playing).or_else(|| instance.cycles.get("IDLE"))
        else {
            continue;
        };
        for (part, copy) in cycle.parts.iter().zip(copies) {
            let light =
                sky * instance.sky_scale * part.ambient + fixed + Vec3::splat(part.emission);
            let colour = light.extend(1.0).to_array();
            let Some(mut mesh) = meshes.get_mut(copy) else { continue };
            let count = part.source.len();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![colour; count]);
        }
    }
}

/// Walkers play WALK while moving and IDLE when still.
pub fn choose_walker_cycles(
    mut commands: Commands,
    walkers: Query<
        (Entity, &avian3d::prelude::LinearVelocity, Option<&CycleRequest>),
        With<MonsterAppearance>,
    >,
) {
    for (entity, velocity, current) in &walkers {
        let cycle = if velocity.0.with_y(0.0).length() > 0.3 { "WALK" } else { "IDLE" };
        if current.is_none_or(|c| c.cycle != cycle) {
            commands.entity(entity).insert(CycleRequest::looping(cycle));
        }
    }
}
