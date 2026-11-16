//! A conversation, driven through its plugin by what the party does and
//! checked by what the player would see: what the person says, what was
//! paid, how they now stand.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::*;
use crate::content::characters::{Gender, Profession, Race, Skill};
use crate::content::npcs::{
    DialogueOp, DialogueScript, FactCatalog, FactDefinition, FactId, Gift, JournalVisibility,
    NpcCatalog, NpcDefinition, NpcId, Quote, Response,
};
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::PartyInventory;
use crate::game::npcs::{ActorId, Npc, NpcAssets, NpcTrigger};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::{Party, PartyMember};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::level::Level;

/// The topic quotes of the script below, after the stock ones.
const SWORD: u16 = 0x25;
const SHIELD: u16 = 0x26;
const HELM: u16 = 0x27;
const UMPANI: u16 = 0x28;
const PEACE: u16 = 0x29;
const UMPANI_FRIENDLY: FactId = FactId(7);

struct Scene {
    app: App,
    party: Entity,
    speaker: Entity,
    person: Entity,
}

/// One person with a script whose quote `n` says "q`n`", standing as their
/// record says, and a party of one with 500 gold.
fn scene(definition: NpcDefinition) -> Scene {
    let mut quotes: Vec<Quote> =
        (0..=0x76).map(|n| Quote { text: vec![format!("q{n}")], ..default() }).collect();
    let topic = |word: &str| DialogueOp::Keywords(vec![word.to_owned()]);
    quotes[usize::from(SWORD)].operations =
        vec![topic("sword"), DialogueOp::Price { gold: 100, always: false }];
    quotes[usize::from(SHIELD)].operations = vec![
        topic("shield"),
        DialogueOp::Give(Gift::Gold(30)),
        DialogueOp::Next(Response::Fixed(3)),
    ];
    quotes[usize::from(HELM)].operations = vec![topic("helm")];
    quotes[usize::from(UMPANI)].operations = vec![
        topic("umpani"),
        DialogueOp::Conditional { facts: vec![(UMPANI_FRIENDLY, 1)], response: Response::Fixed(4) },
        DialogueOp::Next(Response::Fixed(6)),
    ];
    quotes[usize::from(PEACE)].operations =
        vec![topic("peace"), DialogueOp::SetFact(UMPANI_FRIENDLY, 1)];
    let fact = FactDefinition {
        highlight_when_true: false,
        visibility: JournalVisibility::AllDifficulties,
        id: UMPANI_FRIENDLY,
        symbol: "FACT_FACTION_UMPANI_COMMON_FRIENDLY".into(),
        description: String::new(),
        alternate_description: String::new(),
    };
    let mut app = App::new();
    crate::game::items::plugin(&mut app);
    crate::game::actors::plugin(&mut app);
    let mut scripts = Assets::<DialogueScript>::default();
    let script = scripts.add(DialogueScript { quotes });
    app.insert_resource(scripts)
        .insert_resource(NpcAssets { scripts: [(NpcId(0), script)].into(), ..default() })
        .insert_resource(Catalogs {
            npcs: NpcCatalog(vec![definition]),
            facts: FactCatalog(vec![fact]),
            ..default()
        })
        .insert_resource(GameRng::seeded(1))
        .init_resource::<GameClock>()
        .init_resource::<crate::game::factions::Factions>()
        .init_resource::<crate::game::npcs::story::StoryFacts>()
        .add_message::<crate::game::factions::FactionNews>()
        .add_message::<crate::game::hits::Damaged>();
    install(&mut app);
    app.add_message::<crate::game::npcs::story::StoryTold>()
        .add_observer(crate::game::npcs::story::dispatch)
        .add_observer(crate::game::npcs::story::follow)
        .add_observer(crate::game::npcs::story::befalls_the_story);
    crate::game::npcs::trade::install(&mut app);
    let world = app.world_mut();
    let party = world.spawn((Party, Position(Vec3::ZERO), PartyBody::default())).id();
    world.get_mut::<PartyInventory>(party).unwrap().gold = 500;
    let level = world
        .spawn(Level {
            path: String::new(),
            definition: default(),
            navmeshes: vec![],
            messages: None,
        })
        .id();
    let sheet = Character::new("Vi", Race::Human, Gender::Female, Profession::Fighter);
    let vitals = Vitals::new(&sheet);
    let speaker = world.spawn((sheet, vitals, PartyMember(party))).id();
    let npc = Npc::from_definition(&world.resource::<Catalogs>().npcs.0[0]);
    let person = world.spawn((ActorId(NpcId(0)), npc, NpcTrigger(level))).id();
    world.flush();
    Scene { app, party, speaker, person }
}

impl Scene {
    fn world(&mut self) -> &mut World {
        self.app.world_mut()
    }

