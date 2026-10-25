//! Navigation over the active level: a Rerecast-baked navmesh used by a
//! Landmass archipelago.
//!
//! Landmass chooses each agent's desired velocity (path following and local
//! avoidance); a Tnua walk controller turns that into motion under Avian
//! collision; the resulting velocity goes back to Landmass on the next step.
//! The party is a Landmass character: an obstacle agents avoid, not an agent.
//!
//! The navmesh is baked from the same triangles as the level collider by
//! Bevy's asset processor ([`asset`]), which caches the result.

pub mod asset;
pub mod bake;
pub mod links;

use avian3d::prelude::*;
use bake::NavmeshBakeSettings;
use bevy::asset::processor::LoadTransformAndSave;
use bevy::asset::{AssetApp, LoadState};
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_landmass::{NavMeshHandle, PointSampleDistance3d};
use bevy_tnua::builtins::TnuaBuiltinWalk;
use bevy_tnua::prelude::*;

use super::level::{AwaitingNavmesh, Level};
use crate::game::party::body::{FLOAT_HEIGHT, PartyBody};
use crate::retail::source::NAVMESH_EXTENSION;

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn finish(&self, app: &mut App) {
        // After Landmass registers `NavMesh3d`.
        app.init_asset::<asset::LevelCollision>()
            .init_asset::<asset::BakedNavmesh>()
            .register_asset_loader(asset::LevelCollisionLoader)
            .register_asset_loader(asset::NavmeshLoader)
            .register_asset_processor(LoadTransformAndSave::<
                asset::LevelCollisionLoader,
                asset::BakeNavmesh,
                asset::NavmeshSaver,
            >::new(asset::BakeNavmesh, asset::NavmeshSaver))
            .set_default_asset_processor::<LoadTransformAndSave<
                asset::LevelCollisionLoader,
                asset::BakeNavmesh,
                asset::NavmeshSaver,
            >>(NAVMESH_EXTENSION);
    }

    fn build(&self, app: &mut App) {
        app.add_plugins(Landmass3dPlugin::default())
            .add_plugins(TnuaControllerPlugin::<WalkerMotion>::new(FixedUpdate))
            .add_systems(
                Update,
                (
                    install_navmesh,
                    links::spawn_level_links,
                    links::find_door_links,
                    links::sync_door_links,
                ),
            )
            // Each fixed step: Landmass plans (FixedPreUpdate), walkers feed
            // Tnua (FixedUpdate), Avian moves them (FixedPostUpdate), and
            // the resulting velocities go back to Landmass.
            .add_systems(
                FixedUpdate,
                (links::use_reached_links, links::finish_crossings, steer_walkers)
                    .chain()
                    .in_set(TnuaUserControlsSystems),
            )
            .add_systems(
                FixedPostUpdate,
                (track_party_character, sync_walker_velocity).after(PhysicsSystems::Writeback),
            );
    }
}

/// Movement of a navigating world actor.
#[derive(TnuaScheme)]
#[scheme(basis = TnuaBuiltinWalk)]
pub enum WalkerMotion {}

/// On a level root: the level's archipelago, once its navmesh is installed.
#[derive(Component, Debug)]
pub struct LevelNavigation {
    pub archipelago: Entity,
}

/// A world actor moved by Landmass desired velocities through Tnua.
/// `speed` is the walk config's top speed, used to scale desired velocity
/// into Tnua's normalized motion input.
#[derive(Component, Debug)]
pub struct Walker {
    pub speed: f32,
    /// How far the body's origin floats above its feet.
    pub float_height: f32,
}

/// Install a level's navmesh under its root once it has loaded. The navmesh
/// comes from the asset processor's cache (baked on first use).
fn install_navmesh(
    mut commands: Commands,
    levels: Query<(Entity, &Level), With<AwaitingNavmesh>>,
    assets: Res<AssetServer>,
) {
    for (root, level) in &levels {
        match assets.get_load_state(&level.navmesh) {
            Some(LoadState::Loaded) => {}
            Some(LoadState::Failed(error)) => {
                // The level stays without navigation; nothing will path.
                error!("navmesh unavailable: {error}");
                commands.entity(root).remove::<AwaitingNavmesh>();
                continue;
            }
            _ => continue,
        }
        let mut options =
            ArchipelagoOptions::from_agent_radius(NavmeshBakeSettings::default().agent_radius);
        // Actor origins float above the ground; sample the navmesh beneath.
        options.point_sample_distance = PointSampleDistance3d {
            horizontal_distance: 1.0,
            distance_above: 0.5,
            distance_below: FLOAT_HEIGHT + 0.6,
            vertical_preference_ratio: 2.0,
            animation_link_max_vertical_distance: 1.0,
        };
        let archipelago = commands
            .spawn((Name::new("Archipelago"), Archipelago3d::new(options), ChildOf(root)))
            .id();
        commands.spawn((
            Name::new("Level navmesh"),
            Transform::default(),
            Island3dBundle {
                island: Island,
                archipelago_ref: ArchipelagoRef3d::new(archipelago),
                nav_mesh: NavMeshHandle(level.navmesh.clone()),
            },
            ChildOf(root),
        ));
        commands.entity(root).remove::<AwaitingNavmesh>().insert(LevelNavigation { archipelago });
        info!("navmesh installed");
    }
}

/// Register the party with the archipelago and report its velocity.
fn track_party_character(
    mut commands: Commands,
    navigation: Query<&LevelNavigation>,
    mut party: Query<(Entity, &LinearVelocity, Option<&mut Velocity3d>), With<PartyBody>>,
) {
    let (Ok(navigation), Ok((entity, velocity, tracked))) =
        (navigation.single(), party.single_mut())
    else {
        return;
    };
    match tracked {
        Some(mut tracked) => tracked.velocity = velocity.0,
        None => {
            commands.entity(entity).insert((
                Character3dBundle {
                    character: default(),
                    settings: CharacterSettings {
                        radius: 500.0 / crate::retail::space::UNITS_PER_METRE,
                    },
                    archipelago_ref: ArchipelagoRef3d::new(navigation.archipelago),
                },
                Velocity3d { velocity: velocity.0 },
            ));
        }
    }
}

/// Feed each walker's physical velocity back to Landmass.
fn sync_walker_velocity(mut walkers: Query<(&LinearVelocity, &mut Velocity3d), With<Walker>>) {
    for (linear, mut velocity) in &mut walkers {
        velocity.velocity = linear.0;
    }
}

/// Turn Landmass's desired velocity into Tnua walk input.
/// Feed Landmass's desired velocity to Tnua; while crossing a doorway link,
/// head straight for its far side instead.
fn steer_walkers(
    mut walkers: Query<(
        &Walker,
        &AgentDesiredVelocity3d,
        Option<&links::Crossing>,
        &Transform,
        &mut TnuaController<WalkerMotion>,
    )>,
) {
    for (walker, desired, crossing, transform, mut controller) in &mut walkers {
        let velocity = match crossing {
            Some(crossing) => {
                (crossing.to - transform.translation).with_y(0.0).normalize_or_zero()
                    * walker.speed
                    * 0.8
            }
            None => desired.velocity().with_y(0.0),
        };
        controller.initiate_action_feeding();
        controller.basis = TnuaBuiltinWalk {
            desired_motion: velocity / walker.speed,
            desired_forward: Dir3::new(velocity).ok(),
        };
    }
}
