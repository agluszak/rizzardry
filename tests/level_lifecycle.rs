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
        .init_resource::<rizzardry::game::inventory::PartyInventory>()
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
fn travel_dialogs_end_with_their_question_without_a_cleanup_system() {
    use rizzardry::game::world::level::{TravelAnswer, answer_travel_questions, ask_travel};
    use rizzardry::presentation::dialog::QuestionDialog;

    let mut app = app();
    app.add_message::<TravelAnswer>().add_systems(Update, answer_travel_questions);
    let level = test_level(&mut app, floor());
    let level = app.world_mut().spawn(level).id();
    let old = ask_travel(app.world_mut(), level, "first".into()).unwrap();
    let old_dialog = app.world_mut().spawn(QuestionDialog(old)).id();
    let current = ask_travel(app.world_mut(), level, "second".into()).unwrap();
    assert!(app.world().get_entity(old_dialog).is_err());

    let dialog = app.world_mut().spawn(QuestionDialog(current)).id();
    app.world_mut().write_message(TravelAnswer { question: old, accept: true });
    app.update();
    assert!(app.world().get_entity(dialog).is_ok(), "a stale answer cannot close the new view");
    app.world_mut().write_message(TravelAnswer { question: current, accept: false });
    app.update();
    assert!(app.world().get_entity(dialog).is_err());

    let question = ask_travel(app.world_mut(), level, "third".into()).unwrap();
    let dialog = app.world_mut().spawn(QuestionDialog(question)).id();
    app.world_mut().despawn(level);
    assert!(app.world().get_entity(dialog).is_err(), "leaving ends the pending view");
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
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn attaching_visuals_does_not_change_physical_state_or_revisit_placement() {
    use rizzardry::content::monster::MonsterModel;
    use rizzardry::game::world::navigation::Walker;
    use rizzardry::game::world::population::ScaledBody;
    use rizzardry::presentation::monsters::{MonsterVisual, build_monster_instances};

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
            assert_eq!(*app.world().get::<Transform>(actor).unwrap(), pose);
            app.world_mut()
                .resource_mut::<Assets<MonsterModel>>()
                .insert(&model, MonsterModel { cycles: default() })
                .unwrap();
            app.update();
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
        let (monster, pose) = world.query::<(&Monster, &Transform)>().single(world).unwrap();
        assert_eq!(monster.body, body);
        assert!(pose.translation.distance(feet) < 0.00001);
        assert!((pose.rotation.to_euler(EulerRot::YXZ).0 - 0.7).abs() < 0.001);
    }
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
fn group_knowledge_survives_revisit() {
    use rizzardry::game::world::identify::{GroupMember, MonsterGroup};
    let mut app = app();
    with_data(&mut app);
    spawn_level(&mut app, floor());
    app.update();
    let old =
        app.world_mut().query_filtered::<Entity, With<MonsterGroup>>().single(app.world()).unwrap();
    let mut group = app.world_mut().get_mut::<MonsterGroup>(old).unwrap();
    group.identified = true;
    group.counted = true;
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.world_mut().run_schedule(PreUpdate);
    let away: Vec<_> =
        app.world_mut().query_filtered::<Entity, With<Destination>>().iter(app.world()).collect();
    for entity in away {
        app.world_mut().despawn(entity);
    }
    let back = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, back));
    let world = app.world_mut();
    let membership = world.query_filtered::<&GroupMember, With<Monster>>().single(world).unwrap();
    let group = world.get::<MonsterGroup>(membership.0).unwrap();
    assert!(group.identified && group.counted);
}

