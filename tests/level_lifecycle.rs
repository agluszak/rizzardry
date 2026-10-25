//! Small headless scenarios at the level/party ownership boundary: a level
//! becomes active in one step once its prerequisites exist, and travel
//! suspends the party until the destination is playable, or leaves it where
//! it was when the destination cannot be.

use avian3d::prelude::{Collider, RigidBodyDisabled};
use bevy::prelude::*;
use rizzardry::content::items::{ItemCatalog, LootTables};
use rizzardry::content::level::{CollisionMesh, LevelGeometry, MonsterPlacement};
use rizzardry::content::monster::MonsterBody;
use rizzardry::content::species::{MonsterCatalog, Species};
use rizzardry::game::data::GameData;
use rizzardry::game::inventory::ItemIds;
use rizzardry::game::party::body::{PartyBody, PartyMotionConfig, PartyStart};
use rizzardry::game::rng::GameRng;
use rizzardry::game::world::level::{
    Departing, Destination, Level, LevelMemory, LevelReady, Travel, activate_level, travel,
};
use rizzardry::game::world::population::{Monster, MonsterBodies};
use rizzardry::game::world::triggers::PendingPartyMove;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<LevelGeometry>()
        .init_asset::<ItemCatalog>()
        .init_asset::<LootTables>()
        .init_asset::<MonsterCatalog>()
        .init_asset::<bevy_landmass::NavMesh3d>()
        .init_asset::<MonsterBody>()
        .init_asset::<rizzardry::content::ui::TextTable>()
        .init_resource::<Assets<PartyMotionConfig>>()
        .init_resource::<LevelMemory>()
        .init_resource::<MonsterBodies>()
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
    // Required body metadata is available without a visual asset or renderer.
    let body = world.resource_mut::<Assets<MonsterBody>>().add(MonsterBody::default());
    world.resource_mut::<MonsterBodies>().0.insert(0, body);
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

#[test]
fn a_level_waits_for_body_metadata_and_sizes_monsters_without_render_assets() {
    let mut app = app();
    with_data(&mut app);
    // The crab's gameplay body metadata is still loading.
    let loading = app.world_mut().resource_mut::<Assets<MonsterBody>>().reserve_handle();
    app.world_mut().resource_mut::<MonsterBodies>().0.insert(0, loading.clone());
    let root = spawn_level(&mut app, floor());

    app.update();
    assert!(!is_played(&app, root) && !failed(&app, root));

    let body = MonsterBody { fight_radius: 0.5, scale: 2.0, ..default() };
    app.world_mut().resource_mut::<Assets<MonsterBody>>().insert(&loading, body).unwrap();
    app.update();
    assert!(is_played(&app, root));
    let world = app.world_mut();
    let crab = world.query::<&Monster>().single(world).unwrap();
    assert_eq!((crab.body.scale, crab.body.radius), (2.0, 1.0), "its body at its size");
}

