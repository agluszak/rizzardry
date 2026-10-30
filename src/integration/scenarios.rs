//! Scenarios on retail content: the real game, built headless as `main`
//! builds it, driven through gameplay requests (never simulated input) and
//! checked on its state. They need `RIZZARDRY_RETAIL` and fail without it.
//!
//! ```sh
//! RIZZARDRY_RETAIL=/path/to/wizardry8 cargo nextest run --run-ignored only -E 'test(/^scenarios::/)'
//! ```

use crate::app::{Options, build};
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::inventory::PartyInventory;
use crate::game::world::items::WorldItem;
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
fn a_green_slime_leaves_treasure() {
    // With this seed the three Green Slimes leave two item stacks.
    let mut app = game(&["--continuous", "--seed", "9", "--start=69.5,10,-24", "--yaw", "0"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    let before = lying_items(app.world_mut());
    step_until(&mut app, "a fight with the slimes", |w| fighting(w));
    step_until(&mut app, "the fight's end", |w| !fighting(w));
    assert_eq!(lying_items(app.world_mut()), before + 2);

    // Taking the resolved treasure, within reach, uses the same gameplay
    // operation as the rendered input adapter.
    let world = app.world_mut();
    let loot = world.query::<(Entity, &Name)>().iter(world).find(|(_, n)| n.as_str() == "loot");
    let (loot, _) = loot.expect("the treasure lies in the world");
    let at = world.get::<Transform>(loot).unwrap().translation;
    let mut body = world
        .query_filtered::<&mut avian3d::prelude::Position, With<crate::game::party::body::PartyBody>>()
        .single_mut(world)
        .unwrap();
    body.0 = at + Vec3::X;
    crate::game::world::items::take_item(world, loot).unwrap();
    step_until(&mut app, "taking the treasure", |w| {
        w.try_query::<&PartyInventory>().unwrap().single(w).is_ok_and(|p| p.hand.held.is_some())
    });
    assert_eq!(lying_items(app.world_mut()), before + 1);
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
fn burz_conversation_trade_and_revisit_keep_one_actor() {
    use crate::game::inventory::{ItemOwner, ItemRef};
    use crate::game::npcs::conversation::{
        self, Conversation, ConversationRequest, ConversationWait,
    };
    use crate::game::npcs::trade::TradeSource;
    use crate::game::npcs::{Npc, NpcBodies};
    let mut app = game(&["--seed", "8", "--start=225,36.7,-68.6"]);
    step_until(&mut app, "the Monastery becoming active", |w| playing(w).is_some());
    let catalog = &app.world().resource::<Catalogs>().npcs;
    let burz_id = crate::content::npcs::NpcId(
        catalog.0.iter().position(|d| d.source_name == "Burz").unwrap() as u16,
    );
    let actor = app
        .world_mut()
        .query::<(Entity, &crate::game::npcs::ActorId)>()
        .iter(app.world())
        .find(|(_, identity)| identity.npc() == burz_id)
        .unwrap()
        .0;
    assert!(
        app.world().get::<ChildOf>(actor).is_none(),
        "map lifetime does not own the named actor"
    );
    let body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    let position = app.world().get::<avian3d::prelude::Position>(body).unwrap().0;
    for mut p in app.world_mut().query_filtered::<&mut avian3d::prelude::Position,With<crate::game::party::body::PartyBody>>().iter_mut(app.world_mut()){p.0=position+Vec3::X*2.0;}
    let party = crate::game::party::roster::the_party(app.world_mut());
    app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 10000;
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
    let c = app.world().get::<Conversation>(session).unwrap();
    let topic = crate::game::npcs::conversation::script(app.world(), c).topic("MONASTERY").unwrap();
    app.world_mut().write_message(ConversationRequest::Topic {
        conversation: session,
        text: "MONASTERY".into(),
    });
    finish(&mut app, session);
    let c = app.world().get::<Conversation>(session).unwrap();
    assert!(matches!(c.wait, ConversationWait::Price { .. }));
    app.world_mut()
        .write_message(ConversationRequest::PriceAnswer { conversation: session, accept: true });
    finish(&mut app, session);
    let c = app.world().get::<Conversation>(session).unwrap();
    assert!(matches!(c.wait, ConversationWait::Options { .. }));
    app.world_mut()
        .write_message(ConversationRequest::Reply { conversation: session, text: "NO".into() });
    finish(&mut app, session);
    assert!(app.world().get::<Conversation>(session).unwrap().ready());
    assert!(app.world().get::<Npc>(actor).unwrap().paid_topics.contains(&topic));
    let lot =
        app.world().get::<Npc>(actor).unwrap().stock.lots.iter().find(|l| l.units > 0).unwrap().id;
    let quote =
        conversation::quote_trade(app.world_mut(), session, TradeSource::Stock(lot), 1).unwrap();
    let gold = app.world().get::<PartyInventory>(party).unwrap().gold;
    app.world_mut().write_message(ConversationRequest::Trade(quote));
    app.world_mut().write_message(ConversationRequest::Trade(quote));
    app.update();
    assert_eq!(
        app.world().get::<PartyInventory>(party).unwrap().gold,
        gold - quote.gold,
        "duplicate confirmation settles once"
    );
    let bought = app.world().get::<PartyInventory>(party).unwrap().pool.stacks[0].id;
    let sale = conversation::quote_trade(
        app.world_mut(),
        session,
        TradeSource::Party(ItemRef { owner: ItemOwner::Pool, id: bought }),
        1,
    )
    .unwrap();
    app.world_mut().write_message(ConversationRequest::Trade(sale));
    app.update();
    assert_eq!(
        app.world().get::<PartyInventory>(party).unwrap().gold,
        gold - quote.gold + sale.gold
    );
    let c = app.world().get::<Conversation>(session).unwrap();
    let current = c.current.unwrap().0;
    let fact = crate::game::npcs::conversation::script(app.world(), c).quotes[current as usize]
        .operations
        .iter()
        .find_map(|op| match op {
            crate::content::npcs::DialogueOp::SetFact(id, _) => Some(*id),
            _ => None,
        })
        .unwrap();
    let stock = app.world().get::<Npc>(actor).unwrap().stock.clone();
    let invalid = {
        let server = app.world().resource::<AssetServer>();
        crate::game::world::level::Level::load("levels/missing/missing.pvl", server)
    };
    let invalid = app
        .world_mut()
        .spawn(crate::game::world::level::Destination::new(
            invalid,
            default(),
            crate::game::world::level::Entry::Travel,
        ))
        .id();
    step_until(&mut app, "failed preparation", |w| w.get_entity(invalid).is_err());
    assert!(app.world().get::<Conversation>(session).is_none());
    assert!(playing(app.world_mut()).is_some_and(|p| p.ends_with("monastery1.pvl")));
    assert_eq!(app.world().get::<Npc>(actor).unwrap().stock, stock);

    app.world_mut().write_message(ConversationRequest::Close { conversation: session });
    app.update();
    assert!(app.world().get_entity(session).is_err());
    app.world_mut().write_message(Travel { entrance: "MO204".into() });
    step_until(&mut app, "leaving Burz's level", |w| {
        playing(w).is_some_and(|p| p.ends_with("monastery2.pvl"))
    });
    assert!(app.world().get::<Monster>(actor).is_none());
    assert!(app.world().get::<bevy::ecs::entity_disabling::Disabled>(body).is_some());
    let stock_offmap = app.world().get::<Npc>(actor).unwrap().stock.clone();
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(app.world().get::<Npc>(actor).unwrap().stock, stock_offmap);
    app.world_mut().write_message(Travel { entrance: "MO105".into() });
    step_until(&mut app, "returning to Burz", |w| {
        playing(w).is_some_and(|p| p.ends_with("monastery1.pvl"))
    });
    let returned_body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    assert!(app.world().get::<Monster>(returned_body).is_some());
    assert!(app.world().get::<bevy::ecs::entity_disabling::Disabled>(body).is_none());
    let npc = app.world().get::<Npc>(actor).unwrap();
    assert_eq!(npc.stock, stock);
    assert!(npc.paid_topics.contains(&topic));
    assert!(!npc.greeting_pending);
    let facts = &app.world().resource::<Catalogs>().facts;
    assert_eq!(app.world().resource::<conversation::StoryFacts>().get(fact, facts), Ok(1));
    assert_eq!(
        app.world_mut()
            .query::<(&crate::game::npcs::ActorId, &Npc)>()
            .iter(app.world())
            .filter(|(identity, _)| identity.npc() == burz_id)
            .count(),
        1
    );
    let position = app.world().get::<avian3d::prelude::Position>(returned_body).unwrap().0;
    for mut p in app.world_mut().query_filtered::<&mut avian3d::prelude::Position, With<crate::game::party::body::PartyBody>>().iter_mut(app.world_mut()) {
        p.0 = position + Vec3::X * 2.0;
    }
    let reopened = conversation::begin(app.world_mut(), actor).unwrap();
    app.update();
    finish(&mut app, reopened);
    // The retained NPC uses its repeat greeting, not the first meeting branch.
    assert_eq!(app.world().get::<Conversation>(reopened).unwrap().current, Some((1, 0)));
    assert!(app.world().get::<Conversation>(reopened).unwrap().ready());
}
