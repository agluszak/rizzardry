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
    Destination, Entry, Level, LevelEntryFailed, LevelMemory, LevelOwned, LevelReady, Travel,
    activate_level, travel,
};
use crate::game::world::population::{Monster, MonsterBodies};
use crate::game::world::triggers::PendingPartyMove;
use avian3d::prelude::{Collider, RigidBodyDisabled};
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
        .init_resource::<crate::game::factions::Factions>()
        .add_message::<crate::game::factions::FactionNews>()
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
    let crab = Species { name: "Crab".into(), model: "crab".into(), ..default() };
    world.insert_resource(Catalogs {
        items: starting_catalog(),
        species: MonsterCatalog(vec![crab]),
        ..default()
    });
    // Required body metadata is available without a visual asset or renderer.
    let body = world.resource_mut::<Assets<MonsterBody>>().add(MonsterBody::default());
    for mut destination in world.query::<&mut Destination>().iter_mut(world) {
        destination.bodies.0.insert(0, body.clone());
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
    destination.bodies.0.insert(0, body.0.clone());
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
    use crate::game::world::level::{TravelAnswer, answer_travel_questions, ask_travel};
    use crate::presentation::dialog::QuestionDialog;

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

/// A new game's party is complete the moment its level is ready: within the
/// same update, a system after activation finds every member equipped, with
/// spell points, vitals and a seat.
#[test]
fn a_new_party_is_complete_when_its_first_level_is_ready() {
    use crate::game::inventory::Carried;
    use crate::game::magic::SpellPoints;
    #[derive(Resource, Default)]
    struct Seen(Vec<(bool, bool, bool, bool)>);
    let mut app = app();
    with_data(&mut app);
    app.init_resource::<Seen>().add_systems(
        Update,
        (|mut ready: MessageReader<LevelReady>,
          members: Query<(
            Entity,
            Option<&Carried>,
            Option<&SpellPoints>,
            Option<&crate::game::characters::Vitals>,
            &crate::game::party::roster::PartyMember,
        )>,
          formations: Query<&crate::game::party::formation::Formation>,
          mut seen: ResMut<Seen>| {
            if ready.read().next().is_some() {
                for (member, carried, points, vitals, party) in &members {
                    let seated =
                        formations.get(party.0).is_ok_and(|f| f.place_of(member).is_some());
                    seen.0.push((
                        carried.is_some_and(|c| c.stacks().next().is_some()),
                        points.is_some(),
                        vitals.is_some(),
                        seated,
                    ));
                }
            }
        })
        .after(activate_level),
    );
    spawn_level(&mut app, floor());
    app.update();
    assert_eq!(app.world().resource::<Seen>().0, [(true, true, true, true)]);
}

/// A starting item the catalog lacks refuses the new game's entry rather
/// than making an incomplete member.
#[test]
fn a_new_game_missing_a_starting_item_does_not_begin() {
    let mut app = app();
    with_data(&mut app);
    app.world_mut().resource_mut::<Catalogs>().items = ItemCatalog::default();
    let destination = spawn_level(&mut app, floor());
    app.update();
    assert!(failed(&app, destination));
    assert!(app.world_mut().query::<&Party>().iter(app.world()).next().is_none());
    assert!(
        app.world_mut()
            .query::<&crate::game::party::roster::PartyMember>()
            .iter(app.world())
            .next()
            .is_none()
    );
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

/// A solid prop whose collision cannot be built fails the destination
/// before the level being left or the party changes; one the party walks
/// through is built without it.
#[test]
fn a_solid_prop_without_collision_fails_entry_and_keeps_the_source() {
    use crate::content::level::{PropModel, PropModelPart};
    let (mut app, first_level) = playing();
    let party = party_entities(&mut app);
    let prop = |collidable| PropModel {
        name: "broken".into(),
        collidable,
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
    let enter = |app: &mut App, collidable: bool| {
        let level = test_level(app, floor());
        let mut definition = app
            .world()
            .resource::<Assets<LevelDefinition>>()
            .get(&level.definition)
            .unwrap()
            .clone();
        definition.props = vec![prop(collidable)];
        app.world_mut()
            .resource_mut::<Assets<LevelDefinition>>()
            .insert(&level.definition, definition)
            .unwrap();
        app.world_mut().spawn(Destination::new(level, default(), Entry::Travel)).id()
    };
    let destination = enter(&mut app, true);
    app.update();
    assert!(failed(&app, destination));
    assert!(is_played(&app, first_level));
    assert_eq!(party_entities(&mut app), party);
    let destination = enter(&mut app, false);
    app.update();
    assert!(is_played(&app, destination));
    assert_eq!(party_entities(&mut app), party);
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
    assert!(!is_played(&app, root) && !failed(&app, root));

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
fn direct_damage_settles_death_once_and_rejection_leaves_the_target_unchanged() {
    use crate::content::items::{Dice, ItemDefinition, ItemId};
    use crate::content::species::{Drop, Treasure};
    use crate::game::inventory::PartyInventory;
    use crate::game::world::damage::MonsterDamage;
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
    let level = spawn_level(&mut app, floor());
    app.update();
    let world = app.world_mut();
    let target = world.query_filtered::<Entity, With<Monster>>().single(world).unwrap();
    let hit = |world: &mut World| {
        world
            .run_system_once(
                move |mut damage: MonsterDamage,
                      mut purse: Single<&mut crate::game::inventory::PartyInventory>,
                      mut rng: ResMut<GameRng>| {
                    damage.apply(target, 10, (1, 0), &mut purse, &mut rng)
                },
            )
            .unwrap()
    };
    use crate::game::world::navigation::{Walker, WalkerMotion, links::Crossing};
    use avian3d::prelude::{LinearVelocity, Position};
    use bevy_landmass::{AgentTarget3d, UsingAnimationLink};
    use bevy_tnua::{TnuaController, TnuaMotor};
    let physical = Vec3::new(20.0, 3.0, 40.0);
    let mut controller = TnuaController::<WalkerMotion>::default();
    controller.basis.desired_motion = Vec3::X;
    let link = world.spawn_empty().id();
    world.entity_mut(target).insert((
        Position(physical),
        LinearVelocity(Vec3::new(2.0, -1.0, 3.0)),
        Walker { speed: 2.0, float_height: 1.0 },
        Crossing { link, to: Vec3::splat(100.0) },
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
    use crate::game::world::level::level_active;
    use bevy::ecs::system::RunSystemOnce;
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
    use crate::game::party::walk_in::{PathFlight, move_party};
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
        Vec3::new(9.0, 4.0, 2.0) - Vec3::Y * crate::game::party::body::EYE_ABOVE_CENTRE
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
    assert!(!app.world().contains_resource::<Assets<Mesh>>());
    assert!(!app.world().contains_resource::<Assets<StandardMaterial>>());
    assert!(app.world().resource::<Assets<LevelGeometry>>().is_empty());
}

#[test]
fn named_actor_memory_stock_and_world_role_survive_an_unloaded_map() {
    use crate::content::{
        items::{ItemDefinition, ItemId},
        npcs::{DialogueScript, FactCatalog, NpcCatalog, NpcDefinition, NpcId, Quote, StockRule},
    };
    use crate::game::npcs::{ActorId, Npc, NpcAssets, NpcBodies};
    let mut app = app();
    with_data(&mut app);
    {
        let mut catalogs = app.world_mut().resource_mut::<Catalogs>();
        catalogs.items.0[0] = ItemDefinition {
            id: ItemId(0),
            value: 100,
            class: crate::content::items::EquipClass::Potion,
            ..default()
        };
        catalogs.species.0[0].npc = Some(NpcId(0));
    }
    let catalog = NpcCatalog(vec![NpcDefinition {
        source_name: "Fixture".into(),
        merchant: true,
        disposition: 80,
        owns_stock: true,
        accepted_classes: crate::content::items::EquipClass::Potion.into(),
        stock: vec![StockRule { item: ItemId(0), quantity: 3, persistent: true }],
        ..default()
    }]);
    let facts = FactCatalog::default();
    let script = app
        .world_mut()
        .resource_mut::<Assets<DialogueScript>>()
        .add(DialogueScript { quotes: vec![Quote { text: vec!["Greeting".into()], ..default() }] });
    {
        let mut catalogs = app.world_mut().resource_mut::<Catalogs>();
        catalogs.npcs = catalog;
        catalogs.facts = facts;
    }
    app.insert_resource(NpcAssets { scripts: [(NpcId(0), script)].into() });
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
    use crate::game::npcs::conversation::{self, Conversation, ConversationRequest};
    let at = app.world().get::<Transform>(body).unwrap().translation;
    let party = party_entities(&mut app)[0];
    app.world_mut().get_mut::<avian3d::prelude::Position>(party).unwrap().0 = at + Vec3::X;
    for round in 0..2 {
        let session = conversation::begin(app.world_mut(), actor).unwrap();
        app.update();
        assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((0, 0)));
        // Retail lets a queued line play out after the party leaves.
        if let conversation::ConversationWait::Line { speech, .. } =
            app.world().get::<Conversation>(session).unwrap().wait
        {
            app.world_mut()
                .write_message(ConversationRequest::Advance { conversation: session, speech });
            app.update();
        }
        // Buying one and selling it back, through the merchant's definition
        // and the item catalog at once.
        if round == 0 {
            use crate::game::inventory::{ItemOwner, ItemRef, PartyInventory};
            use crate::game::npcs::trade::TradeSource;
            let purse = |app: &App| app.world().get::<PartyInventory>(party).unwrap().gold;
            app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 10_000;
            let lot = app.world().get::<Npc>(actor).unwrap().stock.lots[0].id;
            let quote =
                conversation::quote_trade(app.world_mut(), session, TradeSource::Stock(lot), 1)
                    .unwrap();
            // A confirmation buys once, however often it arrives.
            app.world_mut().write_message(ConversationRequest::Trade(quote));
            app.world_mut().write_message(ConversationRequest::Trade(quote));
            app.update();
            app.world_mut().write_message(ConversationRequest::Trade(quote));
            app.update();
            assert_eq!(purse(&app), 10_000 - quote.gold);
            assert_eq!(app.world().get::<Npc>(actor).unwrap().stock.lots[0].units, 1);
            let bought = app.world().get::<PartyInventory>(party).unwrap().pool.stacks[0].id;
            let source = TradeSource::Party(ItemRef { owner: ItemOwner::Pool, id: bought });
            let sale = conversation::quote_trade(app.world_mut(), session, source, 1).unwrap();
            app.world_mut().write_message(ConversationRequest::Trade(sale));
            app.world_mut().write_message(ConversationRequest::Trade(sale));
            app.update();
            assert_eq!(purse(&app), 10_000 - quote.gold + sale.gold);
            assert!(app.world().get::<PartyInventory>(party).unwrap().pool.stacks.is_empty());
        }
        app.world_mut().write_message(ConversationRequest::Close { conversation: session });
        app.update();
        assert!(app.world().get_entity(session).is_err());
    }
    let provoked = crate::game::world::hostility::SetStance { stance: Stance::Hostile, at: 7 };
    let group = app.world().get::<crate::game::world::identify::GroupMember>(body).unwrap().0;
    app.world_mut().get_mut::<crate::game::world::identify::MonsterGroup>(group).unwrap().set =
        Some(provoked);
    let stock = app.world().get::<Npc>(actor).unwrap().stock.clone();
    // Replacing the current map departs and reuses this role in the same
    // activation, before its queued Disabled insertion is applied.
    let replacement = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, replacement));
    assert!(app.world().get_entity(source).is_err());
    assert_eq!(app.world().get::<NpcBodies>(actor).unwrap()[0], body);
    assert_eq!(app.world().get::<LevelOwned>(body).unwrap().0, replacement);
    assert!(app.world().get::<bevy::ecs::entity_disabling::Disabled>(body).is_none());
    assert!(app.world().get::<Collider>(body).is_some());
    assert_eq!(app.world().get::<Monster>(body).unwrap().hp, 4);
    assert_eq!(app.world().get::<Npc>(actor).unwrap().stock, stock);
    let source = replacement;
    app.world_mut()
        .entity_mut(body)
        .insert(crate::game::world::navigation::links::Crossing { link: source, to: Vec3::X });
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
    assert!(app.world().get::<bevy::ecs::entity_disabling::Disabled>(body).is_some());
    assert_eq!(app.world().get::<NpcBodies>(actor).unwrap()[0], body);
    assert_eq!(app.world().get::<Monster>(body).unwrap().hp, 4);
    assert!(app.world().get::<Collider>(body).is_none());
    assert!(app.world().get::<avian3d::prelude::RigidBody>(body).is_none());
    assert!(
        app.world_mut().query::<(Entity, &Monster)>().iter(app.world()).all(|(e, _)| e != body)
    );
    app.update();
    assert_eq!(app.world().get::<Npc>(actor).unwrap().stock, stock);
    let back = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, back));
    let returned_body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    assert_eq!(returned_body, body);
    assert!(app.world().get::<bevy_landmass::UsingAnimationLink>(body).is_none());
    assert_eq!(app.world().get::<LevelOwned>(returned_body).unwrap().0, back);
    assert!(app.world().get::<Collider>(returned_body).is_some());
    assert!(app.world().get::<bevy::ecs::entity_disabling::Disabled>(returned_body).is_none());
    assert_eq!(app.world().get::<Monster>(returned_body).unwrap().hp, 4);
    let group =
        app.world().get::<crate::game::world::identify::GroupMember>(returned_body).unwrap().0;
    assert_eq!(
        app.world().get::<crate::game::world::identify::MonsterGroup>(group).unwrap().set,
        Some(provoked),
        "the NPC's group keeps the provocation"
    );
    let npc = app.world().get::<Npc>(actor).unwrap();
    assert_eq!(npc.stock, stock);
    assert_eq!(npc.paid_topics, [7]);
    assert!(!npc.greeting_pending);
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);
    assert_eq!(app.world().get::<ActorId>(actor), Some(&ActorId::Named(NpcId(0))));
}

