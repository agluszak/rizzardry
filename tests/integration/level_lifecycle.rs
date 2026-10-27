//! Small headless scenarios at the level/party ownership boundary: a level
//! becomes active in one step once its prerequisites exist, and travel
//! keeps the source authoritative until the destination can commit, or
//! leaves it untouched when destination preparation fails.

use avian3d::prelude::{Collider, RigidBodyDisabled};
use bevy::prelude::*;
use rizzardry::content::items::{ItemCatalog, LootTables};
use rizzardry::content::level::{CollisionMesh, LevelDefinition, MonsterPlacement};
use rizzardry::content::monster::MonsterBody;
use rizzardry::content::species::{MonsterCatalog, Species};
use rizzardry::game::characters::{Character, Gender, Profession, Race};
use rizzardry::game::data::GameData;
use rizzardry::game::inventory::ItemIds;
use rizzardry::game::party::body::{PartyBody, PartyMotionConfig};
use rizzardry::game::party::roster::Party;
use rizzardry::game::rng::GameRng;
use rizzardry::game::world::level::{
    Destination, Entry, Level, LevelEntryFailed, LevelMemory, LevelOwned, LevelReady, Travel,
    activate_level, travel,
};
use rizzardry::game::world::population::{Monster, MonsterBodies};
use rizzardry::game::world::triggers::PendingPartyMove;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(rizzardry::game::npcs::NpcPlugin)
        .init_resource::<rizzardry::game::time::GameClock>()
        .init_asset::<LevelDefinition>()
        .init_asset::<ItemCatalog>()
        .init_asset::<LootTables>()
        .init_asset::<MonsterCatalog>()
        .init_asset::<rizzardry::content::spells::SpellCatalog>()
        .init_asset::<bevy_landmass::NavMesh3d>()
        .init_asset::<MonsterBody>()
        .init_asset::<rizzardry::content::ui::TextTable>()
        .init_resource::<Assets<PartyMotionConfig>>()
        .init_resource::<Assets<rizzardry::game::world::navigation::WalkerMotionConfig>>()
        .init_resource::<LevelMemory>()
        .init_resource::<MonsterBodies>()
        .init_resource::<rizzardry::game::factions::Factions>()
        .init_resource::<ItemIds>()
        .init_resource::<GameRng>()
        .add_message::<LevelReady>()
        .add_message::<LevelEntryFailed>()
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
    let spells = world
        .resource_mut::<Assets<rizzardry::content::spells::SpellCatalog>>()
        .add(rizzardry::content::spells::SpellCatalog::default());
    world.insert_resource(GameData { items, loot, species, spells });
    // Required body metadata is available without a visual asset or renderer.
    let body = world.resource_mut::<Assets<MonsterBody>>().add(MonsterBody::default());
    world.resource_mut::<MonsterBodies>().0.insert(0, body);
}

fn test_level(app: &mut App, collision: CollisionMesh) -> Level {
    let crab =
        MonsterPlacement { location: 7, group: 1, species: 0, position: Vec3::ZERO, yaw: 0.0 };
    let geometry = app.world_mut().resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition {
        collision,
        monsters: vec![crab],
        ..default()
    });
    Level {
        path: "levels/test/test.pvl".into(),
        definition: geometry,
        navmeshes: vec![],
        script: default(),
        messages: None,
    }
}

/// The test level, to be entered: a new game's first, or by travel once
/// there is a party.
fn spawn_level(app: &mut App, collision: CollisionMesh) -> Entity {
    let level = test_level(app, collision);
    let world = app.world_mut();
    let entry = if world.query::<&Party>().iter(world).next().is_some() {
        Entry::Travel
    } else {
        Entry::NewGame(vec![(
            Character::new("A", Race::Human, Gender::Male, Profession::Fighter),
            default(),
        )])
    };
    world.spawn(Destination::new(level, default(), entry)).id()
}

fn party_entities(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<PartyBody>>();
    query.iter(world).collect()
}

fn is_played(app: &App, level: Entity) -> bool {
    app.world().get::<Level>(level).is_some()
}

/// The entry was discarded and its failure announced.
fn failed(app: &App, destination: Entity) -> bool {
    app.world().get_entity(destination).is_err()
        && !app.world().resource::<Messages<LevelEntryFailed>>().is_empty()
}

