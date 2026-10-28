//! Navigation over the active level: a Rerecast-baked navmesh used by a
//! Landmass archipelago.
//!
//! Landmass chooses each agent's desired velocity (path following and local
//! avoidance); a Tnua walk controller turns that into motion under Avian
//! collision; the resulting velocity goes back to Landmass on the next step.
//! The party is a Landmass character: an obstacle agents avoid, not an agent.
//!
//! The navmesh is baked from the same triangles as the level collider when
//! a level asks for it, and cached ([`asset`]).

pub mod asset;
pub mod bake;
pub mod links;

use crate::game::world::level::LevelOwned;
use avian3d::prelude::*;
use bevy::asset::LoadState;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_landmass::{NavMeshHandle, PointSampleDistance3d};
use bevy_tnua::builtins::TnuaBuiltinWalk;
use bevy_tnua::prelude::*;

use super::level::Level;
use crate::content::level::LevelDefinition;
use crate::game::party::body::{FLOAT_HEIGHT, PartyBody};

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Landmass3dPlugin::default())
            .add_observer(links::end_crossing)
            .add_plugins(TnuaControllerPlugin::<WalkerMotion>::new(FixedUpdate))
            .add_systems(
                FixedPreUpdate,
                links::sync_door_links
                    .before(bevy_landmass::LandmassSystems::SyncExistence)
                    .run_if(super::level::level_active),
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

/// The share of its top speed a walker cruises at along a path.
const CRUISING_SHARE: f32 = 0.8;
/// Half the height of a monster's body. Bodies are flat cylinders as wide
/// as the model's walk radius, however tall the model.
const HALF_HEIGHT: f32 = 0.5;
/// The ground sensor's radius: the ground under the body's centre decides
/// its height, not the highest point anywhere under a wide body.
const SENSOR_RADIUS: f32 = 0.3;
/// Wider monsters navigate the narrowest navmesh that still fits this much,
/// the largest size class: the wider navmeshes lose narrow ground (beaches,
/// ledges) to erosion, so big monsters may brush corners instead.
const MAX_NAVIGATION_RADIUS: f32 = bake::SIZE_CLASSES[bake::SIZE_CLASSES.len() - 1];

/// How far above its feet a body of `speed` floats: walkers ride over steps,
/// standing monsters sit on the ground.
pub fn float_height(speed: f32) -> f32 {
    if speed > 0.0 { crate::game::world::STEP_HEIGHT + HALF_HEIGHT } else { 0.0 }
}

/// Make a monster a walker: its physics body, Tnua controller and, where
/// the level has navmeshes, its Landmass agent.
pub fn insert_walker(
    entity: &mut EntityCommands,
    (feet, yaw): (Vec3, f32),
    (radius, speed): (f32, f32),
    navigation: &LevelNavigation,
    configs: &mut Assets<WalkerMotionConfig>,
) {
    let float_height = float_height(speed);
    let config = configs.add(WalkerMotionConfig {
        basis: bevy_tnua::builtins::TnuaBuiltinWalkConfig {
            speed,
            float_height,
            max_slope: 50f32.to_radians(),
            ..default()
        },
    });
    entity.insert((
        RigidBody::Dynamic,
        Position(feet + Vec3::Y * float_height),
        Rotation(Quat::from_rotation_y(yaw)),
        Collider::cylinder(radius, 2.0 * HALF_HEIGHT),
        LockedAxes::ROTATION_LOCKED,
        TnuaController::<WalkerMotion>::default(),
        TnuaConfig::<WalkerMotion>(config),
        bevy_tnua_avian3d::prelude::TnuaAvian3dSensorShape(Collider::cylinder(
            SENSOR_RADIUS.min(radius),
            0.0,
        )),
        Walker { speed, float_height },
        AgentTarget3d::None,
        Velocity3d::default(),
    ));
    // Synthetic worlds can omit navigation; production activation requires
    // every requested navmesh before constructing actors.
    if !navigation.archipelagos.is_empty() {
        let radius = radius.min(MAX_NAVIGATION_RADIUS);
        entity.insert(Agent3dBundle {
            agent: default(),
            settings: AgentSettings {
                radius,
                desired_speed: speed * CRUISING_SHARE,
                max_speed: speed,
            },
            archipelago_ref: ArchipelagoRef3d::new(navigation.archipelago_for(radius)),
        });
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
pub fn navmeshes_ready(level: &Level, assets: &AssetServer) -> bool {
    level
        .navmeshes
        .iter()
        .all(|navmesh| matches!(assets.get_load_state(navmesh), Some(LoadState::Loaded)))
}

/// Install a level's navigation under its root, as part of its activation:
/// one archipelago and island per size class, and the level's links.
/// Activation has already checked every required navmesh.
pub fn install(
    commands: &mut Commands,
    root: Entity,
    level: &Level,
    geometry: &LevelDefinition,
) -> LevelNavigation {
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
                LevelOwned(root),
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
            LevelOwned(root),
        ));
        archipelagos.push(archipelago);
    }
    links::spawn_level_links(commands, root, geometry, &archipelagos);
    info!("navmeshes installed for {} size classes", level.navmeshes.len());
    LevelNavigation { archipelagos }
}

