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
use bevy::asset::{AssetApp, LoadState};
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_landmass::{NavMeshHandle, PointSampleDistance3d};
use bevy_tnua::builtins::TnuaBuiltinWalk;
use bevy_tnua::prelude::*;

use super::level::Level;
use crate::content::level::LevelGeometry;
use crate::game::party::body::{FLOAT_HEIGHT, PartyBody};

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn finish(&self, app: &mut App) {
        // After Landmass registers `NavMesh3d`.
        app.init_asset::<asset::BakedNavmesh>().register_asset_loader(asset::NavmeshLoader);
    }

    fn build(&self, app: &mut App) {
        app.add_plugins(Landmass3dPlugin::default())
            .add_plugins(TnuaControllerPlugin::<WalkerMotion>::new(FixedUpdate))
            .add_systems(Update, links::sync_door_links.run_if(super::level::level_active))
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

/// Where the feet of something at `origin` are: under a walker's floating
/// body, else at its origin.
pub fn feet(origin: Vec3, walker: Option<&Walker>) -> Vec3 {
    origin - Vec3::Y * walker.map_or(0.0, |w| w.float_height)
}

/// Whether every mandatory navmesh has loaded successfully.
/// They come from the asset processor's cache (baked on first use).
pub fn navmeshes_ready(level: &Level, assets: &AssetServer) -> bool {
    level
        .navmeshes
        .iter()
        .all(|navmesh| matches!(assets.get_load_state(navmesh), Some(LoadState::Loaded)))
}

/// Install a level's navigation under its root, as part of its activation:
/// one archipelago and island per size class, and the level's links.
/// Activation has already checked every required navmesh.
pub fn install(commands: &mut Commands, root: Entity, level: &Level, geometry: &LevelGeometry) {
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
    links::spawn_level_links(commands, root, geometry, &archipelagos);
    commands.entity(root).insert(LevelNavigation { archipelagos });
    info!("navmeshes installed for {} size classes", level.navmeshes.len());
}

/// The party as a Landmass character in one archipelago: a child of the
/// party body, so walkers of every size class avoid it.
#[derive(Component, Debug)]
pub struct PartyCharacter {
    archipelago: Entity,
}

/// Register the party in every archipelago of the current level and report
/// its velocity. After travel the old archipelagos are gone; their
/// characters are replaced.
fn track_party_character(
    mut commands: Commands,
    navigation: Query<&LevelNavigation>,
    party: Query<(Entity, &LinearVelocity), With<PartyBody>>,
    mut characters: Query<(Entity, &PartyCharacter, &mut Velocity3d)>,
) {
    let (Ok(navigation), Ok((party, velocity))) = (navigation.single(), party.single()) else {
        return;
    };
    let current =
        characters.iter().all(|(_, c, _)| navigation.archipelagos.contains(&c.archipelago));
    if !current {
        for (character, _, _) in &characters {
            commands.entity(character).despawn();
        }
    }
    if characters.is_empty() || !current {
        for &archipelago in &navigation.archipelagos {
            commands.spawn((
                PartyCharacter { archipelago },
                Transform::default(),
                Character3dBundle {
                    character: default(),
                    settings: CharacterSettings { radius: 500.0 / crate::content::UNITS_PER_METRE },
                    archipelago_ref: ArchipelagoRef3d::new(archipelago),
                },
                Velocity3d { velocity: velocity.0 },
                ChildOf(party),
            ));
        }
    }
    for (_, _, mut tracked) in &mut characters {
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
        Option<&super::population::Monster>,
        &AgentDesiredVelocity3d,
        Option<&links::Crossing>,
        &Transform,
        &mut TnuaController<WalkerMotion>,
    )>,
) {
    for (walker, monster, desired, crossing, transform, mut controller) in &mut walkers {
        let velocity = if monster.is_some_and(|m| m.is_dead()) {
            Vec3::ZERO
        } else {
            match crossing {
                Some(crossing) => {
                    (crossing.to - transform.translation).with_y(0.0).normalize_or_zero()
                        * walker.speed
                        * 0.8
                }
                None => desired.velocity().with_y(0.0),
            }
        };
        controller.initiate_action_feeding();
        controller.basis = TnuaBuiltinWalk {
            desired_motion: velocity / walker.speed,
            desired_forward: Dir3::new(velocity).ok(),
        };
    }
}