fn floor() -> CollisionMesh {
    CollisionMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Z],
        triangles: vec![[0, 1, 2]],
        walkable: vec![true],
    }
}

/// The test level on a floor, with the shared databases, activated.
fn playing() -> (App, Entity) {
    let mut app = app();
    with_data(&mut app);
    let level = spawn_level(&mut app, floor());
    app.update();
    (app, level)
}

/// Leave through the real travel boundary, then come back to the test level
/// instead of the (unloadable) destination; returns the level played again.
fn revisit(app: &mut App) -> Entity {
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.world_mut().run_schedule(PreUpdate);
    let away: Vec<Entity> =
        app.world_mut().query_filtered::<Entity, With<Destination>>().iter(app.world()).collect();
    for destination in away {
        app.world_mut().despawn(destination);
    }
    let back = spawn_level(app, floor());
    app.update();
    back
}

/// Update until `done`, failing after five seconds of asynchronous loading.
fn update_until(app: &mut App, what: &str, done: impl Fn(&World) -> bool) {
    let started = std::time::Instant::now();
    loop {
        app.update();
        if done(app.world()) {
            return;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{what} within 5 s");
        std::thread::yield_now();
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
    let (mut app, first_level) = playing();
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
    let (mut app, first_level) = playing();
    let party = party_entities(&mut app)[0];

    // MO2's files are not there to load here: the destination fails.
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    app.world_mut().run_schedule(PreUpdate);
    assert!(is_played(&app, first_level));
    assert!(app.world().get::<RigidBodyDisabled>(party).is_none());
    assert!(app.world().resource::<LevelMemory>().is_empty(), "no snapshot before departure");
    for _ in 0..20 {
        app.update();
    }

    assert!(is_played(&app, first_level), "the source remains playable");
    assert!(app.world().get::<RigidBodyDisabled>(party).is_none());
    assert!(
        app.world().resource::<LevelMemory>().is_empty(),
        "failure never creates a dormant record"
    );
    assert_eq!(party_entities(&mut app), vec![party]);
}

fn monsters(app: &mut App) -> Vec<Monster> {
    let world = app.world_mut();
    let mut query = world.query::<&Monster>();
    query.iter(world).cloned().collect()
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
    update_until(&mut app, "body/visual loads settling independently", |world| {
        let assets = world.resource::<AssetServer>();
        matches!(assets.get_load_state(&body), Some(LoadState::Loaded))
            && matches!(assets.get_load_state(&visual), Some(LoadState::Failed(_)))
    });
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
        let (mut app, first) = playing();
        let actor =
            app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
        let feet = Vec3::new(2.0, 3.0, 4.0);
        let body = ScaledBody { scale: 1.4, radius: 0.75, eye_height: 1.9, speed: 3.0 };
        let float_height = 1.1;
        let pose = Transform::from_translation(feet + Vec3::Y * float_height)
            .with_rotation(Quat::from_rotation_y(0.7));
        app.world_mut().get_mut::<Monster>(actor).unwrap().body = body;
        app.world_mut().entity_mut(actor).insert((
            Walker { speed: body.speed, float_height },
            pose,
            avian3d::prelude::Position(pose.translation),
            avian3d::prelude::Rotation(pose.rotation),
        ));
        if show_visuals {
            app.init_resource::<Assets<MonsterModel>>()
                .init_resource::<Assets<StandardMaterial>>()
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
        let back = revisit(&mut app);
        assert!(is_played(&app, back));
        assert!(app.world().get_entity(first).is_err());
        let world = app.world_mut();
        let (monster, pose, walker) =
            world.query::<(&Monster, &Transform, &Walker)>().single(world).unwrap();
        assert_eq!(monster.body, body);
        assert!(
            rizzardry::game::world::navigation::feet(pose.translation, Some(walker)).distance(feet)
                < 0.00001
        );
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

    let body = MonsterBody { fight_radius: 0.5, scale: 2.0, movement_rate: 1.5, ..default() };
    app.world_mut().resource_mut::<Assets<MonsterBody>>().insert(&loading, body).unwrap();
    app.update();
    assert!(is_played(&app, root));
    let world = app.world_mut();
    let crab = world.query::<&Monster>().single(world).unwrap();
    assert_eq!((crab.body.scale, crab.body.radius), (2.0, 1.0), "its body at its size");
    use bevy_tnua::TnuaController;
    use rizzardry::game::world::navigation::{Walker, WalkerMotion, feet};
    let (at, walker, _) = world
        .query_filtered::<(&Transform, &Walker, &TnuaController<WalkerMotion>), With<Monster>>()
        .single(world)
        .unwrap();
    assert_eq!(walker.speed, 3.0);
    assert_eq!(feet(at.translation, Some(walker)), Vec3::ZERO);
    assert!(
        world.query::<&rizzardry::game::combat::Battle>().iter(world).next().is_none(),
        "walking bodies exist before combat, without visual assets"
    );
}

#[test]
fn group_knowledge_survives_revisit() {
    use rizzardry::game::world::identify::{GroupMember, MonsterGroup};
    let (mut app, _) = playing();
    let monster =
        app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
    app.world_mut().entity_mut(monster).insert(rizzardry::game::world::population::Provoked);
    let old =
        app.world_mut().query_filtered::<Entity, With<MonsterGroup>>().single(app.world()).unwrap();
    let mut group = app.world_mut().get_mut::<MonsterGroup>(old).unwrap();
    group.identified = true;
    group.counted = true;
    let back = revisit(&mut app);
    assert!(is_played(&app, back));
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<rizzardry::game::world::population::Provoked>>()
            .iter(app.world())
            .count(),
        1,
        "revisiting does not forgive provoked aggression",
    );
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
    let source = world.entity_mut(level).take::<Level>().unwrap();
    assert!(hit(world).is_err());
    assert_eq!(world.get::<Monster>(target).unwrap().hp, 10);
    assert_eq!(*world.resource::<GameRng>(), before);
    world.entity_mut(level).insert(source);
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
    assert_eq!(world.query::<&PartyInventory>().single(world).unwrap().gold, 7);
    let units = world
        .query::<&WorldItem>()
        .iter(world)
        .flat_map(|w| w.stacks().iter())
        .filter(|i| i.item == ItemId(0))
        .map(|i| i.units())
        .sum::<u32>();
    assert_eq!(units, 2, "death preserves every drop and cannot award it twice");
}

#[test]
fn entry_suspends_gameplay_and_departure_captures_the_source_at_commit() {
    use bevy::ecs::system::RunSystemOnce;
    use rizzardry::game::world::level::level_active;
    let (mut app, source) = playing();
    let party = party_entities(&mut app)[0];
    let loading = app.world_mut().resource_mut::<Assets<LevelDefinition>>().reserve_handle();
    let mut destination = test_level(&mut app, floor());
    destination.path = "destination".into();
    destination.definition = loading.clone();
    let target =
        app.world_mut().spawn(Destination::new(destination, default(), Entry::Travel)).id();
    app.update();
    assert!(is_played(&app, source), "the source stays intact while the entry loads");
    assert!(!is_played(&app, target));
    assert!(!app.world_mut().run_system_once(level_active).unwrap(), "gameplay is suspended");
    assert!(app.world().get::<RigidBodyDisabled>(party).is_none());
    assert!(app.world().resource::<LevelMemory>().is_empty());

    // What the source holds at the commit is what is remembered.
    let world = app.world_mut();
    world.query::<&mut Monster>().single_mut(world).unwrap().hp = 0;
    world
        .resource_mut::<Assets<LevelDefinition>>()
        .insert(&loading, LevelDefinition { collision: floor(), ..default() })
        .unwrap();
    app.update();
    assert!(is_played(&app, target));
    assert!(app.world_mut().run_system_once(level_active).unwrap(), "gameplay resumes");
    assert!(app.world().get_entity(source).is_err());
    assert!(!app.world().resource::<LevelMemory>().is_empty());

    let returned = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, returned));
    assert!(monsters(&mut app).is_empty(), "departure must remember the kill");
    assert_eq!(party_entities(&mut app), [party]);
}