/// The party as a Landmass character in one archipelago, so walkers of
/// every size class avoid it. It follows the party's transform and lives
/// as long as the level whose archipelago it is registered in.
#[derive(Component, Debug)]
pub struct PartyCharacter;

/// Register the party in each of a level's archipelagos, as part of the
/// level's activation.
pub fn add_party_characters(
    commands: &mut Commands,
    level: Entity,
    party: Entity,
    navigation: &LevelNavigation,
) {
    for &archipelago in &navigation.archipelagos {
        commands.spawn((
            PartyCharacter,
            Transform::default(),
            Character3dBundle {
                character: default(),
                settings: CharacterSettings { radius: crate::game::party::body::RADIUS },
                archipelago_ref: ArchipelagoRef3d::new(archipelago),
            },
            Velocity3d::default(),
            ChildOf(party),
            LevelOwned(level),
        ));
    }
}

/// Report the party's velocity to its avoidance characters.
fn track_party_character(
    party: Query<&LinearVelocity, With<PartyBody>>,
    mut characters: Query<&mut Velocity3d, With<PartyCharacter>>,
) {
    let Ok(velocity) = party.single() else { return };
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

/// Whether a walker is to move now, the one rule for all its locomotion: it
/// has somewhere to go, and a monster can still act. Ending a move clears
/// the target, so a crossing it began keeps its progress but no longer moves
/// the walker, and recovering from a condition does not resume it; only a
/// new move does.
pub(crate) fn walking(
    target: &AgentTarget3d,
    monster: Option<&super::population::Monster>,
) -> bool {
    !matches!(target, AgentTarget3d::None) && monster.is_none_or(|m| m.can_fight())
}

/// Feed Landmass's desired velocity to Tnua; while crossing a doorway link,
/// head straight for its far side instead.
fn steer_walkers(
    mut walkers: Query<(
        &Walker,
        &AgentTarget3d,
        Option<&super::population::Monster>,
        Option<&AgentDesiredVelocity3d>,
        Option<&links::Crossing>,
        &Transform,
        &mut TnuaController<WalkerMotion>,
    )>,
) {
    for (walker, target, monster, desired, crossing, transform, mut controller) in &mut walkers {
        let velocity = if !walking(target, monster) {
            Vec3::ZERO
        } else {
            match crossing {
                Some(crossing) => {
                    (crossing.to - transform.translation).with_y(0.0).normalize_or_zero()
                        * walker.speed
                        * CRUISING_SHARE
                }
                None => desired.map_or(Vec3::ZERO, |d| d.velocity().with_y(0.0)),
            }
        };
        controller.initiate_action_feeding();
        controller.basis = TnuaBuiltinWalk {
            desired_motion: velocity / walker.speed,
            desired_forward: Dir3::new(velocity).ok(),
        };
    }
}
