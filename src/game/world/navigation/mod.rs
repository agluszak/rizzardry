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

/// On a level root: one archipelago per walker size class, once the
/// level's navmeshes are installed.
#[derive(Component, Debug)]
pub struct LevelNavigation {
    pub archipelagos: Vec<Entity>,
}

impl LevelNavigation {
    /// The archipelago a walker of `radius` metres navigates in.
    pub fn archipelago_for(&self, radius: f32) -> Entity {
        self.archipelagos[bake::size_class(radius)]
    }
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

/// Install a level's navmeshes under its root once they have loaded: one
/// archipelago and island per size class. The navmeshes come from the asset
/// processor's cache (baked on first use).
fn install_navmesh(
    mut commands: Commands,
    levels: Query<(Entity, &Level), With<AwaitingNavmesh>>,
    assets: Res<AssetServer>,
) {
    for (root, level) in &levels {
        let states: Vec<_> = level.navmeshes.iter().map(|h| assets.get_load_state(h)).collect();
        if let Some(Some(LoadState::Failed(error))) =
            states.iter().find(|s| matches!(s, Some(LoadState::Failed(_))))
        {
            // The level stays without navigation; nothing will path.
            error!("navmesh unavailable: {error}");
            commands.entity(root).remove::<AwaitingNavmesh>();
            continue;
        }
        if !states.iter().all(|s| matches!(s, Some(LoadState::Loaded))) {
            continue;
        }
        let mut archipelagos = Vec::new();
        for (class, navmesh) in level.navmeshes.iter().enumerate() {
            let radius = bake::SIZE_CLASSES[class];
            let mut options = ArchipelagoOptions::from_agent_radius(radius);
            // Actor origins float above the ground; sample the navmesh beneath.
            options.point_sample_distance = PointSampleDistance3d {
                horizontal_distance: 1.0,
                distance_above: 0.5,
                distance_below: FLOAT_HEIGHT + 2.0,
                vertical_preference_ratio: 2.0,
                animation_link_max_vertical_distance: 1.0,
            };
            let archipelago = commands
                .spawn((
                    Name::new(format!("Archipelago for {radius} m walkers")),
                    Archipelago3d::new(options),
                    ChildOf(root),
                ))
                .id();
            commands.spawn((
                Name::new(format!("Navmesh for {radius} m walkers")),
                Transform::default(),
                Island3dBundle {
                    island: Island,
                    archipelago_ref: ArchipelagoRef3d::new(archipelago),
                    nav_mesh: NavMeshHandle(navmesh.clone()),
                },
                ChildOf(root),
            ));
            archipelagos.push(archipelago);
        }
        commands.entity(root).remove::<AwaitingNavmesh>().insert(LevelNavigation { archipelagos });
        info!("navmeshes installed for {} size classes", level.navmeshes.len());
    }
}

/// The party as a Landmass character in one archipelago: a child of the
/// party body, so walkers of every size class avoid it.
#[derive(Component, Debug)]
pub struct PartyCharacter;

/// Register the party in every archipelago and report its velocity.
fn track_party_character(
    mut commands: Commands,
    navigation: Query<&LevelNavigation>,
    party: Query<(Entity, &LinearVelocity), With<PartyBody>>,
    mut characters: Query<&mut Velocity3d, With<PartyCharacter>>,
) {
    let (Ok(navigation), Ok((party, velocity))) = (navigation.single(), party.single()) else {
        return;
    };
    if characters.is_empty() {
        for &archipelago in &navigation.archipelagos {
            commands.spawn((
                PartyCharacter,
                Transform::default(),
                Character3dBundle {
                    character: default(),
                    settings: CharacterSettings {
                        radius: 500.0 / crate::retail::space::UNITS_PER_METRE,
                    },
                    archipelago_ref: ArchipelagoRef3d::new(archipelago),
                },
                Velocity3d { velocity: velocity.0 },
                ChildOf(party),
            ));
        }
    }
    for mut tracked in &mut characters {
        tracked.velocity = velocity.0;
    }
}

/// Feed each walker's physical velocity back to Landmass.
fn sync_walker_velocity(mut walkers: Query<(&LinearVelocity, &mut Velocity3d), With<Walker>>) {
    for (linear, mut velocity) in &mut walkers {
        velocity.velocity = linear.0;
    }
}

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