#[test]
fn body_metadata_loads_even_when_the_visual_cycles_fail() {
    use bevy::asset::LoadState;
    use rizzardry::content::monster::MonsterModel;
    use rizzardry::retail::monster_loader::{MlsBodyLoader, MlsLoader};

    let directory =
        std::env::temp_dir().join(format!("rizzardry-body-loader-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("crab.mls"),
        "scalefactor 2\nfightradius 0.75\ntargetheight 1.5\nIDLE broken\n",
    )
    .unwrap();
    std::fs::write(directory.join("broken.mon"), [0]).unwrap();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin { file_path: directory.to_string_lossy().into_owned(), ..default() },
    ))
    .init_asset::<MonsterBody>()
    .init_asset::<MonsterModel>()
    .init_asset_loader::<MlsBodyLoader>()
    .init_asset_loader::<MlsLoader>();
    app.finish();
    app.cleanup();
    let body = app.world().resource::<AssetServer>().load::<MonsterBody>("crab.mls");
    let visual = app.world().resource::<AssetServer>().load::<MonsterModel>("crab.mls");
    let started = std::time::Instant::now();
    loop {
        app.update();
        let assets = app.world().resource::<AssetServer>();
        if matches!(assets.get_load_state(&body), Some(LoadState::Loaded))
            && matches!(assets.get_load_state(&visual), Some(LoadState::Failed(_)))
        {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "body/visual loads did not settle independently"
        );
        std::thread::yield_now();
    }
    let definition = app.world().resource::<Assets<MonsterBody>>().get(&body).unwrap();
    assert_eq!(
        (definition.scale, definition.fight_radius, definition.target_height),
        (2.0, 0.75, 1.5)
    );
    assert!(app.world().resource::<Assets<MonsterModel>>().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn attaching_visuals_does_not_change_physical_state_or_revisit_placement() {
    use rizzardry::content::monster::MonsterModel;
    use rizzardry::game::world::navigation::Walker;
    use rizzardry::game::world::population::ScaledBody;
    use rizzardry::presentation::monsters::{
        MonsterInstance, MonsterVisual, build_monster_instances,
    };

    for show_visuals in [false, true] {
        let mut app = app();
        with_data(&mut app);
        let first = spawn_level(&mut app, floor());
        app.update();
        let actor =
            app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
        let feet = Vec3::new(2.0, 3.0, 4.0);
        let body = ScaledBody { scale: 1.4, radius: 0.75, eye_height: 1.9, speed: 3.0 };
        let float_height = 1.1;
        let pose = Transform::from_translation(feet + Vec3::Y * float_height)
            .with_rotation(Quat::from_rotation_y(0.7));
        app.world_mut().get_mut::<Monster>(actor).unwrap().body = body;
        app.world_mut()
            .entity_mut(actor)
            .insert((Walker { speed: body.speed, float_height }, pose));
        if show_visuals {
            app.init_resource::<Assets<MonsterModel>>()
                .init_resource::<Assets<Mesh>>()
                .add_systems(Update, build_monster_instances);
            let model = app.world_mut().resource_mut::<Assets<MonsterModel>>().reserve_handle();
            app.world_mut().entity_mut(actor).insert(MonsterVisual(model.clone()));
            app.update();
            assert!(app.world().get::<MonsterInstance>(actor).is_none());
            assert_eq!(*app.world().get::<Transform>(actor).unwrap(), pose);
            app.world_mut()
                .resource_mut::<Assets<MonsterModel>>()
                .insert(&model, MonsterModel { cycles: default() })
                .unwrap();
            app.update();
            assert!(app.world().get::<MonsterInstance>(actor).is_some());
            assert_eq!(*app.world().get::<Transform>(actor).unwrap(), pose);
            assert_eq!(app.world().get::<Monster>(actor).unwrap().body, body);
        }
        // Capture through the real travel boundary, then consume that level's
        // dormant record. The controller offset is gameplay data in both runs.
        app.world_mut().write_message(Travel { entrance: "MO204".into() });
        app.world_mut().run_schedule(PreUpdate);
        let away: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<Destination>>()
            .iter(app.world())
            .collect();
        for destination in away {
            app.world_mut().despawn(destination);
        }
        let back = spawn_level(&mut app, floor());
        app.update();
        assert!(is_played(&app, back));
        assert!(app.world().get_entity(first).is_err());
        let world = app.world_mut();
        let (entity, monster, pose) =
            world.query::<(Entity, &Monster, &Transform)>().single(world).unwrap();
        assert_eq!(monster.body, body);
        assert!(pose.translation.distance(feet) < 0.00001);
        assert!((pose.rotation.to_euler(EulerRot::YXZ).0 - 0.7).abs() < 0.001);
        assert!(world.get::<Walker>(entity).is_none());
        assert!(world.get::<MonsterVisual>(entity).is_none());
    }
}