    fn conversation(&mut self) -> Option<Entity> {
        let world = self.world();
        world.query_filtered::<Entity, With<Conversation>>().iter(world).next()
    }

    /// What the party does next.
    fn act<E>(&mut self, act: impl FnOnce(Entity) -> E)
    where
        E: Event,
        for<'t> E::Trigger<'t>: Default,
    {
        let conversation = self.conversation().expect("a conversation");
        self.world().trigger(act(conversation));
        self.world().flush();
    }

    /// Hear the person out: every line they say until they stop.
    fn hear(&mut self) -> Vec<String> {
        let mut heard = Vec::new();
        loop {
            let world = self.world();
            let line = world.query::<(Entity, &ConversationSpeech)>().iter(world).next();
            let Some((speech, line)) = line.map(|(speech, line)| (speech, line.text.clone()))
            else {
                return heard;
            };
            heard.push(line);
            world.trigger(Spoken { speech });
            world.flush();
        }
    }

    fn begin(&mut self) -> Vec<String> {
        let actor = self.person;
        self.world().trigger(Begin { actor, quote: None, with_held: false });
        self.world().flush();
        self.hear()
    }

    fn gold(&mut self) -> u32 {
        let party = self.party;
        self.world().get::<PartyInventory>(party).unwrap().gold
    }

    fn regard(&mut self) -> u8 {
        let person = self.person;
        self.world().get::<Npc>(person).unwrap().disposition
    }

    fn asked(&mut self) -> Option<Asked> {
        let conversation = self.conversation()?;
        self.world().get::<Asked>(conversation).cloned()
    }
}

fn friend() -> NpcDefinition {
    NpcDefinition { source_name: "Rosa".into(), disposition: 80, ..default() }
}

/// A friend greets the party; a topic with a price is asked for and said
/// once paid, the price less a five-hundredth for each point of the party's
/// best Communication; a topic that gives gold gives it after its line and
/// before what follows; leaving ends it. There is only ever one
/// conversation.
#[test]
fn a_friend_greets_answers_what_is_paid_for_and_is_left() {
    let mut scene = scene(friend());
    let speaker = scene.speaker;
    scene.world().get_mut::<crate::game::characters::Training>(speaker).unwrap().0
        [Skill::Communication] = 100;
    scene.world().run_schedule(crate::game::actors::Settle);
    assert_eq!(scene.begin(), ["q0"], "the first greeting");
    scene.begin();
    let world = scene.world();
    assert_eq!(world.query::<&Conversation>().iter(world).count(), 1);

    scene.act(|conversation| Ask { conversation, text: "Sword?".into(), where_is: false });
    assert_eq!(scene.hear(), ["q18"], "they name a price before saying it");
    assert_eq!(scene.asked(), Some(Asked::Price { quote: SWORD, gold: 80, always: false }));
    scene.act(|conversation| Answer { conversation, reply: Reply::Accept(true) });
    assert_eq!(scene.hear(), ["q37"]);
    assert_eq!(scene.gold(), 420);
    // Paid for once, it is said again for nothing.
    scene.act(|conversation| Ask { conversation, text: "sword".into(), where_is: false });
    assert_eq!(scene.hear(), ["q37"]);
    assert_eq!(scene.gold(), 420);

    scene.act(|conversation| Ask { conversation, text: "shield".into(), where_is: false });
    assert_eq!(scene.hear(), ["q38", "q3"]);
    assert_eq!(scene.gold(), 450);
    let conversation = scene.conversation().unwrap();
    let transcript = &scene.world().get::<Conversation>(conversation).unwrap().transcript;
    let said: Vec<_> = transcript.iter().rev().take(3).rev().cloned().collect();
    let given = QuestReward::Gold { giver: "Rosa".into(), amount: 30 };
    assert_eq!(
        said,
        [
            TranscriptLine::Npc("q38".into()),
            TranscriptLine::Reward(given),
            TranscriptLine::Npc("q3".into())
        ]
    );

    scene.act(|conversation| Leave { conversation });
    assert_eq!(scene.hear(), ["q92"], "a friend says farewell");
    assert_eq!(scene.conversation(), None);
}

/// A price refused costs nothing and the quote is not said; one the party
/// cannot pay is answered with 0x14.
#[test]
fn a_price_refused_or_beyond_the_purse_is_not_paid() {
    let mut scene = scene(friend());
    scene.begin();
    scene.act(|conversation| Ask { conversation, text: "sword".into(), where_is: false });
    scene.hear();
    scene.act(|conversation| Answer { conversation, reply: Reply::Accept(false) });
    assert_eq!(scene.hear(), [] as [&str; 0]);
    assert_eq!(scene.gold(), 500);

    let party = scene.party;
    scene.world().get_mut::<PartyInventory>(party).unwrap().gold = 99;
    scene.act(|conversation| Ask { conversation, text: "sword".into(), where_is: false });
    scene.hear();
    scene.act(|conversation| Answer { conversation, reply: Reply::Accept(true) });
    assert_eq!(scene.hear(), ["q20"]);
    assert_eq!(scene.gold(), 99);
}

