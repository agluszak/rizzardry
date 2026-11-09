//! Small headless scenarios at the level/party ownership boundary: a level
//! becomes active in one step once its prerequisites exist, and travel
//! keeps the source authoritative until the destination can commit, or
//! leaves it untouched when destination preparation fails.

use crate::content::items::ItemCatalog;
use crate::content::level::{CollisionMesh, LevelDefinition, MonsterPlacement};
use crate::content::monster::MonsterBody;
use crate::content::species::{MonsterCatalog, Species};
use crate::game::characters::{Character, Gender, Profession, Race};
use crate::game::data::Catalogs;
use crate::game::inventory::ItemIds;
use crate::game::party::body::{PartyBody, PartyMotionConfig};
use crate::game::party::roster::Party;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;
use crate::game::world::level::{
    Destination, Entry, Level, LevelMemory, LevelOwned, LevelReady, Travel, activate_level, travel,
};
use crate::game::world::population::{Monster, MonsterBodies};
use crate::game::world::triggers::PendingPartyMove;
use bevy::prelude::*;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(crate::game::npcs::NpcPlugin)
        .init_resource::<crate::game::time::GameClock>()
        .init_asset::<LevelDefinition>()
        .init_asset::<bevy_landmass::NavMesh3d>()
        .init_asset::<MonsterBody>()
        .init_asset::<crate::content::ui::TextTable>()
        .init_resource::<Assets<PartyMotionConfig>>()
        .init_resource::<Assets<crate::game::world::navigation::WalkerMotionConfig>>()
        .init_resource::<LevelMemory>()
        .add_observer(give_bodies)
        .add_observer(crate::game::world::damage::monster_died)
        .add_systems(
            Update,
            (crate::game::characters::sync_member_maxima, crate::game::characters::settle_deaths)
                .chain()
                .before(activate_level),
        )
        .init_resource::<crate::game::factions::Factions>()
        .add_message::<crate::game::factions::FactionNews>()
        .init_resource::<ItemIds>()
        .init_resource::<GameRng>()
        .add_message::<LevelReady>()
        .add_message::<Travel>()
        .add_systems(PreUpdate, travel)
        .add_systems(Update, activate_level.in_set(crate::game::GameplaySet::LevelActivation));
    app.finish();
    app.cleanup();
    app
}

/// The shared databases, nearly empty (one species), as gameplay needs them
/// loaded.
fn with_data(app: &mut App) {
    let world = app.world_mut();
    let crab = Species { name: "Crab".into(), model: "crab".into(), ..default() };
    world.insert_resource(Catalogs {
        items: starting_catalog(),
        species: MonsterCatalog(vec![crab]),
        ..default()
    });
    // Required body metadata is available without a visual asset or renderer.
    let body = world.resource_mut::<Assets<MonsterBody>>().add(MonsterBody::default());
    for mut destination in world.query::<&mut Destination>().iter_mut(world) {
        destination.bodies.get_or_insert_default().0.insert(0, body.clone());
    }
    world.insert_resource(CrabBody(body));
}

/// The crab's body, which a destination would otherwise load from retail.
#[derive(Resource)]
struct CrabBody(Handle<MonsterBody>);

fn give_bodies(
    add: On<Add<Destination>>,
    body: Option<Res<CrabBody>>,
    mut destinations: Query<&mut Destination>,
) {
    let (Some(body), Ok(mut destination)) = (body, destinations.get_mut(add.entity)) else {
        return;
    };
    destination.bodies.get_or_insert_default().0.insert(0, body.0.clone());
}

/// Plain items under every id a new character can start with.
fn starting_catalog() -> ItemCatalog {
    use crate::content::items::{ItemDefinition, ItemId};
    ItemCatalog(
        (0..800u16)
            .map(|id| ItemDefinition {
                id: ItemId(id),
                name: format!("item {id}"),
                professions: enumset::EnumSet::all(),
                races: enumset::EnumSet::all(),
                genders: enumset::EnumSet::all(),
                ..default()
            })
            .collect(),
    )
}