/// A slain NPC gives up what it carries, not its species' treasure, pays
/// its purse once, and its world role never comes back.
#[test]
fn a_slain_npc_gives_up_what_it_carries_and_never_returns() {
    use crate::content::items::{Dice, ItemId, LootTable};
    use crate::content::npcs::{
        DialogueScript, FactCatalog, NpcCatalog, NpcDefinition, NpcId, Quote,
    };
    use crate::game::inventory::PartyInventory;
    use crate::game::npcs::{Npc, NpcAssets, NpcBodies};
    use crate::game::world::damage::MonsterDamage;
    use crate::game::world::items::{GroundLoot, WorldItem};
    use bevy::ecs::system::RunSystemOnce;
    let mut app = app();
    with_data(&mut app);
    let mut catalogs = app.world_mut().resource_mut::<Catalogs>();
    catalogs.species.0[0].npc = Some(NpcId(0));
    catalogs.loot.0 = vec![LootTable {
        name: "Pockets".into(),
        entries: vec![(ItemId(1), 0), (ItemId(2), 5), (ItemId(3), 5)],
        level_scaled: false,
        item_count: Dice { count: 0, sides: 0, base: 1 },
        gold: default(),
    }];
    let catalog = NpcCatalog(vec![NpcDefinition {
        source_name: "Fixture".into(),
        disposition: 80,
        item_table: Some(0),
        gold: 25,
        ..default()
    }]);
    let facts = FactCatalog::default();
    let script = app
        .world_mut()
        .resource_mut::<Assets<DialogueScript>>()
        .add(DialogueScript { quotes: vec![Quote { text: vec!["Greeting".into()], ..default() }] });
    {
        let mut catalogs = app.world_mut().resource_mut::<Catalogs>();
        catalogs.npcs = catalog;
        catalogs.facts = facts;
    }
    app.insert_resource(NpcAssets { scripts: [(NpcId(0), script)].into() });
    let source = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, source));
    let actor = app.world_mut().query::<(Entity, &Npc)>().single(app.world()).unwrap().0;
    let body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    let party =
        app.world_mut().query_filtered::<Entity, With<Party>>().single(app.world()).unwrap();
    let strike = move |damage: MonsterDamage,
                       mut purses: Query<&mut PartyInventory>,
                       mut rng: ResMut<GameRng>| {
        let mut damage = damage;
        damage.apply(body, 1000, (1, 0), &mut purses.get_mut(party).unwrap(), &mut rng).unwrap()
    };
    let killed = app.world_mut().run_system_once(strike).unwrap();
    assert!(killed.died);
    let npc = app.world().get::<Npc>(actor).unwrap();
    assert!(npc.dead);
    assert!(npc.possessions.items.is_empty());
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 25);
    let mut dropped: Vec<_> = app
        .world_mut()
        .query::<&WorldItem>()
        .iter(app.world())
        .flat_map(|lying| match &lying.loot {
            GroundLoot::Loose(item) => vec![item.item],
            GroundLoot::Pile { contents, .. } => contents.iter().map(|item| item.item).collect(),
        })
        .collect();
    dropped.sort();
    assert_eq!(dropped.len(), 2, "the sure item and one draw: {dropped:?}");
    assert_eq!(dropped[0], ItemId(1));
    // The corpse pays nothing more.
    let again = app.world_mut().run_system_once(strike).unwrap();
    assert!(!again.died);
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 25);
    // Away and back: the NPC stays dead and out of the world.
    let away = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, away));
    let back = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, back));
    assert_eq!(app.world().get::<NpcBodies>(actor).unwrap()[0], body);
    assert!(app.world().get::<bevy::ecs::entity_disabling::Disabled>(body).is_some());
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);
}