#[test]
fn preparation_keeps_the_source_live_and_commit_captures_its_latest_state() {
    use avian3d::prelude::Position;
    let (mut app, source) = playing();
    let party = party_entities(&mut app)[0];
    let at = app.world().get::<Position>(party).unwrap().0;
    // Start through the real camera-path operation, not a hand-made marker.
    use bevy::ecs::system::RunSystemOnce;
    use rizzardry::game::party::walk_in::{PathFlight, move_party};
    use rizzardry::game::world::triggers::PartyMove;
    let handle = app.world().get::<Level>(source).unwrap().definition.clone();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&handle)
        .unwrap()
        .camera_paths
        .push(rizzardry::content::level::CameraPathModel {
            name: "travel flight".into(),
            keys: vec![Transform::default()],
            keys_per_second: 1.0,
        });
    app.world_mut().entity_mut(source).insert(PendingPartyMove(PartyMove::Fly(0)));
    app.world_mut().run_system_once(move_party).unwrap();
    assert!(app.world().get::<PathFlight>(party).is_some());
    let mut next = app.world().get::<Level>(source).unwrap().clone();
    let geometry =
        app.world().resource::<Assets<LevelDefinition>>().get(&next.definition).unwrap().clone();
    let loading = app.world_mut().resource_mut::<Assets<LevelDefinition>>().reserve_handle();
    next.definition = loading.clone();
    let mut destination = Destination::new(next.clone(), default(), Entry::Travel);
    destination.start.eye = Some(Vec3::new(9.0, 4.0, 2.0));
    let destination = app.world_mut().spawn(destination).id();
    app.update();
    assert!(is_played(&app, source));
    assert_eq!(app.world().get::<Position>(party).unwrap().0, at);
    assert!(app.world().resource::<LevelMemory>().is_empty());
    assert!(app.world().get::<PathFlight>(party).is_some());
    assert!(app.world().get::<RigidBodyDisabled>(party).is_some());
    // Damage while loading must survive same-path replacement. An early
    // snapshot would resurrect this monster at the commit boundary.
    let monster =
        app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
    app.world_mut().get_mut::<Monster>(monster).unwrap().hp = 0;
    app.world_mut().resource_mut::<Assets<LevelDefinition>>().insert(&loading, geometry).unwrap();
    app.update();
    assert!(is_played(&app, destination));
    assert!(app.world().get::<PathFlight>(party).is_none());
    assert!(app.world().get::<RigidBodyDisabled>(party).is_none());
    assert!(app.world().get_entity(source).is_err());
    assert!(monsters(&mut app).is_empty());
    assert_eq!(
        app.world().get::<Position>(party).unwrap().0,
        Vec3::new(9.0, 4.0, 2.0) - Vec3::Y * rizzardry::game::party::body::EYE_ABOVE_CENTRE
    );
}