#[test]
fn direct_damage_settles_death_once_and_rejection_leaves_the_target_unchanged() {
    use bevy::ecs::system::RunSystemOnce;
    use rizzardry::content::items::{Dice, ItemDefinition, ItemId};
    use rizzardry::content::species::{Drop, Treasure};
    use rizzardry::game::inventory::PartyInventory;
    use rizzardry::game::world::damage::MonsterDamage;
    use rizzardry::game::world::items::WorldItem;
    let mut app = app();
    with_data(&mut app);
    let (items, species) = {
        let d = app.world().resource::<GameData>();
        (d.items.clone(), d.species.clone())
    };
    app.world_mut()
        .resource_mut::<Assets<ItemCatalog>>()
        .get_mut(&items)
        .unwrap()
        .0
        .push(ItemDefinition { id: ItemId(0), ..default() });
    {
        let mut catalogs = app.world_mut().resource_mut::<Assets<MonsterCatalog>>();
        let mut catalog = catalogs.get_mut(&species).unwrap();
        let kind = &mut catalog.0[0];
        kind.hit_points = Dice { base: 10, ..default() };
        kind.gold = Dice { base: 7, ..default() };
        kind.treasure = vec![Treasure {
            drop: Drop::Item(ItemId(0)),
            chance: 100,
            dice: Dice { base: 2, ..default() },
        }];
    }
    let level = spawn_level(&mut app, floor());
    app.update();
    let world = app.world_mut();
    let target = world.query_filtered::<Entity, With<Monster>>().single(world).unwrap();
    let hit = |world: &mut World| {
        world
            .run_system_once(move |mut damage: MonsterDamage, mut rng: ResMut<GameRng>| {
                damage.apply(target, 10, 1, &mut rng)
            })
            .unwrap()
    };
    use avian3d::prelude::{LinearVelocity, Position};
    use bevy_landmass::{AgentTarget3d, UsingAnimationLink};
    use bevy_tnua::{TnuaController, TnuaMotor};
    use rizzardry::game::world::navigation::{Walker, WalkerMotion, links::Crossing};
    let physical = Vec3::new(20.0, 3.0, 40.0);
    let mut controller = TnuaController::<WalkerMotion>::default();
    controller.basis.desired_motion = Vec3::X;
    world.entity_mut(target).insert((
        Position(physical),
        LinearVelocity(Vec3::new(2.0, -1.0, 3.0)),
        Walker { speed: 2.0, float_height: 1.0 },
        Crossing { to: Vec3::splat(100.0) },
        UsingAnimationLink,
        AgentTarget3d::Point(Vec3::splat(100.0)),
        controller,
    ));
    world.get_mut::<TnuaMotor>(target).unwrap().lin.boost = Vec3::X;
    let before = world.resource::<GameRng>().clone();
    world.entity_mut(level).insert(Departing);
    assert!(hit(world).is_err());
    assert_eq!(world.get::<Monster>(target).unwrap().hp, 10);
    assert_eq!(*world.resource::<GameRng>(), before);
    world.entity_mut(level).remove::<Departing>();
    assert!(hit(world).unwrap().died);
    assert!(world.get::<Crossing>(target).is_none());
    assert!(world.get::<UsingAnimationLink>(target).is_none());
    assert!(matches!(world.get::<AgentTarget3d>(target), Some(AgentTarget3d::None)));
    assert_eq!(
        world.get::<TnuaController<WalkerMotion>>(target).unwrap().basis.desired_motion,
        Vec3::ZERO
    );
    assert_eq!(world.get::<LinearVelocity>(target).unwrap().0, Vec3::NEG_Y);
    assert_eq!(world.get::<TnuaMotor>(target).unwrap().lin.boost, Vec3::ZERO);
    assert_eq!(world.get::<Walker>(target).unwrap().float_height, 1.0);
    assert!(
        world
            .query_filtered::<&Transform, With<WorldItem>>()
            .iter(world)
            .all(|at| { at.translation == physical - Vec3::Y + Vec3::Y * 0.25 })
    );
    let after = world.resource::<GameRng>().clone();
    hit(world).unwrap();
    assert_eq!(*world.resource::<GameRng>(), after);
    assert_eq!(world.resource::<PartyInventory>().gold, 7);
    let units = world
        .query::<&WorldItem>()
        .iter(world)
        .flat_map(|w| std::iter::once(&w.item).chain(w.contents.iter()))
        .filter(|i| i.item == ItemId(0))
        .map(|i| i.units())
        .sum::<u32>();
    assert_eq!(units, 2, "death preserves every drop and cannot award it twice");
}