/// A topic the person does not have is answered with 0x23 or 0x24; two
/// words they have are both answered, with 0x21 between; more than two
/// with 0x20.
#[test]
fn topics_are_matched_by_word() {
    let mut scene = scene(friend());
    scene.begin();
    scene.act(|conversation| Ask { conversation, text: "dragons".into(), where_is: false });
    let unknown = scene.hear();
    assert!(unknown == ["q35"] || unknown == ["q36"], "{unknown:?}");
    scene.act(|conversation| Ask {
        conversation,
        text: "the helm or that shield".into(),
        where_is: false,
    });
    assert_eq!(scene.hear(), ["q39", "q33", "q38", "q3"]);
    scene.act(|conversation| Ask {
        conversation,
        text: "helm shield sword".into(),
        where_is: false,
    });
    assert_eq!(scene.hear(), ["q32"]);
}

/// One who is neither friend nor foe keeps the party to gifts and charm.
/// Gold counts against what they still want: short of it they say 7; once
/// it is used up they are a friend (75), say 0x10 and greet the party.
#[test]
fn gold_buys_a_cold_persons_friendship_once_it_covers_what_they_want() {
    let cold = NpcDefinition { disposition: 50, trade_pool: 100, ..friend() };
    let mut scene = scene(cold);
    assert_eq!(scene.begin(), ["q2"]);
    scene.act(|conversation| Ask { conversation, text: "shield".into(), where_is: false });
    assert_eq!(scene.hear(), [] as [&str; 0], "they will not talk of anything yet");

    scene.act(|conversation| OfferGold { conversation, amount: 60, sure: false });
    assert_eq!(scene.hear(), ["q7"]);
    assert_eq!((scene.gold(), scene.regard()), (440, 50));
    scene.act(|conversation| OfferGold { conversation, amount: 40, sure: false });
    assert_eq!(scene.hear(), ["q16", "q0"]);
    assert_eq!((scene.gold(), scene.regard()), (400, 75));
    scene.act(|conversation| Ask { conversation, text: "helm".into(), where_is: false });
    assert_eq!(scene.hear(), ["q39"]);
}

/// Giving everything the party has is asked about first.
#[test]
fn giving_all_the_gold_asks_first() {
    let mut scene = scene(friend());
    scene.begin();
    scene.act(|conversation| OfferGold { conversation, amount: 500, sure: false });
    assert_eq!(scene.asked(), Some(Asked::GiveAll { gold: 500 }));
    assert_eq!(scene.gold(), 500);
    scene.act(|conversation| Answer { conversation, reply: Reply::Accept(true) });
    assert_eq!(scene.hear(), ["q16"]);
    assert_eq!(scene.gold(), 0);
}

/// Small talk moves a person by their own figure for it, and they answer
/// by how they then stand; it cannot be tried again for twelve hours.
#[test]
fn small_talk_moves_a_person_once_in_twelve_hours() {
    let cold = NpcDefinition { disposition: 50, talk_scale: 8, ..friend() };
    let mut scene = scene(cold);
    scene.begin();
    let chat = |conversation| Sway { conversation, approach: Approach::SmallTalk };
    scene.act(chat);
    assert_eq!(scene.hear(), ["q3"]);
    assert_eq!(scene.regard(), 58);
    scene.act(chat);
    assert_eq!(scene.regard(), 58, "not again so soon");
    scene.world().resource_mut::<GameClock>().ms += 12 * crate::game::time::HOUR_MS + 1;
    scene.act(chat);
    assert_eq!(scene.hear(), ["q0"], "won over, they greet the party");
    assert_eq!(scene.regard(), 66);
}

/// Small talk that makes a person hostile ends the conversation.
#[test]
fn small_talk_that_offends_ends_it() {
    let touchy = NpcDefinition { disposition: 40, talk_scale: -10, ..friend() };
    let mut scene = scene(touchy);
    scene.begin();
    scene.act(|conversation| Sway { conversation, approach: Approach::SmallTalk });
    assert_eq!(scene.hear(), ["q24"]);
    assert_eq!(scene.conversation(), None);
}

