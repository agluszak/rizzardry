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
    step_until(app, "the Monastery becoming active", |w| playing(w).is_some());
    step_until(app, "a fight on the beach", |w| w.contains_resource::<Battle>());
    step_until(app, "the fight's end", |w| !w.contains_resource::<Battle>());
}

#[test]
#[ignore = "requires a retail installation"]
fn a_green_slime_leaves_treasure() {
    // With this seed one of the three Green Slimes drops from its table.
    let mut app = game(&["--continuous", "--seed", "5", "--start=69.5,10,-24", "--yaw", "0"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
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
    step_until(&mut app, "taking the treasure", |w| {
        w.resource::<PartyInventory>().hand.held.is_some()
    });
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
    step_until(&mut app, "arriving in Monastery 2", |w| {
        playing(w).as_deref() == Some("levels/monastery/monastery2.pvl")
    });
    app.world_mut().write_message(Travel { entrance: "MO102".into() });
    step_until(&mut app, "returning to Monastery 1", |w| {
        playing(w).as_deref() == Some("levels/monastery/monastery1.pvl")
    });

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

#[test]
#[ignore = "requires a retail installation"]
fn burz_conversation_trade_and_revisit_keep_one_actor() {
    use rizzardry::content::npcs::{DialogueScript, NpcCatalog};
    use rizzardry::game::npcs::conversation::{
        self, Conversation, ConversationRequest, ConversationWait,
    };
    use rizzardry::game::npcs::trade::TradeSource;
    use rizzardry::game::npcs::{Npc, NpcAssets};
    let mut app = game(&["--seed", "8", "--start=225,36.7,-68.6"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    let assets = app.world().resource::<NpcAssets>();
    let catalog = app.world().resource::<Assets<NpcCatalog>>().get(&assets.catalog).unwrap();
    let burz_id = rizzardry::content::npcs::NpcId(
        catalog.0.iter().position(|d| d.source_name == "Burz").unwrap() as u16,
    );
    let actor = app
        .world_mut()
        .query::<(Entity, &Npc)>()
        .iter(app.world())
        .find(|(_, n)| n.definition == burz_id)
        .unwrap()
        .0;
    assert!(
        app.world().get::<ChildOf>(actor).is_none(),
        "map lifetime does not own the named actor"
    );
    assert!(app.world().get::<rizzardry::game::npcs::NpcPresence>(actor).is_some());
    let position = app.world().get::<avian3d::prelude::Position>(actor).unwrap().0;
    for mut p in app.world_mut().query_filtered::<&mut avian3d::prelude::Position,With<rizzardry::game::party::body::PartyBody>>().iter_mut(app.world_mut()){p.0=position+Vec3::X*2.0;}
    app.world_mut().resource_mut::<PartyInventory>().gold = 10000;
    let session = conversation::begin(app.world_mut(), actor).unwrap();
    fn finish(app: &mut App, session: Entity) {
        for _ in 0..40 {
            app.update();
            let c = app.world().get::<Conversation>(session).expect("conversation remains open");
            match c.wait {
                ConversationWait::Line { speech, .. } => {
                    app.world_mut().write_message(ConversationRequest::Advance {
                        conversation: session,
                        speech,
                    });
                }
                ConversationWait::Blocked { ref reason, .. } => {
                    panic!("unexpected content gap: {reason}")
                }
                _ => return,
            }
        }
        panic!("conversation did not reach input");
    }
    finish(&mut app, session);
    assert_eq!(app.world().get::<Conversation>(session).unwrap().current, Some((121, 0)));
    let script = app.world().get::<Conversation>(session).unwrap().script.clone();
    let topic = app
        .world()
        .resource::<Assets<DialogueScript>>()
        .get(&script)
        .unwrap()
        .topic("MONASTERY")
        .unwrap();
    let revision = app.world().get::<Conversation>(session).unwrap().revision;
    app.world_mut().write_message(ConversationRequest::Topic {
        conversation: session,
        revision,
        text: "MONASTERY".into(),
    });
    finish(&mut app, session);
    let c = app.world().get::<Conversation>(session).unwrap();
    assert!(matches!(c.wait, ConversationWait::Price { .. }));
    let revision = c.revision;
    app.world_mut().write_message(ConversationRequest::PriceAnswer {
        conversation: session,
        revision,
        accept: true,
    });
    finish(&mut app, session);
    let c = app.world().get::<Conversation>(session).unwrap();
    assert!(matches!(c.wait, ConversationWait::Options { .. }));
    let revision = c.revision;
    app.world_mut().write_message(ConversationRequest::Reply {
        conversation: session,
        revision,
        text: "NO".into(),
    });
    finish(&mut app, session);
    assert!(app.world().get::<Conversation>(session).unwrap().ready());
    assert!(app.world().get::<Npc>(actor).unwrap().paid_topics.contains(&topic));
    let lot =
        app.world().get::<Npc>(actor).unwrap().stock.lots.iter().find(|l| l.units > 0).unwrap().id;
    let quote =
        conversation::quote_trade(app.world_mut(), session, TradeSource::Stock(lot), 1).unwrap();
    let gold = app.world().resource::<PartyInventory>().gold;
    app.world_mut().write_message(ConversationRequest::Trade(quote));
    app.world_mut().write_message(ConversationRequest::Trade(quote));
    app.update();
    assert_eq!(
        app.world().resource::<PartyInventory>().gold,
        gold - quote.gold,
        "duplicate confirmation settles once"
    );
    let bought = app.world().resource::<PartyInventory>().pool.stacks[0].id;
    let sale =
        conversation::quote_trade(app.world_mut(), session, TradeSource::Pool(bought), 1).unwrap();
    app.world_mut().write_message(ConversationRequest::Trade(sale));
    app.update();
    assert_eq!(app.world().resource::<PartyInventory>().gold, gold - quote.gold + sale.gold);
    let current = app.world().get::<Conversation>(session).unwrap().current.unwrap().0;
    let fact = app.world().resource::<Assets<DialogueScript>>().get(&script).unwrap().quotes
        [current as usize]
        .operations
        .iter()
        .find_map(|op| match op {
            rizzardry::content::npcs::DialogueOp::SetFact(id, _) => Some(*id),
            _ => None,
        })
        .unwrap();
    let stock = app.world().get::<Npc>(actor).unwrap().stock.clone();
    let invalid = {
        let server = app.world().resource::<AssetServer>();
        rizzardry::game::world::level::Level::load("levels/missing/missing.pvl", server)
    };
    let invalid = app
        .world_mut()
        .spawn(rizzardry::game::world::level::Destination::new(invalid, default()))
        .id();
    step_until(&mut app, "failed preparation", |w| {
        w.get::<rizzardry::game::world::level::Destination>(invalid).is_some_and(|d| d.failed)
    });
    assert!(app.world().get::<Conversation>(session).is_some());

    app.world_mut().write_message(ConversationRequest::Close { conversation: session });
    app.update();
    assert!(app.world().get_entity(session).is_err());
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    step_until(&mut app, "leaving Burz's level", |w| {
        playing(w).is_some_and(|p| p.ends_with("monastery2.pvl"))
    });
    assert!(app.world().get::<Monster>(actor).is_none());
    assert!(app.world().get::<Npc>(actor).unwrap().dormant.is_some());
    let stock_offmap = app.world().get::<Npc>(actor).unwrap().stock.clone();
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(app.world().get::<Npc>(actor).unwrap().stock, stock_offmap);
    app.world_mut().write_message(Travel { entrance: "MO105".into() });
    step_until(&mut app, "returning to Burz", |w| {
        playing(w).is_some_and(|p| p.ends_with("monastery1.pvl"))
    });
    assert!(app.world().get::<Monster>(actor).is_some());
    let npc = app.world().get::<Npc>(actor).unwrap();
    assert_eq!(npc.stock, stock);
    assert!(npc.paid_topics.contains(&topic));
    assert!(!npc.greeting_pending);
    let facts = app
        .world()
        .resource::<Assets<rizzardry::content::npcs::FactCatalog>>()
        .get(&app.world().resource::<NpcAssets>().facts)
        .unwrap();
    assert_eq!(app.world().resource::<conversation::StoryFacts>().get(fact, facts), Ok(1));
    assert!(app.world().get::<rizzardry::game::party::roster::NeedsEquipment>(actor).is_none());
    assert_eq!(
        app.world_mut()
            .query::<&Npc>()
            .iter(app.world())
            .filter(|n| n.definition == burz_id)
            .count(),
        1
    );
}
