//! Scenarios on retail content: the real game, built headless as `main`
//! builds it, driven through gameplay requests (never simulated input) and
//! checked on its state. They need `RIZZARDRY_RETAIL` and fail without it.
//!
//! ```sh
//! RIZZARDRY_RETAIL=/path/to/wizardry8 cargo test --test scenarios -- --ignored
//! ```

use std::sync::Mutex;

use bevy::prelude::*;
use clap::Parser;
use rizzardry::app::{Options, build};
use rizzardry::game::combat::Battle;
use rizzardry::game::inventory::PartyInventory;
use rizzardry::game::world::interaction::WorldClick;
use rizzardry::game::world::items::WorldItem;
use rizzardry::game::world::level::{Departing, Level, Travel};
use rizzardry::game::world::population::Monster;

/// One game at a time: they share the navmesh cache and the asset
/// processor's output.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Plenty of fixed steps (each 1/64 s of game time) for anything here.
const STEPS: usize = 64 * 120;

/// The game with these command-line options, headless, ready to step.
fn game(args: &[&str]) -> App {
    let options = Options::try_parse_from(["rizzardry", "--headless"].iter().chain(args))
        .expect("set RIZZARDRY_RETAIL to a Wizardry 8 installation");
    let mut app = build(&options).expect("the game builds");
    app.finish();
    app.cleanup();
    app
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
    let mut levels = world.query_filtered::<&Level, Without<Departing>>();
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
    step_until(app, "the Monastery becoming active", |w| playing(w).is_some());
    step_until(app, "a fight on the beach", |w| w.contains_resource::<Battle>());
    step_until(app, "the fight's end", |w| !w.contains_resource::<Battle>());
}

#[test]
#[ignore = "requires a retail installation"]
fn the_party_beats_the_beach_crabs() {
    let _one = ONE_AT_A_TIME.lock();
    let mut app = game(&["--continuous", "--start=108,2.5,-96"]);
    crab_fight(&mut app);
    let (_, dead) = monsters(app.world_mut());
    assert_eq!(dead, 3, "the three crabs are dead");
}

#[test]
#[ignore = "requires a retail installation"]
fn a_green_slime_leaves_treasure() {
    let _one = ONE_AT_A_TIME.lock();
    // With this seed one of the three Green Slimes drops from its table.
    let mut app = game(&["--continuous", "--seed", "4", "--start=69.5,10,-24", "--yaw", "0"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    let before = lying_items(app.world_mut());
    step_until(&mut app, "a fight with the slimes", |w| w.contains_resource::<Battle>());
    step_until(&mut app, "the fight's end", |w| !w.contains_resource::<Battle>());
    assert_eq!(lying_items(app.world_mut()), before + 1);

    // Clicking the treasure, within reach, takes it into the hand.
    let world = app.world_mut();
    let loot = world.query::<(Entity, &Name)>().iter(world).find(|(_, n)| n.as_str() == "loot");
    let (loot, _) = loot.expect("the treasure lies in the world");
    let ray = Ray3d::new(Vec3::ZERO, Dir3::NEG_Z);
    world.write_message(WorldClick { ray, under: Some((loot, 1.0)) });
    step_until(&mut app, "taking the treasure", |w| w.resource::<PartyInventory>().held.is_some());
    assert_eq!(lying_items(app.world_mut()), before);
}

#[test]
#[ignore = "requires a retail installation"]
fn the_dead_stay_dead_across_travel() {
    let _one = ONE_AT_A_TIME.lock();
    let mut app = game(&["--continuous", "--start=108,2.5,-96"]);
    crab_fight(&mut app);
    let (count, dead) = monsters(app.world_mut());

    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    step_until(&mut app, "arriving in Monastery 2", |w| {
        playing(w).as_deref() == Some("levels/monastery/monastery2.pvl")
    });
    app.world_mut().write_message(Travel { entrance: "MO102".into() });
    step_until(&mut app, "returning to Monastery 1", |w| {
        playing(w).as_deref() == Some("levels/monastery/monastery1.pvl")
    });

    assert_eq!(monsters(app.world_mut()), (count - dead, 0), "the crabs did not come back");
}