fn test_level(app: &mut App, collision: CollisionMesh) -> Level {
    let crab = MonsterPlacement {
        location: 7,
        group: 1,
        species: 0,
        position: Vec3::ZERO,
        yaw: 0.0,
        unborn: false,
    };
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

/// A prop the party walks through is built without collision.
#[test]
fn a_walk_through_prop_without_collision_is_entered() {
    use crate::content::level::{PropModel, PropModelPart};
    let (mut app, _) = playing();
    let party = party_entities(&mut app);
    let prop = PropModel {
        name: "broken".into(),
        collidable: false,
        face_camera: false,
        frames_per_second: 10.0,
        parts: vec![PropModelPart {
            transform: Transform::IDENTITY,
            keys: vec![],
            collision: CollisionMesh::default(),
        }],
        wraps: false,
        slots: vec![],
        trigger: None,
    };
    let enter = |app: &mut App| {
        let level = test_level(app, floor());
        let mut definition = app
            .world()
            .resource::<Assets<LevelDefinition>>()
            .get(&level.definition)
            .unwrap()
            .clone();
        definition.props = vec![prop];
        app.world_mut()
            .resource_mut::<Assets<LevelDefinition>>()
            .insert(&level.definition, definition)
            .unwrap();
        app.world_mut().spawn(Destination::new(level, default(), Entry::Travel)).id()
    };
    let destination = enter(&mut app);
    app.update();
    assert!(is_played(&app, destination));
    assert_eq!(party_entities(&mut app), party);
}

#[test]
fn a_level_waits_for_the_shared_databases() {
    let mut app = app();
    let root = spawn_level(&mut app, floor());

    app.update();
    assert!(!is_played(&app, root));
    assert!(party_entities(&mut app).is_empty());

    with_data(&mut app);
    app.update();
    assert!(is_played(&app, root));
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

fn monsters(app: &mut App) -> Vec<Monster> {
    let world = app.world_mut();
    let mut query = world.query::<&Monster>();
    query.iter(world).cloned().collect()
}

#[test]
fn body_metadata_loads_even_when_the_visual_cycles_fail() {
    use crate::content::monster::MonsterModel;
    use crate::retail::monster_loader::{MlsBodyLoader, MlsLoader};
    use bevy::asset::LoadState;

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
    use crate::content::monster::MonsterModel;
    use crate::game::world::navigation::Walker;
    use crate::game::world::population::ScaledBody;
    use crate::presentation::monsters::{MonsterVisual, build_monster_instances};

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
            crate::game::world::navigation::feet(pose.translation, Some(walker)).distance(feet)
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
    app.world_mut().insert_resource(CrabBody(loading.clone()));
    let root = spawn_level(&mut app, floor());

    app.update();
    assert!(!is_played(&app, root));

    let body = MonsterBody { fight_radius: 0.5, scale: 2.0, movement_rate: 1.5, ..default() };
    app.world_mut().resource_mut::<Assets<MonsterBody>>().insert(&loading, body).unwrap();
    app.update();
    assert!(is_played(&app, root));
    let world = app.world_mut();
    let crab = world.query::<&Monster>().single(world).unwrap();
    assert_eq!((crab.body.scale, crab.body.radius), (2.0, 1.0), "its body at its size");
    let held = world.get::<MonsterBodies>(root).expect("the played level holds its bodies");
    assert_eq!(held.0[&0], loading, "for monsters spawned while it is played");
    use crate::game::world::navigation::{Walker, WalkerMotion, feet};
    use bevy_tnua::TnuaController;
    let (at, walker, _) = world
        .query_filtered::<(&Transform, &Walker, &TnuaController<WalkerMotion>), With<Monster>>()
        .single(world)
        .unwrap();
    assert_eq!(walker.speed, 3.0);
    assert_eq!(feet(at.translation, Some(walker)), Vec3::ZERO);
    assert!(
        world.query::<&crate::game::combat::Battle>().iter(world).next().is_none(),
        "walking bodies exist before combat, without visual assets"
    );
}

#[test]
fn group_knowledge_survives_revisit() {
    use crate::game::world::identify::{GroupMember, MonsterGroup};
    let (mut app, _) = playing();
    let monster =
        app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
    let provoked = crate::game::world::hostility::SetStance { stance: Stance::Hostile, at: 7 };
    let old = app.world().get::<GroupMember>(monster).unwrap().0;
    let mut group = app.world_mut().get_mut::<MonsterGroup>(old).unwrap();
    group.identified = true;
    group.counted = true;
    group.set = Some(provoked);
    let back = revisit(&mut app);
    assert!(is_played(&app, back));
    let world = app.world_mut();
    let membership = world.query_filtered::<&GroupMember, With<Monster>>().single(world).unwrap();
    let group = world.get::<MonsterGroup>(membership.0).unwrap();
    assert!(group.identified && group.counted);
    assert_eq!(group.set, Some(provoked), "revisiting does not forgive provoked aggression");
}

#[test]
fn direct_damage_settles_death_once() {
    use crate::content::items::{Dice, ItemDefinition, ItemId};
    use crate::content::species::{Drop, Treasure};
    use crate::game::inventory::PartyInventory;
    use crate::game::world::items::WorldItem;
    use bevy::ecs::system::RunSystemOnce;
    let mut app = app();
    with_data(&mut app);
    {
        let mut catalogs = app.world_mut().resource_mut::<Catalogs>();
        catalogs.items.0.push(ItemDefinition { id: ItemId(0), ..default() });
        let kind = &mut catalogs.species.0[0];
        kind.hit_points = Dice { base: 10, ..default() };
        kind.gold = Dice { base: 7, ..default() };
        kind.treasure = vec![Treasure {
            drop: Drop::Item(ItemId(0)),
            chance: 100,
            dice: Dice { base: 2, ..default() },
        }];
    }
    spawn_level(&mut app, floor());
    app.update();
    let world = app.world_mut();
    let target = world.query_filtered::<Entity, With<Monster>>().single(world).unwrap();
    let hit = |world: &mut World| {
        let lost = world.get_mut::<crate::game::characters::Vitals>(target).unwrap().hurt(10);
        // Death settles its consequences.
        world.run_system_once(crate::game::characters::settle_deaths).unwrap();
        lost
    };
    use crate::game::world::navigation::Walker;
    use avian3d::prelude::Position;
    let physical = Vec3::new(20.0, 3.0, 40.0);
    world.entity_mut(target).insert((Position(physical), Walker { speed: 2.0, float_height: 1.0 }));
    assert_eq!(hit(world), 10);
    assert!(world.get::<crate::game::characters::Vitals>(target).unwrap().is_dead());
    assert!(
        world
            .query_filtered::<&Transform, With<WorldItem>>()
            .iter(world)
            .all(|at| { at.translation == physical - Vec3::Y + Vec3::Y * 0.25 })
    );
    hit(world);
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
fn departure_keeps_deaths_made_while_destination_loads() {
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

    // What the source holds at the commit is what is remembered.
    let world = app.world_mut();
    world
        .query_filtered::<&mut crate::game::characters::Vitals, With<Monster>>()
        .single_mut(world)
        .unwrap()
        .health
        .current = 0;
    world
        .resource_mut::<Assets<LevelDefinition>>()
        .insert(&loading, LevelDefinition { collision: floor(), ..default() })
        .unwrap();
    app.update();
    assert!(is_played(&app, target));
    assert!(app.world().get_entity(source).is_err());

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
    use crate::game::party::walk_in::move_party;
    use crate::game::world::triggers::PartyMove;
    use bevy::ecs::system::RunSystemOnce;
    let handle = app.world().get::<Level>(source).unwrap().definition.clone();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&handle)
        .unwrap()
        .camera_paths
        .push(crate::content::level::CameraPathModel {
            name: "travel flight".into(),
            keys: vec![Transform::default()],
            keys_per_second: 1.0,
        });
    app.world_mut().entity_mut(source).insert(PendingPartyMove(PartyMove::Fly(0)));
    app.world_mut().run_system_once(move_party).unwrap();
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
    // Damage while loading must survive same-path replacement. An early
    // snapshot would resurrect this monster at the commit boundary.
    let monster =
        app.world_mut().query_filtered::<Entity, With<Monster>>().single(app.world()).unwrap();
    app.world_mut().get_mut::<crate::game::characters::Vitals>(monster).unwrap().health.current = 0;
    app.world_mut().resource_mut::<Assets<LevelDefinition>>().insert(&loading, geometry).unwrap();
    app.update();
    assert!(is_played(&app, destination));
    assert!(app.world().get_entity(source).is_err());
    assert!(monsters(&mut app).is_empty());
    assert_eq!(
        app.world().get::<Position>(party).unwrap().0,
        Vec3::new(9.0, 4.0, 2.0) - Vec3::Y * crate::game::party::body::EYE_ABOVE_CENTRE
    );
}

#[test]
fn retail_level_definitions_load_without_render_asset_resources() {
    use crate::content::level::LevelGeometry;
    use crate::retail::level_loader::{PvlDefinitionLoader, PvlLoader};
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
    let files = std::sync::Arc::new(crate::retail::files::RetailFiles::open(directory).unwrap());
    let mut app = App::new();
    app.register_asset_source("retail", crate::retail::source::retail_source(files))
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
}

#[test]
fn prop_motion_resumes_from_its_actual_frame_after_a_revisit() {
    use crate::content::level::{PropModel, PropModelPart};
    use crate::game::world::props::{Prop, PropMotion};
    let (mut app, source) = playing();
    let definition = app.world().get::<Level>(source).unwrap().definition.clone();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&definition)
        .unwrap()
        .props
        .push(PropModel {
            name: "moving".into(),
            collidable: false,
            face_camera: false,
            frames_per_second: 1.,
            wraps: false,
            slots: vec![],
            trigger: None,
            parts: vec![PropModelPart {
                transform: Transform::IDENTITY,
                collision: CollisionMesh::default(),
                keys: vec![Transform::IDENTITY, Transform::from_translation(Vec3::X)],
            }],
        });
    // This is the live motion at departure, independent of the authored starting pose.
    app.world_mut().spawn((
        Prop { definition: 0 },
        PropMotion { frame: 0.25, target: 1. },
        LevelOwned(source),
    ));
    let mut destination = app.world().get::<Level>(source).unwrap().clone();
    let mut away = test_level(&mut app, floor());
    away.path = "levels/away/away.pvl".into();
    app.world_mut().spawn(Destination::new(away, default(), Entry::Travel));
    app.update();
    // The source asset remains the source of shared definitions on the return.
    destination.definition = definition;
    app.world_mut().spawn(Destination::new(destination, default(), Entry::Travel));
    app.update();
    let motion = app.world_mut().query::<&PropMotion>().single(app.world()).unwrap();
    assert_eq!((motion.frame, motion.target), (0.25, 1.));
}

/// An unborn group waits outside the world, rolled with the rest, through
/// a revisit; its birth brings it in once, and it stays born on the next
/// revisit (retail `GiveBirthToMonster`).
#[test]
fn an_unborn_group_waits_until_born_and_is_born_once() {
    use crate::game::world::identify::{GroupMember, MonsterGroup};
    use crate::game::world::population::{Birth, Unborn, give_birth};
    let mut app = app();
    app.add_message::<Birth>().add_systems(Update, give_birth.after(activate_level));
    with_data(&mut app);
    let level = test_level(&mut app, floor());
    {
        let mut definitions = app.world_mut().resource_mut::<Assets<LevelDefinition>>();
        let mut definition = definitions.get_mut(&level.definition).unwrap();
        definition.monsters.push(MonsterPlacement {
            location: 8,
            group: 2,
            species: 0,
            position: Vec3::X,
            yaw: 0.0,
            unborn: true,
        });
    }
    let new_game = Entry::NewGame(vec![(
        Character::new("A", Race::Human, Gender::Male, Profession::Fighter),
        default(),
    )]);
    let root = app.world_mut().spawn(Destination::new(level, default(), new_game)).id();
    app.update();
    assert!(is_played(&app, root));
    assert_eq!(monsters(&mut app).len(), 1, "the unborn one is not in the world");
    assert_eq!(app.world().get::<Unborn>(root).unwrap().0.len(), 1);
    let root = revisit(&mut app);
    assert_eq!(monsters(&mut app).len(), 1);
    assert_eq!(app.world().get::<Unborn>(root).unwrap().0.len(), 1, "it waits across a revisit");
    for _ in 0..2 {
        app.world_mut().write_message(Birth(2));
        app.update();
        let born: Vec<_> = monsters(&mut app).into_iter().filter(|m| m.location == 8).collect();
        assert_eq!(born.len(), 1, "born once");
    }
    let world = app.world_mut();
    let (_, membership) = world
        .query::<(&Monster, &GroupMember)>()
        .iter(world)
        .find(|(m, _)| m.location == 8)
        .unwrap();
    assert_eq!(world.get::<MonsterGroup>(membership.0).unwrap().authored, 2);
    let root = revisit(&mut app);
    assert_eq!(monsters(&mut app).len(), 2, "born for good");
    assert!(app.world().get::<Unborn>(root).unwrap().0.is_empty());
}
