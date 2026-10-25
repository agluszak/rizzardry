//! Small headless scenarios at the level/party ownership boundary.

use avian3d::prelude::Collider;
use bevy::prelude::*;
use rizzardry::content::level::{CollisionMesh, LevelGeometry};
use rizzardry::game::party::body::{PartyBody, PartyMotionConfig, place_party_on_ready};
use rizzardry::game::party::teleports::InsideTriggers;
use rizzardry::game::world::level::{Level, LevelReady, build_level_geometry};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<LevelGeometry>>()
        .init_resource::<Assets<PartyMotionConfig>>()
        .init_resource::<InsideTriggers>()
        .add_message::<LevelReady>()
        .add_systems(
            Update,
            (build_level_geometry, place_party_on_ready.after(build_level_geometry)),
        );
    app.finish();
    app.cleanup();
    app
}

fn spawn_level(app: &mut App, collision: CollisionMesh) -> Entity {
    let geometry = app.world_mut().resource_mut::<Assets<LevelGeometry>>().add(LevelGeometry {
        parts: vec![],
        collision,
        map_layers: vec![],
        environment: None,
        lighting: default(),
        entrances: vec![],
        triggers: vec![],
        props: vec![],
        links: vec![],
        teleports: vec![],
        animated_textures: vec![],
    });
    app.world_mut()
        .spawn(Level { geometry, navmeshes: vec![], sky: None, script: default(), messages: None })
        .id()
}

fn party_entities(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<PartyBody>>();
    query.iter(world).collect()
}

fn floor() -> CollisionMesh {
    CollisionMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Z],
        triangles: vec![[0, 1, 2]],
        walkable: vec![true],
    }
}

#[test]
fn a_level_without_collision_cannot_activate_the_party() {
    let mut app = app();
    let root = spawn_level(&mut app, CollisionMesh::default());

    app.update();

    assert!(app.world().get::<Collider>(root).is_none());
    assert!(app.world().resource::<Messages<LevelReady>>().is_empty());
    assert!(party_entities(&mut app).is_empty());
}

#[test]
fn the_party_survives_level_replacement_without_duplication() {
    let mut app = app();
    let first_level = spawn_level(&mut app, floor());

    app.update();
    assert!(app.world().get::<Collider>(first_level).is_some());
    let party = party_entities(&mut app);
    assert_eq!(party.len(), 1);
    app.update();
    assert_eq!(party_entities(&mut app), party);

    app.world_mut().despawn(first_level);
    let second_level = spawn_level(&mut app, floor());
    app.update();

    assert!(app.world().get::<Collider>(second_level).is_some());
    assert_eq!(party_entities(&mut app), party);
}