#[test]
fn failed_navigation_is_a_failed_destination_not_degraded_activation() {
    let (mut app, source) = playing();
    let mut next = test_level(&mut app, floor());
    next.navmeshes.push(app.world().resource::<AssetServer>().load("missing.navmesh"));
    let destination = app.world_mut().spawn(Destination::new(next, default(), Entry::Travel)).id();
    update_until(&mut app, "the destination failing", |world| {
        world.get_entity(destination).is_err()
    });
    assert!(failed(&app, destination));
    assert!(is_played(&app, source));
    assert!(!is_played(&app, destination));
    assert!(app.world().resource::<LevelMemory>().is_empty());
}

#[test]
fn retail_level_definitions_load_without_render_asset_resources() {
    use rizzardry::content::level::LevelGeometry;
    use rizzardry::retail::level_loader::{PvlDefinitionLoader, PvlLoader};
    // An empty compiled level with one material. Even this material would need
    // StandardMaterial registration if gameplay still ran the visual loader.
    let mut pvl = vec![0; 8]; // mesh/root counts
    pvl.extend(1i16.to_le_bytes());
    pvl.extend([0; 0x11a]);
    pvl.extend([-1i32, -1].into_iter().flat_map(i32::to_le_bytes));
    pvl.extend(0i16.to_le_bytes()); // lights
    pvl.extend((-1i32).to_le_bytes());
    for _ in 0..8 {
        // paths, items, missiles, props, billboards, cameras, environment, triggers
        pvl.extend([0i32, -1].into_iter().flat_map(i32::to_le_bytes));
    }
    pvl.extend([0i32, 0, -1].into_iter().flat_map(i32::to_le_bytes)); // sway, map cuts
    pvl.extend([0i32, 0, 0, 0, -1].into_iter().flat_map(i32::to_le_bytes)); // offset, particles
    pvl.extend([0i32, -1, 0, 0, -1].into_iter().flat_map(i32::to_le_bytes)); // names, map
    let mut oct = vec![0; 0xf5];
    oct[..2].copy_from_slice(&0x22u16.to_le_bytes());
    oct[0x86..0x8a].copy_from_slice(&1u32.to_le_bytes()); // collision stream present
    oct.extend([-1i32, 0, -1, -1, -1, 1].into_iter().flat_map(i32::to_le_bytes));
    oct.extend([0; 100]); // empty collision tables and bounds
    oct.extend((-1i32).to_le_bytes());
    /// Removed however the test ends.
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = TempDir(
        std::env::temp_dir().join(format!("rizzardry-level-loader-{}", std::process::id())),
    );
    let directory = &directory.0;
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(directory.join("empty.pvl"), pvl).unwrap();
    std::fs::write(directory.join("empty.oct"), oct).unwrap();
    let files =
        std::sync::Arc::new(rizzardry::retail::files::RetailFiles::open(directory).unwrap());
    let mut app = App::new();
    app.register_asset_source("retail", rizzardry::retail::source::retail_source(files))
        .add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<LevelDefinition>()
        .init_asset::<LevelGeometry>()
        .init_asset_loader::<PvlDefinitionLoader>()
        .init_asset_loader::<PvlLoader>();
    app.finish();
    app.cleanup();
    let definition =
        app.world().resource::<AssetServer>().load::<LevelDefinition>("retail://empty.pvl");
    update_until(&mut app, "the definition loading", |world| {
        match world.resource::<AssetServer>().get_load_state(&definition) {
            Some(bevy::asset::LoadState::Failed(error)) => panic!("definition failed: {error}"),
            state => matches!(state, Some(bevy::asset::LoadState::Loaded)),
        }
    });
    assert!(app.world().resource::<Assets<LevelDefinition>>().contains(&definition));
    assert!(!app.world().contains_resource::<Assets<Mesh>>());
    assert!(!app.world().contains_resource::<Assets<StandardMaterial>>());
    assert!(app.world().resource::<Assets<LevelGeometry>>().is_empty());
}

