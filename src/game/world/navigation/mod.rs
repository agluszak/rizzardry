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
use crate::content::level::LevelDefinition;
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

/// Whether a level's navmeshes have settled: all loaded, or one failed.
/// They come from the asset processor's cache (baked on first use).
pub fn navmeshes_settled(level: &Level, assets: &AssetServer) -> bool {
    level.navmeshes.iter().all(|navmesh| {
        matches!(assets.get_load_state(navmesh), Some(LoadState::Loaded | LoadState::Failed(_)))
    })
}

/// Install a level's navigation under its root, as part of its activation:
/// one archipelago and island per size class, and the level's links. A
/// navmesh that failed to load leaves the level without navigation; nothing
/// will path.
pub fn install(
    commands: &mut Commands,
    root: Entity,
    level: &Level,
    geometry: &LevelDefinition,
    assets: &AssetServer,
) {
    if let Some(Some(LoadState::Failed(error))) = level
        .navmeshes
        .iter()
        .map(|h| assets.get_load_state(h))
        .find(|s| matches!(s, Some(LoadState::Failed(_))))
    {
        error!("navmesh unavailable: {error}");
        return;
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
    links::spawn_level_links(commands, root, geometry, &archipelagos);
    commands.entity(root).insert(LevelNavigation { archipelagos });
    info!("navmeshes installed for {} size classes", level.navmeshes.len());
}

/// The party as a Landmass character in one archipelago: a child of the
/// party body, so walkers of every size class avoid it.
#[derive(Component, Debug)]
pub struct PartyCharacter;

/// Register the party in every archipelago of the current level and report
/// its velocity. After travel the old archipelagos are gone; their
/// characters are replaced.
fn track_party_character(
    mut commands: Commands,
    navigation: Query<&LevelNavigation>,
    party: Query<(Entity, &LinearVelocity), With<PartyBody>>,
    mut characters: Query<(Entity, &ArchipelagoRef3d, &mut Velocity3d), With<PartyCharacter>>,
) {
    let (Ok(navigation), Ok((party, velocity))) = (navigation.single(), party.single()) else {
        return;
    };
    for (character, archipelago, mut tracked) in &mut characters {
        if navigation.archipelagos.contains(&archipelago.entity) {
            tracked.velocity = velocity.0;
        } else {
            commands.entity(character).despawn();
        }
    }
    for &archipelago in &navigation.archipelagos {
        if !characters.iter().any(|(_, reference, _)| reference.entity == archipelago) {
            commands.spawn((
                PartyCharacter,
                Transform::default(),
                Character3dBundle {
                    character: default(),
                    settings: CharacterSettings { radius: crate::game::party::body::RADIUS },
                    archipelago_ref: ArchipelagoRef3d::new(archipelago),
                },
                Velocity3d { velocity: velocity.0 },
                ChildOf(party),
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

#[cfg(test)]
mod party_character_tests {
    use super::*;

    #[test]
    fn avoidance_proxies_follow_velocity_repair_missing_classes_and_retire_on_travel() {
        let mut app = App::new();
        app.add_systems(Update, track_party_character);
        let party = app.world_mut().spawn((PartyBody::default(), LinearVelocity(Vec3::X))).id();
        let first = app.world_mut().spawn_empty().id();
        let second = app.world_mut().spawn_empty().id();
        let level =
            app.world_mut().spawn(LevelNavigation { archipelagos: vec![first, second] }).id();
        app.update();
        let proxies = app.world().get::<Children>(party).unwrap().to_vec();
        assert_eq!(proxies.len(), 2);
        for &proxy in &proxies {
            assert_eq!(app.world().get::<Velocity3d>(proxy).unwrap().velocity, Vec3::X);
        }
        app.world_mut().despawn(proxies[0]);
        app.world_mut().entity_mut(party).insert(LinearVelocity(Vec3::Z));
        app.update();
        let repaired = app.world().get::<Children>(party).unwrap().to_vec();
        assert_eq!(repaired.len(), 2);
        assert!(repaired.contains(&proxies[1]), "retain the surviving avoidance proxy");
        let mut references: Vec<_> = repaired
            .iter()
            .map(|&proxy| {
                assert_eq!(app.world().get::<Velocity3d>(proxy).unwrap().velocity, Vec3::Z);
                app.world().get::<ArchipelagoRef3d>(proxy).unwrap().entity
            })
            .collect();
        references.sort();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(references, expected);
        let next = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(level).insert(LevelNavigation { archipelagos: vec![next] });
        app.update();
        for proxy in repaired {
            assert!(app.world().get_entity(proxy).is_err());
        }
        let children = app.world().get::<Children>(party).unwrap();
        assert_eq!(children.len(), 1);
        let proxy = children[0];
        assert_eq!(app.world().get::<ArchipelagoRef3d>(proxy).unwrap().entity, next);
        app.world_mut().despawn(party);
        assert!(app.world().get_entity(proxy).is_err());
    }
}