/// A hostile person says so and there is nothing to be done with them.
#[test]
fn a_hostile_person_will_not_talk() {
    let mut scene = scene(NpcDefinition { disposition: 10, ..friend() });
    scene.begin();
    let heard: Vec<_> =
        scene.world().resource_mut::<Messages<NpcSpeech>>().drain().map(|line| line.text).collect();
    assert_eq!(heard, ["q24"]);
    assert_eq!(scene.conversation(), None);
}

/// What a person says may turn on a fact read off the world: here, on
/// whether a faction is friendly now.
#[test]
fn a_quote_follows_a_fact_read_off_the_world() {
    use crate::content::factions::Faction;
    use crate::game::factions::Factions;
    let mut scene = scene(friend());
    scene.begin();
    scene.world().resource_mut::<Factions>().adjust(Faction::Umpani, -100, 0);
    scene.act(|conversation| Ask { conversation, text: "umpani".into(), where_is: false });
    assert_eq!(scene.hear(), ["q40", "q6"]);
    scene.world().resource_mut::<Factions>().adjust(Faction::Umpani, 100, 0);
    scene.act(|conversation| Ask { conversation, text: "umpani".into(), where_is: false });
    assert_eq!(scene.hear(), ["q40", "q4"]);
}

/// What a person says may set a fact, and what that sets off happens: here
/// a faction becomes friendly, which the next thing asked turns on.
#[test]
fn a_quote_sets_a_fact_and_what_it_sets_off_follows() {
    use crate::content::factions::Faction;
    use crate::game::factions::Factions;
    let mut scene = scene(friend());
    scene.begin();
    scene.world().resource_mut::<Factions>().adjust(Faction::Umpani, -100, 0);
    scene.act(|conversation| Ask { conversation, text: "peace".into(), where_is: false });
    assert_eq!(scene.hear(), ["q41"]);
    scene.act(|conversation| Ask { conversation, text: "umpani".into(), where_is: false });
    assert_eq!(scene.hear(), ["q40", "q4"]);
}

/// Asked where something is, a person answers with what they know of the
/// name, or that they do not know; asked after themselves, that they are
/// here. `join` asks them to join.
#[test]
fn a_person_says_where_things_are_if_they_know() {
    // The bit for the Umpani.
    let mut scene = scene(NpcDefinition { knows: 0x20, ..friend() });
    scene.begin();
    let wheres = |text: &str| {
        let text = text.to_owned();
        move |conversation| Ask { conversation, text, where_is: true }
    };
    scene.act(wheres("Umpani"));
    assert_eq!(scene.hear(), ["q111"]);
    scene.act(wheres("Rattkin"));
    assert_eq!(scene.hear(), ["q118"], "they know nothing of the Rattkin");
    scene.act(|conversation| Ask { conversation, text: "where is Rosa".into(), where_is: false });
    assert_eq!(scene.hear(), ["q117"]);
    scene.act(|conversation| Ask { conversation, text: "join us".into(), where_is: false });
    assert_eq!(scene.hear(), ["q11"], "one with no character to bring will not join");
}

/// Turning on a person is asked about first; then they say they want
/// nothing more of the party, it is over, and they are cool toward it.
#[test]
fn turning_on_a_person_ends_it_badly() {
    let mut scene = scene(friend());
    scene.begin();
    scene.act(|conversation| Attack { conversation });
    assert_eq!(scene.asked(), Some(Asked::Attack));
    scene.act(|conversation| Answer { conversation, reply: Reply::Accept(true) });
    assert_eq!(scene.hear(), ["q24"]);
    assert_eq!(scene.conversation(), None);
    assert_eq!(scene.regard(), 25);
}

/// A talk begun with something in the party's hand, as a device that speaks
/// for someone begins it, opens on what they make of it and not on their
/// greeting (retail `BeginNpcDialogue` with an item).
#[test]
fn a_talk_begun_with_an_item_answers_it_instead_of_greeting() {
    use crate::content::items::{ItemCatalog, ItemDefinition, ItemId};
    use bevy::ecs::system::RunSystemOnce;
    let mut scene = scene(friend());
    let party = scene.party;
    let world = scene.world();
    let definition = ItemDefinition { id: ItemId(0), name: "Ball".into(), ..default() };
    world.resource_mut::<Catalogs>().items = ItemCatalog(vec![definition.clone()]);
    world
        .run_system_once(move |mut commands: Commands| {
            let item = crate::game::items::make(&mut commands, &definition, 1, true);
            commands.trigger(crate::game::items::Put {
                item,
                at: crate::game::items::Place::Hand { party },
            });
        })
        .unwrap();
    world.flush();
    let actor = scene.person;
    scene.world().trigger(Begin { actor, quote: None, with_held: true });
    scene.world().flush();
    let heard = scene.hear();
    assert!(!heard.is_empty(), "they answer");
    assert!(!heard.contains(&"q0".to_owned()), "no greeting: {heard:?}");
}
