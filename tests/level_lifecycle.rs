//! Small headless scenarios at the level/party ownership boundary: a level
//! becomes active in one step once its prerequisites exist, and travel
//! suspends the party until the destination is playable, or leaves it where
//! it was when the destination cannot be.

use avian3d::prelude::{Collider, RigidBodyDisabled};
use bevy::prelude::*;
use rizzardry::content::items::{ItemCatalog, LootTables};
use rizzardry::content::level::{CollisionMesh, LevelGeometry, MonsterPlacement};
use rizzardry::content::species::{MonsterCatalog, Species};
use rizzardry::game::data::GameData;
use rizzardry::game::inventory::ItemIds;
use rizzardry::game::party::body::{PartyBody, PartyMotionConfig, PartyStart};
use rizzardry::game::rng::GameRng;
use rizzardry::game::world::level::{
    Departing, Destination, Level, LevelMemory, LevelReady, Travel, activate_level, travel,
};
use rizzardry::game::world::population::Monster;
use rizzardry::game::world::triggers::PendingPartyMove;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<LevelGeometry>()
        .init_asset::<ItemCatalog>()
        .init_asset::<LootTables>()
        .init_asset::<MonsterCatalog>()
        .init_asset::<bevy_landmass::NavMesh3d>()
        .init_asset::<rizzardry::content::monster::MonsterModel>()
        .init_asset::<rizzardry::content::ui::TextTable>()
        .init_resource::<Assets<PartyMotionConfig>>()
        .init_resource::<LevelMemory>()
        .init_resource::<ItemIds>()
        .init_resource::<GameRng>()
        .init_resource::<PartyStart>()
        .init_resource::<PendingPartyMove>()
        .add_message::<LevelReady>()
        .add_message::<Travel>()
        .add_systems(PreUpdate, travel)
        .add_systems(Update, activate_level);
    app.finish();
    app.cleanup();
    app
}

/// The shared databases, nearly empty (one species), as gameplay needs them
/// loaded.
fn with_data(app: &mut App) {
    let world = app.world_mut();
    let items = world.resource_mut::<Assets<ItemCatalog>>().add(ItemCatalog::default());
    let loot = world.resource_mut::<Assets<LootTables>>().add(LootTables::default());
    let crab = Species { name: "Crab".into(), model: "crab".into(), ..default() };
    let species = world.resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(vec![crab]));
    world.insert_resource(GameData { items, loot, species });
}

fn test_level(app: &mut App, collision: CollisionMesh) -> Level {
    let crab =
        MonsterPlacement { location: 7, group: 1, species: 0, position: Vec3::ZERO, yaw: 0.0 };
    let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
        collision,
        monsters: vec![crab],
        ..default()
    });
    Level {
        path: "levels/test/test.pvl".into(),
        geometry,
        navmeshes: vec![],
        sky: None,
        script: default(),
        messages: None,
    }
}

/// The test level, to be activated.
fn spawn_level(app: &mut App, collision: CollisionMesh) -> Entity {
    let level = test_level(app, collision);
    app.world_mut().spawn(Destination::new(level)).id()
}

fn party_entities(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<PartyBody>>();
    query.iter(world).collect()
}

fn is_played(app: &App, level: Entity) -> bool {
    app.world().get::<Level>(level).is_some() && app.world().get::<Departing>(level).is_none()
}

fn failed(app: &App, destination: Entity) -> bool {
    app.world().get::<Destination>(destination).is_some_and(|d| d.failed)
}

fn floor() -> CollisionMesh {
    CollisionMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Z],
        triangles: vec![[0, 1, 2]],
        walkable: vec![true],
    }
}

#[test]
fn a_level_without_collision_fails_and_places_no_party() {
    let mut app = app();
    with_data(&mut app);
    let root = spawn_level(&mut app, CollisionMesh::default());

    app.update();

    assert!(failed(&app, root));
    assert!(app.world().get::<Collider>(root).is_none());
    assert!(app.world().resource::<Messages<LevelReady>>().is_empty());
    assert!(party_entities(&mut app).is_empty());
}

#[test]
fn a_level_waits_for_the_shared_databases() {
    let mut app = app();
    let root = spawn_level(&mut app, floor());

    app.update();
    assert!(!is_played(&app, root) && !failed(&app, root));
    assert!(party_entities(&mut app).is_empty());

    with_data(&mut app);
    app.update();
    assert!(is_played(&app, root));
    assert!(app.world().get::<Collider>(root).is_some());
    assert_eq!(party_entities(&mut app).len(), 1);
}

#[test]
fn the_party_survives_level_replacement_without_duplication() {
    let mut app = app();
    with_data(&mut app);
    let first_level = spawn_level(&mut app, floor());

    app.update();
    let party = party_entities(&mut app);
    assert_eq!(party.len(), 1);
    app.update();
    assert_eq!(party_entities(&mut app), party);

    app.world_mut().despawn(first_level);
    let second_level = spawn_level(&mut app, floor());
    app.update();

    assert!(is_played(&app, second_level));
    assert_eq!(party_entities(&mut app), party);
}

#[test]
fn a_destination_that_cannot_load_leaves_the_party_where_it_was() {
    let mut app = app();
    with_data(&mut app);
    let first_level = spawn_level(&mut app, floor());
    app.update();
    let party = party_entities(&mut app)[0];

    // MO2's files are not there to load here: the destination fails.
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.world_mut().run_schedule(PreUpdate);
    assert!(app.world().get::<Departing>(first_level).is_some());
    assert!(app.world().get::<RigidBodyDisabled>(party).is_some());
    for _ in 0..20 {
        app.update();
    }

    assert!(is_played(&app, first_level), "the party is back in the level it left");
    assert!(app.world().get::<RigidBodyDisabled>(party).is_none());
    assert!(app.world().resource::<LevelMemory>().is_empty(), "the stale record is gone");
    assert_eq!(party_entities(&mut app), vec![party]);
}

fn monsters(app: &mut App) -> Vec<Monster> {
    let world = app.world_mut();
    let mut query = world.query::<&Monster>();
    query.iter(world).cloned().collect()
}

#[test]
fn a_killed_monster_stays_dead_when_the_party_returns() {
    let mut app = app();
    with_data(&mut app);
    spawn_level(&mut app, floor());
    app.update();
    assert_eq!(monsters(&mut app).len(), 1);
    {
        let world = app.world_mut();
        let mut query = world.query::<&mut Monster>();
        query.single_mut(world).unwrap().hp = 0;
    }

    // Leave, then come straight back: the travel step remembers the level,
    // and its destination (which cannot load here) gives way to the same
    // level, as returning would.
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.world_mut().run_schedule(PreUpdate);
    let mut destinations = app.world_mut().query_filtered::<Entity, With<Destination>>();
    let away: Vec<Entity> = destinations.iter(app.world()).collect();
    for destination in away {
        app.world_mut().despawn(destination);
    }
    let back = spawn_level(&mut app, floor());
    app.update();

    assert!(is_played(&app, back));
    assert_eq!(app.world_mut().query::<&Level>().iter(app.world()).count(), 1);
    assert!(monsters(&mut app).is_empty(), "the dead crab came back");
}
