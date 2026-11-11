//! Scenarios on retail content: the real game, built headless as `main`
//! builds it, driven through gameplay requests (never simulated input) and
//! checked on its state. They need `RIZZARDRY_RETAIL` and fail without it.
//!
//! ```sh
//! RIZZARDRY_RETAIL=/path/to/wizardry8 cargo nextest run --run-ignored only -E 'test(/^scenarios::/)'
//! ```

use crate::app::{Options, build};
use crate::game::combat::Battle;
use crate::game::world::level::{Level, Travel};
use crate::game::world::population::Monster;
use bevy::prelude::*;
use clap::Parser;

/// Ten simulated minutes includes full movement budgets in multi-round fights.
const STEPS: usize = 64 * 600;

/// The game with these command-line options, headless, ready to step.
fn game(args: &[&str]) -> App {
    let options = Options::try_parse_from(["rizzardry", "--headless"].iter().chain(args))
        .expect("set RIZZARDRY_RETAIL to a Wizardry 8 installation");
    let mut app = build(&options).expect("the game builds");
    app.finish();
    app.cleanup();
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
    let mut query = world.query_filtered::<&crate::game::characters::Vitals, With<Monster>>();
    let all: Vec<&crate::game::characters::Vitals> = query.iter(world).collect();
    (all.len(), all.iter().filter(|m| m.is_dead()).count())
}

/// Where the level's monsters are: the dead ones, or the living.
fn lying(world: &mut World, dead: bool) -> Vec<Vec3> {
    let mut query = world.query_filtered::<
        (&crate::game::characters::Vitals, &avian3d::prelude::Position),
        With<Monster>,
    >();
    query.iter(world).filter(|(vitals, _)| vitals.is_dead() == dead).map(|(_, at)| at.0).collect()
}

fn fighting(world: &World) -> bool {
    world.try_query::<&Battle>().is_some_and(|mut q| q.iter(world).next().is_some())
}

/// Fight the beach crabs by the Monastery's door to the end.
fn crab_fight(app: &mut App) {
    step_until(app, "the Monastery becoming active", |w| playing(w).is_some());
    step_until(app, "a fight on the beach", |w| fighting(w));
    step_until(app, "the fight's end", |w| !fighting(w));
}

/// One play-through, since every scenario begins with the same fight: the
/// party beats the beach crabs, rests eight hours on the quiet beach, and
/// finds the crabs still dead after leaving the level and coming back.
#[test]
#[ignore = "requires a retail installation"]
fn the_party_fights_rests_and_finds_the_dead_still_dead() {
    use crate::game::camp::{Camp, CampNews, CampRequest};
    use crate::game::party::body::PartyBody;
    use crate::game::time::GameClock;
    use avian3d::prelude::LinearVelocity;
    let mut app = game(&["--continuous", "--start=108,2.5,-96"]);
    crab_fight(&mut app);
    let (_, dead) = monsters(app.world_mut());
    assert_eq!(dead, 3, "the party beat the three beach crabs");
    let fell = lying(app.world_mut(), true);

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

    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    wait_for_level(&mut app, "levels/monastery/monastery2.pvl");
    app.world_mut().write_message(Travel { entrance: "MO102".into() });
    wait_for_level(&mut app, "levels/monastery/monastery1.pvl");
    // Wandering encounters come and go while the party rests; the crabs do
    // not come back to where they fell.
    assert_eq!(monsters(app.world_mut()).1, 0, "the dead were cleared away");
    let stand = lying(app.world_mut(), false);
    let back = stand.iter().any(|alive| fell.iter().any(|dead| dead.distance(*alive) < 10.0));
    assert!(!back, "the crabs did not come back");
}

/// A device of the level being played, by its name.
fn device(world: &mut World, name: &str) -> Entity {
    use crate::content::level::LevelDefinition;
    use crate::game::world::triggers::Device;
    let handle = world.query::<&Level>().iter(world).next().unwrap().definition.clone();
    let definitions = world.resource::<Assets<LevelDefinition>>();
    let mut named = definitions.get(&handle).unwrap().triggers.iter();
    let index = named.rposition(|trigger| trigger.name.eq_ignore_ascii_case(name)).unwrap();
    let mut devices = world.query::<(Entity, &Device)>();
    devices.iter(world).find(|(_, device)| device.definition == index).unwrap().0
}

/// How many of a species are in the level.
fn of_species(world: &mut World, species: u32) -> usize {
    world.query::<&Monster>().iter(world).filter(|monster| monster.species == species).count()
}