/// An NPC that joins brings its character and belongings and leaves its
/// body dormant; dismissed, it stands where it left and keeps what it wants;
/// it rejoins as itself; dying in the party, it leaves for good. Never a
/// second NPC, and never a body in the world while it travels with the party.
#[test]
fn a_recruited_npc_joins_leaves_returns_and_dies_as_one() {
    use crate::content::characters::{Attribute, Skill};
    use crate::content::items::ItemId;
    use crate::content::npcs::{
        DialogueScript, FactCatalog, NpcCatalog, NpcCharacter, NpcDefinition, NpcId, Quote,
    };
    use crate::game::characters::{Condition, Lasting, Vitals};
    use crate::game::inventory::{Carried, EquipSlot, PartyInventory};
    use crate::game::npcs::recruit::{self, JoinRefusal};
    use crate::game::npcs::{Npc, NpcAssets, NpcBodies, NpcWorldRole};
    use crate::game::party::roster::{PartyMember, PartyRoster};
    use crate::game::world::level::LevelOwned;
    use bevy::ecs::entity_disabling::Disabled;
    let mut app = app();
    with_data(&mut app);
    app.world_mut().resource_mut::<Catalogs>().species.0[0].npc = Some(NpcId(0));
    let mut equipment = [None; 12];
    equipment[6] = Some(ItemId(5));
    let mut backpack = [None; 8];
    backpack[0] = Some(ItemId(6));
    let template = NpcCharacter {
        name: "Fixture".into(),
        profession: Profession::Fighter,
        race: Race::Human,
        gender: Gender::Male,
        portrait: 0,
        level: 3,
        attributes: enum_map::EnumMap::from_fn(|_: Attribute| 60),
        skills: enum_map::EnumMap::from_fn(|s| if s == Skill::Sword { 20 } else { 0 }),
        spells: vec![],
        equipment,
        backpack,
        wanted: vec![ItemId(5)],
    };
    let catalog = NpcCatalog(vec![NpcDefinition {
        source_name: "Fixture".into(),
        disposition: 80,
        character: Some(template),
        ..default()
    }]);
    let facts = FactCatalog::default();
    let script = app
        .world_mut()
        .resource_mut::<Assets<DialogueScript>>()
        .add(DialogueScript { quotes: vec![Quote { text: vec!["Greeting".into()], ..default() }] });
    {
        let mut catalogs = app.world_mut().resource_mut::<Catalogs>();
        catalogs.npcs = catalog;
        catalogs.facts = facts;
    }
    app.insert_resource(NpcAssets { scripts: [(NpcId(0), script)].into() });
    let source = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, source));
    let actor = app.world_mut().query::<(Entity, &Npc)>().single(app.world()).unwrap().0;
    let body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    let party =
        app.world_mut().query_filtered::<Entity, With<Party>>().single(app.world()).unwrap();
    let afraid = (Condition::Afraid, Lasting::Rounds(3));
    app.world_mut().get_mut::<Monster>(body).unwrap().conditions.inflict(afraid.0, afraid.1);
    let members = |app: &App| app.world().get::<PartyRoster>(party).unwrap().len();

    // Joining.
    recruit::join(app.world_mut(), actor).unwrap();
    assert_eq!(members(&app), 2);
    assert_eq!(app.world().get::<PartyMember>(actor).unwrap().0, party);
    let character = app.world().get::<Character>(actor).unwrap();
    assert_eq!((character.level(), character.profession_levels[Profession::Fighter]), (3, 3));
    assert_eq!(character.experience, character.experience_previous_goal);
    assert_eq!(character.experience, 2000, "level three has passed both Fighter thresholds");
    let experience = character.experience;
    let carried = app.world().get::<Carried>(actor).unwrap();
    assert_eq!(carried.equipment[EquipSlot::Primary].as_ref().unwrap().stack.item, ItemId(5));
    assert_eq!(carried.backpack[0].as_ref().unwrap().item, ItemId(6));
    assert_eq!(app.world().get::<Vitals>(actor).unwrap().conditions, [afraid].into());
    assert!(app.world().get::<Disabled>(body).is_some());
    assert!(app.world().get::<LevelOwned>(body).is_none());
    assert_eq!(recruit::joinable(app.world_mut(), actor), Err(JoinRefusal::Declines));

    // Revisiting its map while it travels with the party leaves its body dormant.
    let mut away = test_level(&mut app, floor());
    away.path = "levels/away/away.pvl".into();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&away.definition)
        .unwrap()
        .monsters
        .clear();
    let away_path = away.path.clone();
    let away = app.world_mut().spawn(Destination::new(away, default(), Entry::Travel)).id();
    app.update();
    assert!(is_played(&app, away));
    let back = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, back));
    assert!(app.world().get::<Disabled>(body).is_some());
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);
    assert_eq!(members(&app), 2);

    // Leaving, on another map: it stands there, keeps what it wants and
    // gives the party the rest.
    let mut elsewhere = test_level(&mut app, floor());
    elsewhere.path = away_path.clone();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&elsewhere.definition)
        .unwrap()
        .monsters
        .clear();
    let elsewhere =
        app.world_mut().spawn(Destination::new(elsewhere, default(), Entry::Travel)).id();
    app.update();
    assert!(is_played(&app, elsewhere));
    assert!(recruit::dismiss(app.world_mut(), actor));
    assert_eq!(members(&app), 1);
    assert!(app.world().get::<PartyMember>(actor).is_none());
    assert!(app.world().get::<Disabled>(body).is_none());
    assert_eq!(app.world().get::<LevelOwned>(body).unwrap().0, elsewhere);
    assert_eq!(app.world().get::<NpcWorldRole>(body).unwrap().map, away_path);
    let monster = app.world().get::<Monster>(body).unwrap();
    assert!(monster.hp > 0);
    assert_eq!(monster.conditions, [afraid].into());
    let pool: Vec<_> = app
        .world()
        .get::<PartyInventory>(party)
        .unwrap()
        .pool
        .stacks
        .iter()
        .map(|stack| stack.item)
        .collect();
    assert!(pool.contains(&ItemId(6)) && !pool.contains(&ItemId(5)), "{pool:?}");
    let carried = app.world().get::<Carried>(actor).unwrap();
    assert!(carried.equipment[EquipSlot::Primary].is_some());
    assert!(carried.backpack[0].is_none());

    // Its authored map no longer places it; the map it left on keeps it.
    let home = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, home));
    assert!(app.world().get::<Disabled>(body).is_some());
    let mut again = test_level(&mut app, floor());
    again.path = away_path.clone();
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&again.definition)
        .unwrap()
        .monsters
        .clear();
    let again = app.world_mut().spawn(Destination::new(again, default(), Entry::Travel)).id();
    app.update();
    assert!(is_played(&app, again));
    assert!(app.world().get::<Disabled>(body).is_none());
    assert_eq!(app.world().get::<LevelOwned>(body).unwrap().0, again);
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);

    // Rejoining: the same character, not a fresh one.
    recruit::join(app.world_mut(), actor).unwrap();
    assert_eq!(members(&app), 2);
    assert_eq!(app.world().get::<Character>(actor).unwrap().experience, experience);
    assert!(app.world().get::<Carried>(actor).unwrap().backpack[0].is_none());

    // Death and accepted travel share a frame: settle before retiring the source level.
    app.world_mut().get_mut::<Vitals>(actor).unwrap().hp = 0;
    let destination = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, destination));
    assert_eq!(members(&app), 1);
    assert!(app.world().get::<Npc>(actor).unwrap().dead);
    let pool = &app.world().get::<PartyInventory>(party).unwrap().pool.stacks;
    assert!(pool.iter().any(|stack| stack.item == ItemId(5)));
    assert!(!recruit::dismiss(app.world_mut(), actor));
    assert!(app.world().get::<Disabled>(body).is_some());
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);
    // Neither its authored map nor the map its body last stood on raises it.
    let home = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, home));
    assert!(app.world().get::<Disabled>(body).is_some());
    let mut last = test_level(&mut app, floor());
    last.path = away_path;
    app.world_mut()
        .resource_mut::<Assets<LevelDefinition>>()
        .get_mut(&last.definition)
        .unwrap()
        .monsters
        .clear();
    let last = app.world_mut().spawn(Destination::new(last, default(), Entry::Travel)).id();
    app.update();
    assert!(is_played(&app, last));
    assert!(app.world().get::<Disabled>(body).is_some(), "the dead stay out of the world");
    assert_eq!(app.world_mut().query::<&Npc>().iter(app.world()).count(), 1);
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
