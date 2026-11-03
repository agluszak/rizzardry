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

fn fighting(world: &World) -> bool {
    world.try_query::<&Battle>().is_some_and(|mut q| q.iter(world).next().is_some())
}

/// Fight the beach crabs by the Monastery's door to the end.
fn crab_fight(app: &mut App) {
    step_until(app, "the Monastery becoming active", |w| playing(w).is_some());
    step_until(app, "a fight on the beach", |w| fighting(w));
    step_until(app, "the fight's end", |w| !fighting(w));
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
    use crate::game::camp::{Camp, CampNews, CampRequest};
    use crate::game::party::body::PartyBody;
    use crate::game::time::GameClock;
    use avian3d::prelude::LinearVelocity;
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

#[test]
#[ignore = "requires a retail installation"]
fn burz_answers_paid_topics_and_settles_trades() {
    use crate::game::inventory::PartyInventory;
    use crate::game::npcs::{
        ActorId, NpcBodies,
        conversation::*,
        trade::{MerchantStock, TradeSource},
    };
    let mut app = game(&["--seed", "8", "--start=225,36.7,-68.6"]);
    let content = app.world().resource::<crate::game::data::Catalogs>();
    let id = content.npcs.0.iter().position(|npc| npc.source_name == "Burz").unwrap() as u16;
    let actor = app
        .world_mut()
        .query::<(Entity, &ActorId)>()
        .iter(app.world())
        .find(|(_, a)| a.npc().0 == id)
        .unwrap()
        .0;
    let body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    let at = app.world().get::<avian3d::prelude::Position>(body).unwrap().0;
    let party = crate::game::party::roster::the_party(app.world_mut());
    app.world_mut().get_mut::<avian3d::prelude::Position>(party).unwrap().0 = at + Vec3::X * 2.0;
    app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 10_000;
    app.world_mut().write_message(BeginConversation { actor });
    app.update();
    let session =
        app.world_mut().query_filtered::<Entity, With<Conversation>>().single(app.world()).unwrap();
    fn finish(app: &mut App, session: Entity) {
        for _ in 0..50 {
            app.update();
            match &app.world().get::<Conversation>(session).unwrap().wait {
                ConversationWait::Line { speech } => {
                    let speech = *speech;
                    app.world_mut().write_message(LineSpoken { conversation: session, speech });
                }
                ConversationWait::Blocked { reason, .. } => panic!("{reason}"),
                _ => return,
            }
        }
        panic!("Burz did not finish speaking");
    }
    finish(&mut app, session);
    app.world_mut()
        .write_message(ConversationRequest::new(session, Talk::Topic { text: "MONASTERY".into() }));
    finish(&mut app, session);
    let ConversationWait::Price { gold: price, .. } =
        app.world().get::<Conversation>(session).unwrap().wait
    else {
        panic!("Burz asks a price")
    };
    app.world_mut()
        .write_message(ConversationRequest::new(session, Talk::PriceAnswer { accept: true }));
    finish(&mut app, session);
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 10_000 - price);
    assert!(matches!(
        app.world().get::<Conversation>(session).unwrap().wait,
        ConversationWait::Options { .. }
    ));
    app.world_mut()
        .write_message(ConversationRequest::new(session, Talk::Reply { text: "NO".into() }));
    finish(&mut app, session);
    assert!(app.world().get::<Conversation>(session).unwrap().ready());
    let lot = app
        .world()
        .get::<MerchantStock>(actor)
        .unwrap()
        .lots
        .iter()
        .find(|l| l.units > 0)
        .unwrap()
        .id;
    let quote = app
        .world_mut()
        .run_system_cached_with(quote_trade, (session, TradeSource::Stock(lot), 1))
        .unwrap()
        .unwrap();
    let gold = app.world().get::<PartyInventory>(party).unwrap().gold;
    app.world_mut().write_message(ConversationRequest::trade(quote));
    app.world_mut().write_message(ConversationRequest::trade(quote));
    app.update();
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, gold - quote.gold);
    assert!(!app.world().get::<PartyInventory>(party).unwrap().pool.stacks.is_empty());
}
