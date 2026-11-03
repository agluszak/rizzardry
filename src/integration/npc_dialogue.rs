//! Requests cross real scheduling and command boundaries in these scenarios.
use super::*;
use crate::content::{
    items::{EquipClass, ItemId},
    npcs::*,
};
use crate::game::{
    characters::Vitals,
    inventory::{Carried, ItemOwner, ItemRef, PartyInventory},
    npcs::{
        Npc, NpcAssets, NpcBodies,
        conversation::*,
        recruit,
        trade::{MerchantStock, TradeSource},
    },
    party::{
        formation::Formation,
        roster::{PartyMember, PartyRoster},
    },
};
use avian3d::prelude::Position;

fn fixture() -> (App, Entity, Entity, Entity) {
    let mut app = app();
    with_data(&mut app);
    let character = NpcCharacter {
        name: "Companion".into(),
        profession: Profession::Fighter,
        race: Race::Human,
        gender: Gender::Male,
        portrait: 0,
        level: 3,
        attributes: enum_map::EnumMap::from_fn(|_| 60),
        skills: default(),
        spells: vec![],
        equipment: [None; 12],
        backpack: [Some(ItemId(0)), None, None, None, None, None, None, None],
        wanted: vec![],
    };
    {
        let mut content = app.world_mut().resource_mut::<Catalogs>();
        content.species.0[0].npc = Some(NpcId(0));
        content.items.0[0].value = 100;
        content.items.0[0].class = EquipClass::Potion;
        content.npcs = NpcCatalog(vec![NpcDefinition {
            source_name: "Companion".into(),
            disposition: 80,
            merchant: true,
            owns_stock: true,
            character: Some(character),
            buy_factor: 1.0,
            sell_factor: 1.0,
            accepted_classes: EquipClass::Potion.into(),
            stock: vec![StockRule { item: ItemId(0), quantity: 3, persistent: true }],
            ..default()
        }]);
    }
    let mut quotes = vec![Quote::default(); 0x20];
    quotes[0].text = vec!["Hello".into()];
    quotes[0xd].text = vec!["I'll join".into()];
    quotes[5] = Quote {
        text: vec!["First part".into(), "Last part".into()],
        operations: vec![
            DialogueOp::Keywords(vec!["reward".into()]),
            DialogueOp::Price { gold: 10, always: false },
            DialogueOp::GiveGold(50),
        ],
        ..default()
    };
    let script =
        app.world_mut().resource_mut::<Assets<DialogueScript>>().add(DialogueScript { quotes });
    app.insert_resource(NpcAssets { scripts: [(NpcId(0), script)].into() });
    let level = spawn_level(&mut app, floor());
    app.update();
    let actor = app.world_mut().query_filtered::<Entity, With<Npc>>().single(app.world()).unwrap();
    let body = app.world().get::<NpcBodies>(actor).unwrap()[0];
    let at = app.world().get::<Transform>(body).unwrap().translation;
    let party = party_entities(&mut app)[0];
    app.world_mut().get_mut::<Position>(party).unwrap().0 = at + Vec3::X;
    app.world_mut().get_mut::<PartyInventory>(party).unwrap().gold = 1000;
    (app, actor, party, level)
}
fn begin(app: &mut App, actor: Entity) -> Entity {
    app.world_mut().write_message(BeginConversation { actor });
    app.world_mut().write_message(BeginConversation { actor });
    app.update();
    app.world_mut()
        .query_filtered::<Entity, With<Conversation>>()
        .single(app.world())
        .expect("only one request can acquire the party")
}
fn finish(app: &mut App, entity: Entity) {
    for _ in 0..20 {
        let Some(c) = app.world().get::<Conversation>(entity) else { return };
        match c.wait {
            ConversationWait::Line { speech } => {
                app.world_mut().write_message(LineSpoken { conversation: entity, speech });
            }
            ConversationWait::Blocked { ref reason, .. } => panic!("{reason}"),
            _ if c.ready() || !matches!(c.wait, ConversationWait::Ready) => return,
            _ => {}
        }
        app.update();
    }
    panic!("dialogue did not finish its speech");
}
fn request(app: &mut App, entity: Entity, talk: Talk) {
    app.world_mut().write_message(ConversationRequest::new(entity, talk));
    app.update();
}