#[test]
fn named_actor_memory_stock_and_world_role_survive_an_unloaded_map() {
    use rizzardry::content::{
        items::{ItemDefinition, ItemId},
        npcs::{DialogueScript, FactCatalog, NpcCatalog, NpcDefinition, NpcId, Quote, StockRule},
    };
    use rizzardry::game::npcs::{ActorId, Npc, NpcAssets, NpcBodies};
    let mut app = app();
    with_data(&mut app);
    let data = app.world().resource::<GameData>();
    let items = data.items.clone();
    let species = data.species.clone();
    app.world_mut()
        .resource_mut::<Assets<ItemCatalog>>()
        .get_mut(&items)
        .unwrap()
        .0
        .push(ItemDefinition { id: ItemId(0), ..default() });
    app.world_mut().resource_mut::<Assets<MonsterCatalog>>().get_mut(&species).unwrap().0[0].npc =
        Some(NpcId(0));
    let catalog =
        app.world_mut().resource_mut::<Assets<NpcCatalog>>().add(NpcCatalog(vec![NpcDefinition {
            source_name: "Fixture".into(),
            merchant: true,
            disposition: 80,
            owns_stock: true,
            stock: vec![StockRule { item: ItemId(0), quantity: 3, persistent: true }],
            ..default()
        }]));
    let facts = app.world_mut().resource_mut::<Assets<FactCatalog>>().add(FactCatalog::default());
    let script = app.world_mut().resource_mut::<Assets<DialogueScript>>().add(DialogueScript {
        quotes: vec![Quote { text: vec!["Greeting".into()], ..default() }],
        ..default()
    });
    app.insert_resource(NpcAssets { catalog, facts, scripts: [(NpcId(0), script)].into() });
    let source = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, source));
    let actor = app.world_mut().query::<(Entity, &Npc)>().single(app.world()).unwrap().0;
    assert!(app.world().get::<ChildOf>(actor).is_none());
    let body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    assert_ne!(body, actor);
    assert_eq!(app.world().get::<LevelOwned>(body).unwrap().0, source);
    {
        let mut npc = app.world_mut().get_mut::<Npc>(actor).unwrap();
        npc.greeting_pending = false;
        npc.paid_topics.push(7);
        npc.stock.lots[0].units = 2;
    }
    {
        let mut body = app.world_mut().get_mut::<Monster>(body).unwrap();
        body.hp = 4;
        body.max_hp = 9;
    }
    // A merchant re-enters at authored quote zero even after its greeting was remembered.
    use rizzardry::game::npcs::conversation::{self, Conversation, ConversationRequest};
    use rizzardry::game::screen::Screen;
    app.insert_resource(State::new(Screen::World)).init_resource::<NextState<Screen>>();
    let at = app.world().get::<Transform>(body).unwrap().translation;
    let party = party_entities(&mut app)[0];
    app.world_mut().get_mut::<avian3d::prelude::Position>(party).unwrap().0 = at + Vec3::X;
    for _ in 0..2 {
        let session = conversation::begin(app.world_mut(), actor).unwrap();
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((0, 0)));
        app.world_mut().write_message(ConversationRequest::Close { conversation: session });
        app.update();
        assert!(app.world().get_entity(session).is_err());
    }
    app.world_mut().entity_mut(body).insert(rizzardry::game::world::population::Provoked);
    let stock = app.world().get::<Npc>(actor).unwrap().stock.clone();
    let mut away = test_level(&mut app, floor());
    away.path = "levels/away/away.pvl".into();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&away.definition)
        .unwrap()
        .monsters
        .clear();
    let away = app.world_mut().spawn(Destination::new(away, default(), Entry::Travel)).id();
    app.update();
    assert!(is_played(&app, away));
    assert!(app.world().get_entity(source).is_err());
    assert!(app.world().get::<Monster>(actor).is_none());
    assert!(app.world().get_entity(body).is_err());
    assert!(app.world().get::<NpcBodies>(actor).is_none_or(|b| b.is_empty()));
    assert_eq!(app.world().get::<Npc>(actor).unwrap().dormant.as_ref().unwrap().body.monster.hp, 4);
    app.update();
    assert_eq!(app.world().get::<Npc>(actor).unwrap().stock, stock);
    let back = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, back));
    let returned_body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    assert_ne!(returned_body, body);
    assert_eq!(app.world().get::<LevelOwned>(returned_body).unwrap().0, back);
    assert_eq!(app.world().get::<Monster>(returned_body).unwrap().hp, 4);
    assert!(
        app.world().get::<rizzardry::game::world::population::Provoked>(returned_body).is_some()
    );
    let npc = app.world().get::<Npc>(actor).unwrap();
    assert_eq!(npc.stock, stock);
    assert_eq!(npc.paid_topics, [7]);
    assert!(!npc.greeting_pending);
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);
    assert_eq!(app.world().get::<ActorId>(actor), Some(&ActorId::Named(NpcId(0))));
}

#[test]
fn a_rendered_entry_waits_for_its_looks_and_fails_without_them() {
    use rizzardry::game::world::level::PresentationReadiness;
    let (mut app, source) = playing();
    let mut next = test_level(&mut app, floor());
    next.path = "shown".into();
    let shown = app
        .world_mut()
        .spawn((Destination::new(next, default(), Entry::Travel), PresentationReadiness::Waiting))
        .id();
    app.update();
    assert!(is_played(&app, source), "the commit waits for the appearance");
    app.world_mut().entity_mut(shown).remove::<PresentationReadiness>();
    app.update();
    assert!(is_played(&app, shown));

    let mut next = test_level(&mut app, floor());
    next.path = "unshowable".into();
    let unshowable = app
        .world_mut()
        .spawn((
            Destination::new(next, default(), Entry::Travel),
            PresentationReadiness::Failed("no appearance".into()),
        ))
        .id();
    app.update();
    assert!(failed(&app, unshowable));
    assert!(is_played(&app, shown), "a failed appearance leaves the level being played");
}
