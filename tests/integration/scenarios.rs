//! Scenarios on retail content: the real game, built headless as `main`
//! builds it, driven through gameplay requests (never simulated input) and
//! checked on its state. They need `RIZZARDRY_RETAIL` and fail without it.
//!
//! ```sh
//! RIZZARDRY_RETAIL=/path/to/wizardry8 cargo nextest run --run-ignored only -E 'test(/^scenarios::/)'
//! ```

use bevy::prelude::*;
use clap::Parser;
use rizzardry::app::{Options, build};
use rizzardry::game::combat::Battle;
use rizzardry::game::inventory::PartyInventory;
use rizzardry::game::world::items::WorldItem;
use rizzardry::game::world::level::{Level, Travel};
use rizzardry::game::world::population::Monster;

/// Plenty of fixed steps (each 1/64 s of game time) for anything here.
const STEPS: usize = 64 * 120;

/// The game with these command-line options, headless, ready to step.
fn game(args: &[&str]) -> App {
    let options = Options::try_parse_from(["rizzardry", "--headless"].iter().chain(args))
        .expect("set RIZZARDRY_RETAIL to a Wizardry 8 installation");
    let mut app = build(&options).expect("the game builds");
    app.finish();
    app.cleanup();
    assert!(app.get_sub_app(bevy::render::RenderApp).is_none());
    assert!(!app.world().contains_resource::<Assets<Mesh>>());
    assert!(!app.world().contains_resource::<Assets<Image>>());
    assert!(!app.world().contains_resource::<Assets<StandardMaterial>>());
    wait_for_level(&mut app, "levels/monastery/monastery1.pvl");
    app
}

/// Cold asset processing is wall-clock work, not a number of simulation ticks.
fn wait_for_level(app: &mut App, path: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while playing(app.world_mut()).as_deref() != Some(path) {
        app.update();
        assert!(std::time::Instant::now() < deadline, "loading {path}");
        std::thread::yield_now();
    }
}

/// Step the game until `done`, failing after [`STEPS`].
fn step_until(app: &mut App, what: &str, done: impl Fn(&mut World) -> bool) {
    for _ in 0..STEPS {
        app.update();
        if done(app.world_mut()) {
            return;
        }
    }
    panic!("{what} did not happen within {STEPS} steps");
}

/// The level being played, by path.
fn playing(world: &mut World) -> Option<String> {
    let mut levels = world.query::<&Level>();
    levels.iter(world).next().map(|level| level.path.clone())
}

fn monsters(world: &mut World) -> (usize, usize) {
    let mut query = world.query::<&Monster>();
    let all: Vec<&Monster> = query.iter(world).collect();
    (all.len(), all.iter().filter(|m| m.is_dead()).count())
}

fn lying_items(world: &mut World) -> usize {
    world.query::<&WorldItem>().iter(world).count()
}

/// Fight the beach crabs by the Monastery's door to the end.
fn crab_fight(app: &mut App) {
    step_until(app, "a fight on the beach", |w| w.contains_resource::<Battle>());
    step_until(app, "the fight's end", |w| !w.contains_resource::<Battle>());
}

#[test]
#[ignore = "requires a retail installation"]
fn a_green_slime_leaves_treasure() {
    // With this seed one of the three Green Slimes drops from its table.
    let mut app = game(&["--continuous", "--seed", "4", "--start=69.5,10,-24", "--yaw", "0"]);
    let before = lying_items(app.world_mut());
    step_until(&mut app, "a fight with the slimes", |w| w.contains_resource::<Battle>());
    step_until(&mut app, "the fight's end", |w| !w.contains_resource::<Battle>());
    assert_eq!(lying_items(app.world_mut()), before + 1);

    // Taking the resolved treasure, within reach, uses the same gameplay
    // operation as the rendered input adapter.
    let world = app.world_mut();
    let loot = world.query::<(Entity, &Name)>().iter(world).find(|(_, n)| n.as_str() == "loot");
    let (loot, _) = loot.expect("the treasure lies in the world");
    rizzardry::game::world::items::take_item(world, loot, 1.0).unwrap();
    step_until(&mut app, "taking the treasure", |w| w.resource::<PartyInventory>().held.is_some());
    assert_eq!(lying_items(app.world_mut()), before);
}

#[test]
#[ignore = "requires a retail installation"]
fn the_dead_stay_dead_across_travel() {
    let mut app = game(&["--continuous", "--start=108,2.5,-96"]);
    crab_fight(&mut app);
    let (count, dead) = monsters(app.world_mut());
    assert_eq!(dead, 3, "the party beat the three beach crabs");

    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    wait_for_level(&mut app, "levels/monastery/monastery2.pvl");
    app.world_mut().write_message(Travel { entrance: "MO102".into() });
    wait_for_level(&mut app, "levels/monastery/monastery1.pvl");

    assert_eq!(monsters(app.world_mut()), (count - dead, 0), "the crabs did not come back");
}

#[test]
#[ignore = "requires a retail installation"]
fn the_party_rests_eight_hours_and_wakes() {
    use avian3d::prelude::LinearVelocity;
    use rizzardry::game::camp::{Camp, CampNews, CampRequest};
    use rizzardry::game::party::body::PartyBody;
    use rizzardry::game::time::GameClock;
    // After beating the beach crabs, with the beach quiet.
    let mut app = game(&["--continuous", "--start=108,2.5,-96"]);
    crab_fight(&mut app);
    // The party may not camp while moving.
    step_until(&mut app, "the party standing still", |w| {
        let mut party = w.query_filtered::<&LinearVelocity, With<PartyBody>>();
        party.iter(w).all(|v| v.length() < 0.05)
    });
    let before = app.world().resource::<GameClock>().ms;
    app.world_mut().write_message(CampRequest);
    app.update();
    let news: Vec<_> = app.world_mut().resource_mut::<Messages<CampNews>>().drain().collect();
    assert!(app.world().contains_resource::<Camp>(), "not camping: {news:?}");
    step_until(&mut app, "waking", |w| !w.contains_resource::<Camp>());
    let rested = app.world().resource::<GameClock>().ms - before;
    assert!(rested >= 8 * 3_600_000, "rested {} game hours", rested / 3_600_000);
}