#[test]
fn conversations_serialize_starts_and_trade_confirmations() {
    let (mut app, actor, party, _) = fixture();
    let session = begin(&mut app, actor);
    finish(&mut app, session);
    let lot = app.world().get::<MerchantStock>(actor).unwrap().lots[0].id;
    let quote = app
        .world_mut()
        .run_system_cached_with(quote_trade, (session, TradeSource::Stock(lot), 1))
        .unwrap()
        .unwrap();
    app.world_mut().write_message(ConversationRequest::trade(quote));
    app.world_mut().write_message(ConversationRequest::trade(quote));
    app.update();
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 1000 - quote.gold);
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().pool.stacks.len(), 1);
    // A quote against a former member's pack may not sell their belongings.
    request(&mut app, session, Talk::Close);
    app.world_mut().run_system_cached_with(recruit::join, actor).unwrap().unwrap();
    let id = app.world().get::<Carried>(actor).unwrap().backpack[0].as_ref().unwrap().id;
    app.world_mut().resource_mut::<Catalogs>().npcs.0[0]
        .character
        .as_mut()
        .unwrap()
        .wanted
        .push(ItemId(0));
    assert!(app.world_mut().run_system_cached_with(recruit::dismiss, actor).unwrap());
    let session = begin(&mut app, actor);
    finish(&mut app, session);
    let sale = app
        .world_mut()
        .run_system_cached_with(
            quote_trade,
            (session, TradeSource::Party(ItemRef { owner: ItemOwner::Member(actor), id }), 1),
        )
        .unwrap();
    assert!(sale.is_err());
}

#[test]
fn paid_answers_settle_once_when_travel_interrupts_speech() {
    let (mut app, actor, party, source) = fixture();
    let session = begin(&mut app, actor);
    finish(&mut app, session);
    request(&mut app, session, Talk::Topic { text: "reward".into() });
    assert!(matches!(
        app.world().get::<Conversation>(session).unwrap().wait,
        ConversationWait::Price { .. }
    ));
    request(&mut app, session, Talk::PriceAnswer { accept: true });
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 990);
    assert!(matches!(
        app.world().get::<Conversation>(session).unwrap().wait,
        ConversationWait::Line { .. }
    ));
    // Already prepared destination: activation and interruption share one update.
    let destination = spawn_level(&mut app, floor());
    app.update();
    assert!(is_played(&app, destination));
    assert!(app.world().get_entity(source).is_err());
    assert!(app.world().get_entity(session).is_err());
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 1040);
    app.update();
    assert_eq!(app.world().get::<PartyInventory>(party).unwrap().gold, 1040);
}

#[test]
fn recruited_npc_keeps_identity_and_membership_owns_its_seat() {
    let (mut app, actor, party, _) = fixture();
    let session = begin(&mut app, actor);
    finish(&mut app, session);
    request(&mut app, session, Talk::Recruit);
    finish(&mut app, session);
    assert!(app.world().get::<PartyRoster>(party).unwrap().contains(&actor));
    assert!(app.world().get::<Formation>(party).unwrap().row_of(actor).is_some());
    assert!(app.world().get_entity(session).is_err());
    app.world_mut().get_mut::<Character>(actor).unwrap().award_experience(123);
    let experience = app.world().get::<Character>(actor).unwrap().experience;
    assert!(app.world_mut().run_system_cached_with(recruit::dismiss, actor).unwrap());
    assert!(app.world().get::<Formation>(party).unwrap().row_of(actor).is_none());
    app.world_mut().run_system_cached_with(recruit::join, actor).unwrap().unwrap();
    assert_eq!(app.world().get::<Character>(actor).unwrap().experience, experience);
    assert!(app.world().get::<Carried>(actor).unwrap().backpack.iter().all(Option::is_none));
    // Death must release the membership and seat, and reject future recruitment.
    app.world_mut().get_mut::<Vitals>(actor).unwrap().health.current = 0;
    app.update();
    assert!(app.world().get::<PartyMember>(actor).is_none());
    assert!(app.world().get::<Formation>(party).unwrap().row_of(actor).is_none());
    assert!(app.world_mut().run_system_cached_with(recruit::join, actor).unwrap().is_err());
}
