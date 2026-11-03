//! Showing and animating monster models.
//!
//! Each [`Monster`] gets its model's cycles as children, one visible at a
//! time. Instances share immutable morph meshes and own their playback weights;
//! lighting and texture animation live on their materials.

use std::time::Duration;

use super::surfaces::{ActorLight, LightSample, SkyResponse};
use bevy::camera::visibility::NoAutoAabb;
use bevy::mesh::morph::{MeshMorphWeights, MorphWeights};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::time::Stopwatch;

use avian3d::prelude::LinearVelocity;

use crate::content::monster::MonsterModel;
use crate::game::world::navigation::Walker;
use crate::game::world::population::Monster;
use crate::presentation::screen_art::visible_if;

/// The cycle a dying monster plays.
const DIE: &str = "DIE";

/// Seconds into its first cycle a monster starts at. Retail starts each
/// idle at a random frame; here it varies by location.
fn first_phase(monster: &Monster) -> f32 {
    (monster.location.unsigned_abs() % 97) as f32 * 0.37
}

/// Optional visual content retained while its model loads and plays. It is
/// never captured in a level record or used to size an actor's gameplay body.
#[derive(Component, Debug)]
pub struct MonsterVisual(pub Handle<MonsterModel>);

/// A durable NPC keeps its rules, but its map-owned visual embodiment ends.
pub fn remove_monster_visuals(
    removed: On<Remove<crate::game::world::level::LevelOwned>>,
    actors: Query<(), With<Monster>>,
    mut commands: Commands,
) {
    if actors.contains(removed.entity) {
        commands
            .entity(removed.entity)
            .try_remove::<(MonsterVisual, MonsterPlayback, LightSample, SkyResponse)>();
    }
}

pub fn request_monster_visuals(
    mut commands: Commands,
    actors: Query<(Entity, &Monster), Without<MonsterVisual>>,
    content: If<Res<crate::game::data::Catalogs>>,
    assets: Res<AssetServer>,
) {
    let catalog = &content.species;
    for (entity, monster) in &actors {
        let kind = &catalog[monster.species];
        let model = assets.load(crate::content::paths::monster(&kind.model));
        commands.entity(entity).insert(MonsterVisual(model));
    }
}

/// The playback of one actor's built model.
#[derive(Component, Debug)]
pub struct MonsterPlayback {
    /// The cycle shown, and how long it has played.
    playing: Option<(&'static str, Stopwatch)>,
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
        // Lit independently of its playback: a settled corpse is still relit.
        commands.entity(actor).insert((
            MonsterPlayback { playing: None },
            LightSample::default(),
            SkyResponse::fading(),
            ActorLight::default(),
            Visibility::default(),
        ));
    }
}

/// The cycle an actor should be playing: its death, which plays once and stays
/// on its last frame; otherwise WALK while moving and IDLE when still.
fn wanted_cycle(
    vitals: &crate::game::characters::Vitals,
    velocity: Option<&LinearVelocity>,
) -> &'static str {
    if vitals.is_dead() {
        DIE
    } else if velocity.is_some_and(|v| v.0.with_y(0.0).length() > 0.3) {
        "WALK"
    } else {
        "IDLE"
    }
}

/// Show each actor's wanted cycle at its current frame. A different cycle
/// restarts playback; the same one continues it.
pub fn animate_monsters(
    time: Res<Time<Virtual>>,
    mut actors: Query<(
        (&Monster, &crate::game::characters::Vitals),
        &MonsterVisual,
        &mut MonsterPlayback,
        Option<&LinearVelocity>,
        &Children,
    )>,
    models: Res<Assets<MonsterModel>>,
    roots: Query<&Children, With<ModelRoot>>,
    mut cycles: Query<(&ModelCycle, &mut MorphWeights, &mut Visibility)>,
) {
    for ((monster, vitals), visual, mut playback, velocity, children) in &mut actors {
        let Some(model) = models.get(&visual.0) else { continue };
        let wanted = wanted_cycle(vitals, velocity);
        let once = wanted == DIE;
        let current = playback.playing.as_ref().is_some_and(|(playing, _)| *playing == wanted);
        let name = if model.cycles.contains_key(wanted) { wanted } else { "IDLE" };
        let Some(cycle) = model.cycles.get(name) else { continue };
        // The previous tick already applied a completed one-shot's final pose.
        // Derive this from its clock rather than storing another completion flag.
        if current
            && once
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
            playback.playing = Some((wanted, played));
        }
        let Some((_, played)) = &mut playback.playing else { continue };
        let frames = cycle.frame_count as f32;
        let elapsed = played.tick(time.delta()).elapsed_secs() * cycle.frames_per_second;
        let frame = if once { elapsed.min(frames - 1.0) } else { elapsed % frames };
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
