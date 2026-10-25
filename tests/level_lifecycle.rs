//! Small headless scenarios at the level/party ownership boundary: a level
//! becomes active in one step once its prerequisites exist, and travel
//! suspends the party until the destination is playable.

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
    Level, LevelMemory, LevelPhase, LevelReady, Travel, activate_level, travel,
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
    let crab = Species { name: "Crab".into(), ..default() };
    let species = world.resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(vec![crab]));
    world.insert_resource(GameData { items, loot, species });
}

fn spawn_level(app: &mut App, collision: CollisionMesh) -> Entity {
    let crab = MonsterPlacement {
        location: 7,
        group: 1,
        species: 0,
        model: "crab".into(),
        position: Vec3::ZERO,
        yaw: 0.0,
    };
    let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
        collision,
        monsters: vec![crab],
        ..default()
    });
    app.world_mut()
        .spawn(Level {
            path: "levels/test/test.pvl".into(),
            geometry,
            navmeshes: vec![],
            sky: None,
            script: default(),
            messages: None,
        })
        .id()
}

fn party_entities(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<PartyBody>>();
    query.iter(world).collect()
}

fn phase(app: &App, level: Entity) -> LevelPhase {
    *app.world().get::<LevelPhase>(level).unwrap()
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

    assert_eq!(phase(&app, root), LevelPhase::Failed);
    assert!(app.world().get::<Collider>(root).is_none());
    assert!(app.world().resource::<Messages<LevelReady>>().is_empty());
    assert!(party_entities(&mut app).is_empty());
}

#[test]
fn a_level_waits_for_the_shared_databases() {
    let mut app = app();
    let root = spawn_level(&mut app, floor());

    app.update();
    assert_eq!(phase(&app, root), LevelPhase::Loading);
    assert!(party_entities(&mut app).is_empty());

    with_data(&mut app);
    app.update();
    assert_eq!(phase(&app, root), LevelPhase::Active);
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

    assert_eq!(phase(&app, second_level), LevelPhase::Active);
    assert_eq!(party_entities(&mut app), party);
}

#[test]
fn travel_suspends_the_party_until_the_destination_activates() {
    let mut app = app();
    with_data(&mut app);
    let first_level = spawn_level(&mut app, floor());
    app.update();
    let party = party_entities(&mut app)[0];

    // MO2's files are not there to load here: the destination fails.
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.update();

    assert!(app.world().get_entity(first_level).is_err());
    assert!(app.world().get::<RigidBodyDisabled>(party).is_some());
    let mut levels = app.world_mut().query::<(Entity, &Level)>();
    let (destination, level) = levels.single(app.world()).unwrap();
    assert_eq!(level.path, "levels/monastery/monastery2.pvl");
    for _ in 0..20 {
        app.update();
    }
    assert_ne!(phase(&app, destination), LevelPhase::Active);
    assert!(app.world().get::<RigidBodyDisabled>(party).is_some());
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

    // Leave (for a level that cannot load here), then come back.
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.update();
    assert!(monsters(&mut app).is_empty());
    let mut levels = app.world_mut().query_filtered::<Entity, With<Level>>();
    let away: Vec<Entity> = levels.iter(app.world()).collect();
    for level in away {
        app.world_mut().despawn(level);
    }
    let back = spawn_level(&mut app, floor());
    app.update();

    assert_eq!(phase(&app, back), LevelPhase::Active);
    assert!(monsters(&mut app).is_empty(), "the dead crab came back");
}
