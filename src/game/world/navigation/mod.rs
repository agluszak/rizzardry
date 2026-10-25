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

use avian3d::prelude::*;
use bake::NavmeshBakeSettings;
use bevy::asset::AssetApp;
use bevy::asset::processor::LoadTransformAndSave;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_landmass::{NavMeshHandle, PointSampleDistance3d};
use bevy_tnua::builtins::TnuaBuiltinWalk;
use bevy_tnua::prelude::*;

use super::level::ActiveLevel;
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
            .add_message::<NavmeshReady>()
            .add_systems(Update, (install_navmesh, track_party_character))
            .add_systems(
                Update,
                (sync_walker_velocity, steer_walkers.in_set(TnuaUserControlsSystems)),
            );
    }
}

/// Movement of a navigating world actor.
#[derive(TnuaScheme)]
#[scheme(basis = TnuaBuiltinWalk)]
pub enum WalkerMotion {}

/// The active level's archipelago, once its navmesh exists.
#[derive(Resource, Debug)]
pub struct LevelNavigation {
    pub archipelago: Entity,
}

/// Sent when the active level's navmesh has been baked and installed.
#[derive(Message, Debug, Clone, Copy)]
pub struct NavmeshReady;

/// A world actor moved by Landmass desired velocities through Tnua.
/// `speed` is the walk config's top speed, used to scale desired velocity
/// into Tnua's normalized motion input.
#[derive(Component, Debug)]
pub struct Walker {
    pub speed: f32,
}

/// Install the active level's navmesh once it and the level are loaded.
/// The navmesh comes from the asset processor's cache (baked on first use).
fn install_navmesh(
    mut commands: Commands,
    active: Option<Res<ActiveLevel>>,
    navigation: Option<Res<LevelNavigation>>,
    party: Query<(), With<PartyBody>>,
    assets: Res<AssetServer>,
    mut ready: MessageWriter<NavmeshReady>,
) {
    let Some(active) = active else { return };
    if navigation.is_some() || party.is_empty() {
        return;
    }
    match assets.get_load_state(&active.navmesh) {
        Some(bevy::asset::LoadState::Loaded) => {}
        Some(bevy::asset::LoadState::Failed(error)) => {
            error!("navmesh unavailable: {error}");
            commands.insert_resource(LevelNavigation { archipelago: Entity::PLACEHOLDER });
            return;
        }
        _ => return,
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
    let archipelago = commands.spawn((Name::new("Archipelago"), Archipelago3d::new(options))).id();
    commands.spawn((
        Name::new("Level navmesh"),
        Transform::default(),
        Island3dBundle {
            island: Island,
            archipelago_ref: ArchipelagoRef3d::new(archipelago),
            nav_mesh: NavMeshHandle(active.navmesh.clone()),
        },
    ));
    commands.insert_resource(LevelNavigation { archipelago });
    info!("navmesh installed");
    ready.write(NavmeshReady);
}

/// Register the party with the archipelago and report its velocity.
fn track_party_character(
    mut commands: Commands,
    navigation: Option<Res<LevelNavigation>>,
    mut party: Query<(Entity, &LinearVelocity, Option<&mut Velocity3d>), With<PartyBody>>,
) {
    let (Some(navigation), Ok((entity, velocity, tracked))) = (navigation, party.single_mut())
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
fn steer_walkers(
    mut walkers: Query<(&Walker, &AgentDesiredVelocity3d, &mut TnuaController<WalkerMotion>)>,
) {
    for (walker, desired, mut controller) in &mut walkers {
        let velocity = desired.velocity().with_y(0.0);
        controller.initiate_action_feeding();
        controller.basis = TnuaBuiltinWalk {
            desired_motion: velocity / walker.speed,
            desired_forward: Dir3::new(velocity).ok(),
        };
    }
}