/// The story has its say in a level's devices: the lid of a coffin in the
/// lower monastery lets out what is in it, once, and the wheel does not
/// turn without its key.
#[test]
#[ignore = "requires a retail installation"]
fn a_coffin_lid_lets_the_head_out_once() {
    use crate::game::world::triggers::activate;
    const HEAD: u32 = 0x12d;
    let mut app = game(&["--continuous"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    app.update();
    let lid = device(app.world_mut(), "Coffinlide");
    assert_eq!(of_species(app.world_mut(), HEAD), 0);
    activate(app.world_mut(), lid, false);
    step_until(&mut app, "the head coming out", |w| of_species(w, HEAD) == 1);
    activate(app.world_mut(), lid, false);
    for _ in 0..256 {
        app.update();
    }
    assert_eq!(of_species(app.world_mut(), HEAD), 1, "it is let out only once");

    // And a device the story stops does nothing: the wheel wants its key.
    let wheel = device(app.world_mut(), "wheel_star");
    assert!(!activate(app.world_mut(), wheel, false), "the wheel turned without the star key");
}

/// A walk over the navmesh is found between the party and a monster of the
/// monastery, and is no shorter than the straight line.
#[test]
#[ignore = "requires a retail installation"]
fn a_monster_has_a_walk_to_the_party() {
    use crate::game::party::body::PartyBody;
    use crate::game::world::navigation::{LevelNavigation, walking_distance};
    use avian3d::prelude::Position;
    let mut app = game(&["--continuous", "--start=108,2.5,-96"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    for _ in 0..64 {
        app.update();
    }
    let world = app.world_mut();
    let party = *world.query_filtered::<&Position, With<PartyBody>>().single(world).unwrap();
    let party = PartyBody::feet_below(PartyBody::eye(&party));
    let navigation = world.query::<&LevelNavigation>().single(world).unwrap().archipelagos.clone();
    let monsters: Vec<(f32, Vec3)> = world
        .query::<(&Monster, &Position)>()
        .iter(world)
        .map(|(monster, at)| (monster.body.radius, at.0))
        .collect();
    let mut archipelagos = world.query::<&bevy_landmass::Archipelago3d>();
    let walks: Vec<(f32, f32)> = monsters
        .iter()
        .filter_map(|&(radius, at)| {
            let class = crate::game::world::navigation::bake::size_class(radius);
            let archipelago = archipelagos.get(world, navigation[class]).ok()?;
            let walked = walking_distance(archipelago, at, party)?;
            Some((walked, at.distance(party)))
        })
        .collect();
    assert!(!walks.is_empty(), "no monster of the monastery has a walk to the party");
    for (walked, straight) in walks {
        assert!(
            walked + 0.5 >= straight * 0.9,
            "a walk of {walked} m is shorter than {straight} m"
        );
    }
}

/// Marten's Bluff's arrow trap shoots along each launcher's facing: a party
/// stood in front of one is struck.
#[test]
#[ignore = "requires a retail installation"]
fn the_arrow_trap_strikes_a_party_in_its_way() {
    use crate::content::level::LevelDefinition;
    use crate::game::characters::Vitals;
    use crate::game::party::body::PartyBody;
    use crate::game::party::roster::PartyRoster;
    use crate::game::world::triggers::activate;
    use avian3d::prelude::Position;
    let mut app = game(&["--continuous"]);
    app.world_mut().write_message(Travel { entrance: "MR201".into() });
    wait_for_level(&mut app, "levels/marten/marten2.pvl");
    for _ in 0..64 {
        app.update();
    }
    let world = app.world_mut();
    let handle = world.query::<&Level>().iter(world).next().unwrap().definition.clone();
    let definition = world.resource::<Assets<LevelDefinition>>().get(&handle).unwrap();
    let (start, aim) =
        (definition.spot("Arrowlauncher1").unwrap(), definition.aim("Arrowlauncher1").unwrap());
    let eye = start + *aim * 6.0;
    let mut party = world.query::<(&mut PartyBody, &mut Position, &mut Transform)>();
    let (mut body, mut position, mut transform) = party.single_mut(world).unwrap();
    body.stand(&mut position, &mut transform, eye, 0.0);
    let health = |world: &mut World| -> u32 {
        let roster: Vec<Entity> =
            world.query::<&PartyRoster>().single(world).unwrap().iter().collect();
        roster.iter().filter_map(|&m| world.get::<Vitals>(m)).map(|v| v.health.current).sum()
    };
    let before = health(app.world_mut());
    let trap = device(app.world_mut(), "Arrowtraptrigger");
    for _ in 0..8 {
        activate(app.world_mut(), trap, false);
        app.update();
    }
    assert!(health(app.world_mut()) < before, "the party stood in the arrows' way was struck");
}

/// Arnika's first lift, called, goes through its turn one part after the
/// other: the first call opens its gate above, the next shuts it, sends the
/// lift down and opens the gate below.
#[test]
#[ignore = "requires a retail installation"]
fn an_arnika_lift_goes_through_its_turn() {
    use crate::game::world::triggers::{LevelState, activate};
    let mut app = game(&["--continuous"]);
    app.world_mut().write_message(Travel { entrance: "ARN01".into() });
    wait_for_level(&mut app, "levels/arnika/arnika2.pvl");
    for _ in 0..64 {
        app.update();
    }
    let state = |world: &mut World| {
        let mut levels = world.query::<&LevelState>();
        levels.iter(world).next().map_or(0, |state| state.value(&"El01State".into()))
    };
    assert_eq!(state(app.world_mut()), 1, "the lift waits above, its gate shut");
    let call = device(app.world_mut(), "El1-TopButtons");
    activate(app.world_mut(), call, false);
    step_until(&mut app, "the gate above opening", |w| state(w) == 3);
    // The button is pressed again once it has come back up.
    step_until(&mut app, "the button coming back", |w| {
        w.get::<crate::game::world::props::PropMotion>(call).is_none_or(|m| m.frame == m.target)
    });
    activate(app.world_mut(), call, false);
    step_until(&mut app, "the lift down with the gate below open", |w| state(w) == 7);
}

/// The crusher under Marten's Bluff closes its plates on what stands between
/// them: a monster there is pushed clear or dies, and the crusher is still
/// again once they have stopped.
#[test]
#[ignore = "requires a retail installation"]
fn the_crusher_closes_on_what_stands_between_its_plates() {
    use crate::game::characters::Vitals;
    use crate::game::world::triggers::{LevelState, activate};
    use avian3d::prelude::{ColliderAabb, Position};
    let mut app = game(&["--continuous"]);
    app.world_mut().write_message(Travel { entrance: "MR201".into() });
    wait_for_level(&mut app, "levels/marten/marten2.pvl");
    for _ in 0..64 {
        app.update();
    }
    let crusher = |world: &mut World| {
        let mut levels = world.query::<&LevelState>();
        levels.iter(world).next().map_or(0, |state| state.value(&"MonsterCrusher".into()))
    };
    let bounds = |world: &mut World, device: Entity| {
        let children: Vec<Entity> = world.get::<Children>(device).unwrap().iter().collect();
        children
            .iter()
            .filter_map(|&part| world.get::<ColliderAabb>(part).map(|b| (b.min, b.max)))
            .reduce(|(lo, hi), (min, max)| (lo.min(min), hi.max(max)))
            .unwrap()
    };
    let (low, high) =
        (device(app.world_mut(), "Squisher-3"), device(app.world_mut(), "Squisher-4"));
    let (lower, upper) = (bounds(app.world_mut(), low), bounds(app.world_mut(), high));
    // A monster of the level stood between the plates.
    let between = Vec3::new(
        (lower.1.x + upper.0.x) / 2.0,
        (lower.0.y.max(upper.0.y) + lower.1.y.min(upper.1.y)) / 2.0,
        (lower.0.z.max(upper.0.z) + lower.1.z.min(upper.1.z)) / 2.0,
    );
    let world = app.world_mut();
    let victim = world.query_filtered::<Entity, With<Monster>>().iter(world).next().unwrap();
    world.get_mut::<Position>(victim).unwrap().0 = between;
    let controls = device(app.world_mut(), "SquisherControls");
    activate(app.world_mut(), controls, false);
    step_until(&mut app, "the crusher closing", |w| crusher(w) == 1);
    step_until(&mut app, "the plates stopping", |w| crusher(w) == 0);
    let world = app.world_mut();
    let at = world.get::<Position>(victim).map(|p| p.0);
    let dead = world.get::<Vitals>(victim).is_none_or(|v| v.is_dead());
    let pushed = at.is_some_and(|at| at.x != between.x);
    assert!(dead || pushed, "the monster between the plates was neither crushed nor pushed");
}

/// The game ends as the story says: once the party has joined the Savant,
/// the congratulation that closes the game ends it with his ending, and the
/// party can do nothing more.
#[test]
#[ignore = "requires a retail installation"]
fn the_game_ends_with_the_ending_the_story_chose() {
    use crate::game::data::Catalogs;
    use crate::game::npcs::scenes::{Ended, Ending};
    use crate::game::npcs::story::{FactSet, StoryFacts};
    let mut app = game(&["--continuous"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    let world = app.world_mut();
    let id = |world: &World, symbol: &str| {
        let facts = &world.resource::<Catalogs>().facts;
        facts.0.iter().find(|fact| fact.symbol == symbol).unwrap().id
    };
    let (joined, congratulated) =
        (id(world, "FACT_ENDGAME_JOIN_SAVANT"), id(world, "FACT_ENDGAME_QUE_CONGRAT_TWO"));
    world.resource_scope(|world, mut story: Mut<StoryFacts>| {
        let catalog = &world.resource::<Catalogs>().facts;
        story.set(joined, 1, catalog, "");
        story.set(congratulated, 1, catalog, "");
    });
    world.trigger(FactSet { fact: congratulated, value: 1 });
    step_until(&mut app, "the game ending", |w| w.contains_resource::<Ended>());
    assert_eq!(*app.world().resource::<Ended>(), Ended(Ending::Savant));
    assert!(!crate::game::freedom::free_in(app.world_mut()), "the party can do nothing more");
}
