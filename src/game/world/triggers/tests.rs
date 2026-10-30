use super::*;
use std::time::Duration;

use crate::content::items::{Dice, ItemCatalog, ItemDefinition, ItemId, LootTable, LootTables};
use crate::game::inventory::{ItemIds, ItemStack, make_item};
use crate::game::world::scripts::LevelScript;

use crate::content::level::{Animation, Arrival, DoorLock, PropModel, StateChange, TriggerSound};
use crate::game::characters::{Gender, Profession, Race};

pub(crate) fn act(action: TriggerAction) -> TriggerUse {
    TriggerUse { action, message: None }
}

pub(crate) fn trigger(name: &str, action: TriggerAction) -> TriggerDefinition {
    TriggerDefinition {
        name: name.into(),
        usable: false,
        once: false,
        fire_linked: false,
        initial: act(action),
        alternate: None,
        fallback: None,
        alternate_toggles: false,
        recipients: vec![],
        sound: None,
        sound_on_commit: false,
        required_states: vec![],
        state_change: None,
        initial_state: None,
        range: (0.0, 10.0),
        door: None,
        place: None,
        loot_table: None,
        lock: None,
    }
}

/// Point each trigger's recipients at the named ones, as the decoder
/// resolves them.
pub(crate) fn link(triggers: &mut [TriggerDefinition], links: &[(&str, &[&str])]) {
    let index = |name: &str| triggers.iter().position(|t| t.name == name).unwrap();
    let resolved: Vec<(usize, Vec<usize>)> = links
        .iter()
        .map(|(from, to)| (index(from), to.iter().map(|t| index(t)).collect()))
        .collect();
    for (from, to) in resolved {
        triggers[from].recipients = to;
    }
}

/// A prop running `trigger`: its frames to the end, whether each play
/// restarts, and its dial slots.
pub(crate) struct PropSpec<'a>(&'a str, f32, bool, &'a [u16]);

use bevy::ecs::system::SystemState;

/// The actual ECS state and operation parameter, with no presentation.
pub(crate) struct Scenario {
    pub(crate) world: World,
    pub(crate) geometry: LevelDefinition,
    pub(crate) script: LevelScript,
    pub(crate) root: Entity,
    pub(crate) devices: Vec<Entity>,
    pub(crate) members: Vec<Entity>,
}

impl Scenario {
    pub(crate) fn new(
        triggers: Vec<TriggerDefinition>,
        props: &[PropSpec],
        script: LevelScript,
    ) -> Self {
        let models = props
            .iter()
            .map(|&PropSpec(name, last, wraps, slots)| PropModel {
                name: name.into(),
                collidable: false,
                face_camera: false,
                frames_per_second: 10.0,
                // One part, its path keys running to the last frame.
                parts: vec![crate::content::level::PropModelPart {
                    transform: Transform::IDENTITY,
                    keys: vec![Transform::IDENTITY; last as usize + 1],
                    collision: default(),
                }],
                wraps,
                slots: slots.to_vec(),
                trigger: triggers.iter().position(|t| t.name == name),
            })
            .collect();
        let geometry = LevelDefinition { triggers, props: models, ..default() };
        let mut world = World::new();
        world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        world.init_resource::<Assets<LevelDefinition>>();
        world.init_resource::<Messages<TriggerEffect>>();
        world.init_resource::<Messages<DamageApplied>>();
        world.init_resource::<Messages<crate::game::world::hostility::Provocation>>();
        world.init_resource::<Messages<crate::game::combat::CombatNews>>();
        world.init_resource::<Messages<crate::game::inventory::ItemNews>>();
        world.init_resource::<Messages<super::super::interaction::LockNews>>();
        world.init_resource::<crate::game::difficulty::Difficulty>();
        world.init_resource::<crate::game::magic::casting::FlightOrder>();
        world.init_resource::<ItemIds>();
        let party = crate::game::party::roster::test_party(&mut world, &[]);
        world.entity_mut(party).insert((
            crate::game::party::body::PartyBody::default(),
            avian3d::prelude::Position(
                Vec3::X - Vec3::Y * crate::game::party::body::EYE_ABOVE_CENTRE,
            ),
        ));
        world.insert_resource(GameClock::at(0));
        world.insert_resource(GameRng::seeded(5));
        let items = ItemCatalog::default();
        let loot = LootTables::default();
        world.insert_resource(crate::game::data::Catalogs { items, loot, ..default() });
        let root = world.spawn_empty().id();
        let mut scenario = Self { world, geometry, script, root, devices: vec![], members: vec![] };
        scenario.visit(None);
        scenario.add_member("A");
        scenario
    }

    pub(crate) fn visit(&mut self, memory: Option<TriggerMemory>) {
        self.world.despawn(self.root);
        let handle =
            self.world.resource_mut::<Assets<LevelDefinition>>().add(self.geometry.clone());
        self.root = self.world.spawn(Level { script: self.script, ..Level::test(handle) }).id();
        let props: Vec<_> = self
            .geometry
            .props
            .iter()
            .map(|_| self.world.spawn(LevelOwned(self.root)).id())
            .collect();
        let tables =
            LootTables(self.world.resource::<crate::game::data::Catalogs>().loot.0.clone());
        let mut state = SystemState::<Commands>::new(&mut self.world);
        let mut commands = state.get_mut(&mut self.world).unwrap();
        self.devices = spawn_devices(
            &mut commands,
            self.root,
            &self.geometry,
            &props,
            self.script,
            &tables,
            memory,
        );
        for (index, &prop) in props.iter().enumerate() {
            commands.entity(prop).insert((Prop { definition: index }, PropMotion::default()));
        }
        state.apply(&mut self.world);
    }

    pub(crate) fn inventory(&mut self) -> Mut<'_, PartyInventory> {
        let party = crate::game::party::roster::the_party(&mut self.world).unwrap();
        self.world.get_mut::<PartyInventory>(party).unwrap()
    }

    pub(crate) fn formation(&mut self) -> &crate::game::party::formation::Formation {
        let party = crate::game::party::roster::the_party(&mut self.world).unwrap();
        self.world.get::<crate::game::party::formation::Formation>(party).unwrap()
    }

    pub(crate) fn device(&self, name: &str) -> Entity {
        self.devices[self.geometry.triggers.iter().position(|t| t.name == name).unwrap()]
    }

    pub(crate) fn add_member(&mut self, name: &str) -> Entity {
        let profile = Character::new(name, Race::Human, Gender::Male, Profession::Fighter);
        let member = self.world.spawn((Vitals::new(&profile), profile)).id();
        crate::game::party::roster::test_join(&mut self.world, member);
        self.members.push(member);
        member
    }

    pub(crate) fn effects(&mut self) -> Vec<TriggerEffect> {
        self.world.resource_mut::<Messages<TriggerEffect>>().drain().collect()
    }

    pub(crate) fn damage(&mut self) -> Vec<DamageApplied> {
        self.world.resource_mut::<Messages<DamageApplied>>().drain().collect()
    }

    pub(crate) fn run(&mut self, name: &str) -> (bool, Vec<TriggerEffect>) {
        let device = self.device(name);
        let committed = activate(&mut self.world, device, false);
        (committed, self.effects())
    }

    pub(crate) fn walk(&mut self, eye: Vec3, elapsed: Duration) -> Option<PartyMove> {
        self.world.run_system_cached_with(walk, Some((self.root, eye, elapsed))).unwrap();
        self.world.entity_mut(self.root).take::<PendingPartyMove>().map(|p| p.0)
    }

    pub(crate) fn motion(&self, name: &str) -> &PropMotion {
        self.world.get::<PropMotion>(self.device(name)).unwrap()
    }

    pub(crate) fn settle(&mut self) {
        for mut motion in self.world.query::<&mut PropMotion>().iter_mut(&mut self.world) {
            motion.frame = motion.target;
        }
    }

    pub(crate) fn variable(&self, name: &str) -> bool {
        self.world.get::<LevelState>(self.root).unwrap().variable(&name.into())
    }

    pub(crate) fn remember(&mut self) -> TriggerMemory {
        let mut records = SystemState::<DeviceRecords>::new(&mut self.world);
        let owned = self.world.get::<super::super::level::LevelEntities>(self.root).unwrap();
        records.get(&self.world).unwrap().remember(
            owned.entities(),
            self.world.get::<LevelState>(self.root).unwrap(),
            self.world.get::<LightActivation>(self.root).unwrap(),
        )
    }
}

pub(crate) fn chest(lock: Option<crate::content::level::Lock>) -> TriggerDefinition {
    let mut chest = trigger("CHEST", TriggerAction::Animate(Animation::Toggle));
    chest.loot_table = Some("CHEST".into());
    chest.usable = true;
    chest.lock = lock;
    chest
}

/// A chest whose table yields `gold` and nothing else.
pub(crate) fn chest_level(lock: Option<crate::content::level::Lock>, gold: i16) -> Scenario {
    let mut level =
        Scenario::new(vec![chest(lock)], &[PropSpec("CHEST", 5.0, false, &[])], LevelScript::None);
    let loot = LootTables(vec![LootTable {
        name: "CHEST".into(),
        entries: vec![],
        level_scaled: false,
        item_count: Dice::default(),
        gold: Dice { count: 0, sides: 0, base: gold },
    }]);
    level.world.resource_mut::<crate::game::data::Catalogs>().loot = loot;
    level.visit(None);
    level
}

pub(crate) fn work(level: &Scenario) -> Option<&super::super::interaction::Work> {
    let (on, _) = super::super::interaction::current(&level.world)?;
    level.world.get::<super::super::interaction::DeviceInteraction>(on).map(|i| &i.work)
}

/// The attempt under way.
pub(crate) fn attempt(level: &Scenario) -> Entity {
    super::super::interaction::current(&level.world).expect("an attempt").0
}

#[test]
fn a_chest_keeps_what_is_left_in_it_through_closing_and_revisits() {
    use super::super::containers::{Recipient, close_container, take_from_container};
    use crate::content::items::QuantityKind;
    let mut level = chest_level(None, 0);
    let mut arrows = ItemDefinition { id: ItemId(0), name: "arrow".into(), ..default() };
    arrows.quantity = QuantityKind::Stack;
    arrows.stack_limit = 99;
    let dagger = ItemDefinition { id: ItemId(1), name: "dagger".into(), ..default() };
    let catalog = ItemCatalog(vec![arrows.clone(), dagger.clone()]);
    level.world.resource_mut::<crate::game::data::Catalogs>().items = catalog;
    let chest = level.device("CHEST");
    let stacks = {
        let mut ids = level.world.resource_mut::<ItemIds>();
        vec![
            ItemStack::new(&arrows, &mut ids, 10, true),
            ItemStack::new(&dagger, &mut ids, 1, true),
        ]
    };
    let (quiver, knife) = (stacks[0].id, stacks[1].id);
    let mut container = level.world.get_mut::<Container>(chest).unwrap();
    container.table = None;
    container.contents.items = stacks;
    // Several items: the chest opens and its contents are looked through.
    assert!(level.run("CHEST").0);
    assert!(matches!(work(&level), Some(Work::Looting)));
    level.settle();
    let before = level.world.get::<Container>(chest).unwrap().contents.items.clone();
    for picks in [&[(quiver, 0)][..], &[(quiver, 11)], &[(knife, 1), (knife, 2)], &[]] {
        let on = attempt(&level);
        assert_eq!(
            take_from_container(&mut level.world, on, picks, Recipient::Party),
            Err(crate::game::inventory::Refusal::Nothing)
        );
    }
    assert_eq!(level.world.get::<Container>(chest).unwrap().contents.items, before);
    assert!(level.inventory().pool.stacks.is_empty());
    let entering = level
        .world
        .spawn(super::super::level::Destination::new(
            Level::test(default()),
            default(),
            super::super::level::Entry::Travel,
        ))
        .id();
    let on = attempt(&level);
    assert_eq!(
        take_from_container(&mut level.world, on, &[(quiver, 4)], Recipient::Party),
        Err(crate::game::inventory::Refusal::Nothing)
    );
    assert_eq!(level.world.get::<Container>(chest).unwrap().contents.items, before);
    assert!(level.inventory().pool.stacks.is_empty());
    level.world.despawn(entering);
    let on = attempt(&level);
    take_from_container(&mut level.world, on, &[(quiver, 4)], Recipient::Party).unwrap();
    let pool = &level.inventory().pool.stacks;
    assert_eq!(pool.iter().map(ItemStack::count).collect::<Vec<_>>(), [4]);
    let left = |level: &Scenario| {
        level.world.get::<Container>(level.device("CHEST")).unwrap().contents.items.clone()
    };
    assert_eq!(
        left(&level).iter().map(|s| (s.id, s.count())).collect::<Vec<_>>(),
        [(quiver, 6), (knife, 1)]
    );
    let on = attempt(&level);
    close_container(&mut level.world, on);
    assert!(work(&level).is_none());
    // Using the open chest looks again without closing it.
    level.run("CHEST");
    assert!(matches!(work(&level), Some(Work::Looting)));
    assert_eq!(level.motion("CHEST").target, 5.0);
    let member = level.members[0];
    level.world.entity_mut(member).insert(crate::game::inventory::Carried::default());
    let on = attempt(&level);
    take_from_container(&mut level.world, on, &[(knife, 1)], Recipient::Member(member)).unwrap();
    let carried = level.world.get::<crate::game::inventory::Carried>(member).unwrap();
    assert_eq!(carried.stacks().map(|s| s.id).collect::<Vec<_>>(), [knife]);
    let on = attempt(&level);
    close_container(&mut level.world, on);
    // Nothing is rolled again on return; the last stack goes to the hand.
    let memory = level.remember();
    level.visit(Some(memory));
    assert_eq!(left(&level).iter().map(|s| (s.id, s.count())).collect::<Vec<_>>(), [(quiver, 6)]);
    level.run("CHEST");
    assert!(work(&level).is_none());
    let held = level.inventory().hand.held.as_ref().map(|h| h.id);
    assert_eq!(held, Some(quiver));
    assert!(left(&level).is_empty());
    // Putting it back is all that using the chest with it in hand does.
    let target = level.motion("CHEST").target;
    level.settle();
    assert!(!level.run("CHEST").0);
    assert_eq!(left(&level).iter().map(|s| s.id).collect::<Vec<_>>(), [quiver]);
    assert_eq!(level.motion("CHEST").target, target);
}

#[test]
fn a_door_opens_then_toggles_and_a_locked_one_stays_shut() {
    let mut door = trigger("DOOR", TriggerAction::OpenDoor);
    door.alternate = Some(act(TriggerAction::ToggleDoor));
    let mut locked = trigger("LOCKED", TriggerAction::OpenDoor);
    locked.door = Some(DoorLock { locked: true, key: None });
    let mut plain = trigger("PLAIN", TriggerAction::OpenDoor);
    plain.usable = true;
    plain.range = (0.5, 2.0);
    let mut level = Scenario::new(
        vec![door, locked, plain],
        &[
            PropSpec("DOOR", 15.0, false, &[]),
            PropSpec("LOCKED", 15.0, false, &[]),
            PropSpec("PLAIN", 10.0, false, &[]),
        ],
        LevelScript::None,
    );
    assert!(level.run("DOOR").0);
    assert_eq!(level.motion("DOOR").target, 15.0);
    // Busy until the door finishes opening.
    assert!(!level.run("DOOR").0);
    level.settle();
    assert!(level.run("DOOR").0);
    assert_eq!(level.motion("DOOR").target, 0.0);
    level.settle();
    assert!(level.run("DOOR").0);
    assert_eq!(level.motion("DOOR").target, 15.0);
    assert_eq!(level.run("LOCKED"), (false, vec![TriggerEffect::Locked]));
    assert_eq!(level.motion("LOCKED").target, 0.0);
    let plain = level.device("PLAIN");
    let party = crate::game::party::roster::the_party(&mut level.world).unwrap();
    level.world.init_resource::<Messages<crate::game::world::WorldRequest>>();
    level.world.write_message(crate::game::world::WorldRequest::Activate { device: plain });
    // Move after queuing the click: execution must use the new position.
    level.world.get_mut::<avian3d::prelude::Position>(party).unwrap().x = 100.0;
    crate::game::world::handle_world_requests(&mut level.world);
    assert_eq!(level.motion("PLAIN").target, 0.0);
    level.world.get_mut::<avian3d::prelude::Position>(party).unwrap().x = 0.1;
    assert!(!activate(&mut level.world, plain, true), "minimum range still applies");
    level.world.get_mut::<avian3d::prelude::Position>(party).unwrap().x = 1.0;
    assert!(activate(&mut level.world, plain, true));
    level.settle();
    assert!(!level.run("PLAIN").0, "opening an open door does nothing");
    assert_eq!(level.motion("PLAIN").target, 10.0);
}

#[test]
fn a_key_in_hand_unlocks_its_door_for_good_and_otherwise_the_fallback_runs() {
    let key = ItemId(7);
    let mut door = trigger("DOOR", TriggerAction::OpenDoor);
    door.door = Some(DoorLock { locked: true, key: Some(key) });
    door.fallback = Some(TriggerUse { action: TriggerAction::Nothing, message: Some(3) });
    let mut level =
        Scenario::new(vec![door], &[PropSpec("DOOR", 15.0, false, &[])], LevelScript::None);
    // Without the key: the fallback, which shows its message.
    assert_eq!(level.run("DOOR"), (true, vec![TriggerEffect::Message { source: None, id: 3 }]));
    assert_eq!(level.motion("DOOR").target, 0.0);
    let definition = ItemDefinition { id: key, ..default() };
    let item =
        make_item(&definition, &mut ItemIds::default(), &mut GameRng::seeded(5), false, true);
    level.inventory().hand.held = Some(item);
    assert!(level.run("DOOR").0);
    assert_eq!(level.motion("DOOR").target, 15.0);
    // Unlocked for good: kept while the party is away.
    let memory = level.remember();
    let old = level.device("DOOR");
    level.visit(Some(memory));
    assert_ne!(old, level.device("DOOR"));
    assert!(!level.world.get::<DoorState>(level.device("DOOR")).unwrap().locked);
}

#[test]
fn a_container_opens_its_table_once_and_one_without_a_table_holds_nothing() {
    let chest = |name: &str, table: &str| {
        let mut chest = trigger(name, TriggerAction::Animate(Animation::Toggle));
        chest.loot_table = Some(table.into());
        chest
    };
    let props = [PropSpec("CHEST", 5.0, false, &[]), PropSpec("LOST", 5.0, false, &[])];
    let mut level = Scenario::new(
        vec![chest("CHEST", "Monastery Chest"), chest("LOST", "No Such Table")],
        &props,
        LevelScript::None,
    );
    // The tables are looked up when the level is prepared.
    let loot = LootTables(vec![LootTable {
        name: "MONASTERY CHEST".into(),
        entries: vec![],
        level_scaled: false,
        item_count: Dice::default(),
        gold: Dice { count: 0, sides: 0, base: 5 },
    }]);
    level.world.resource_mut::<crate::game::data::Catalogs>().loot = loot;
    level.visit(None);
    // An item put in before the first opening leaves the table to be drawn.
    let definition = ItemDefinition { id: ItemId(9), ..default() };
    let item =
        make_item(&definition, &mut ItemIds::default(), &mut GameRng::seeded(5), false, true);
    let deposited = item.id;
    level.inventory().hand.held = Some(item);
    // Putting it in answers the use without running the chest.
    assert_eq!(level.run("CHEST"), (false, vec![]));
    let chest = level.world.get::<Container>(level.device("CHEST")).unwrap();
    assert!(chest.table.is_some() && chest.contents.items.iter().any(|i| i.id == deposited));
    assert_eq!(level.run("CHEST"), (true, vec![TriggerEffect::Gold(5)]));
    assert!(level.world.get::<Container>(level.device("CHEST")).unwrap().table.is_none());
    let back = level.inventory().hand.held.take().map(|i| i.id);
    assert_eq!(back, Some(deposited), "its lone item comes to the hand");
    level.settle();
    assert_eq!(level.run("CHEST"), (true, vec![]), "its gold was taken");
    assert_eq!(level.run("LOST"), (true, vec![]));
    let lost = level.device("LOST");
    let lost = level.world.get::<Container>(lost).unwrap();
    assert!(lost.table.is_none() && lost.contents.items.is_empty(), "no contents are made up");
    assert_eq!(level.inventory().gold, 5);
    let memory = level.remember();
    level.visit(Some(memory));
    assert!(level.run("CHEST").1.is_empty(), "returning never rolls the gold again");
    level.visit(None);
    let chest = level.device("CHEST");
    level.world.entity_mut(chest).remove::<PropMotion>();
    assert!(!level.run("CHEST").0, "a missing prop cannot open a container");
    assert!(level.world.get::<Container>(chest).unwrap().table.is_some());
    assert_eq!(level.inventory().gold, 5);
}

#[test]
fn sounds_play_on_commit_only_when_marked_and_always_for_the_sound_action() {
    let sound = TriggerSound { path: "data/sound/click.wav".into(), volume: 1.0 };
    let mut quiet = trigger("QUIET", TriggerAction::Nothing);
    quiet.sound = Some(sound.clone());
    let mut loud = trigger("LOUD", TriggerAction::Nothing);
    loud.sound = Some(sound.clone());
    loud.sound_on_commit = true;
    let mut only = trigger("ONLY", TriggerAction::Sound);
    only.sound = Some(sound);
    let mut level = Scenario::new(vec![quiet, loud, only], &[], LevelScript::None);
    let click = TriggerEffect::Sound { path: "data/sound/click.wav".into(), volume: 1.0, at: None };
    assert_eq!(level.run("QUIET"), (true, vec![]));
    assert_eq!(level.run("LOUD"), (true, vec![click.clone()]));
    assert_eq!(level.run("ONLY"), (false, vec![click]), "plays without committing");
}

/// The upper monastery's bells, as authored.
fn bells() -> Scenario {
    let toggle = || TriggerAction::Animate(Animation::Toggle);
    let mut dial = trigger("BELLRINGSWITCH", TriggerAction::Dial);
    dial.usable = true;
    dial.state_change = Some(("RING".into(), StateChange::Set));
    dial.initial_state = Some(false);
    let mut button = trigger("BELL_BUTTON", toggle());
    button.usable = true;
    button.fire_linked = true;
    let mut windows = trigger("BELLTRIGGER01", TriggerAction::Nothing);
    windows.once = true;
    windows.fire_linked = true;
    windows.required_states = vec!["RING2".into()];
    let relay = |name: &str, ring: &str| {
        let mut relay = trigger(name, TriggerAction::Relay(-1));
        relay.fire_linked = true;
        relay.required_states = vec![ring.into()];
        relay
    };
    let shake = TriggerAction::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
    let mut triggers = vec![
        dial,
        button,
        windows,
        relay("BELLTRIGGERC", "RING0"),
        relay("BELL_ACTIVATE_2", "RING1"),
        relay("BELL_ACTIVATE_3", "RING2"),
        trigger("BELL_1", toggle()),
        trigger("BELL_2", toggle()),
        trigger("BELL_3", toggle()),
        trigger("GLASS1", toggle()),
        trigger("MONACAMSHAKE", shake),
    ];
    link(
        &mut triggers,
        &[
            (
                "BELL_BUTTON",
                &["BELLTRIGGER01", "BELLTRIGGERC", "BELL_ACTIVATE_2", "BELL_ACTIVATE_3"],
            ),
            ("BELLTRIGGER01", &["GLASS1", "MONACAMSHAKE"]),
            ("BELLTRIGGERC", &["BELL_1"]),
            ("BELL_ACTIVATE_2", &["BELL_2"]),
            ("BELL_ACTIVATE_3", &["BELL_3", "MONACAMSHAKE"]),
        ],
    );
    Scenario::new(
        triggers,
        &[
            PropSpec("BELLRINGSWITCH", 20.0, false, &[0, 1, 10, 20]),
            PropSpec("BELL_BUTTON", 11.0, false, &[]),
            PropSpec("BELL_1", 36.0, true, &[]),
            PropSpec("BELL_2", 36.0, true, &[]),
            PropSpec("BELL_3", 36.0, true, &[]),
            PropSpec("GLASS1", 2.0, false, &[]),
        ],
        LevelScript::Monastery2,
    )
}

#[test]
fn each_dial_position_rings_its_own_bell() {
    let mut level = bells();
    assert!(level.variable("RING0"));
    let (committed, effects) = level.run("BELL_BUTTON");
    assert!(committed);
    assert_eq!(effects, [TriggerEffect::BellRang(1)]);
    assert_eq!(level.motion("BELL_1").target, 36.0);
    assert_eq!(level.motion("BELL_2").target, 0.0);
    level.settle();
    assert!(level.run("BELLRINGSWITCH").0);
    assert!(!level.variable("RING0") && level.variable("RING1"));
    assert_eq!(
        (level.motion("BELLRINGSWITCH").frame, level.motion("BELLRINGSWITCH").target),
        (1.0, 10.0)
    );
    level.settle();
    assert_eq!(level.run("BELL_BUTTON").1, [TriggerEffect::BellRang(2)]);
    assert_eq!(level.motion("BELL_2").target, 36.0);
    assert_eq!(level.motion("GLASS1").target, 0.0, "the windows hold until the third bell");
}

#[test]
fn the_third_bell_breaks_the_windows_once_and_ends_the_puzzle() {
    let mut level = bells();
    level.run("BELLRINGSWITCH");
    level.settle();
    level.run("BELLRINGSWITCH");
    level.settle();
    let (committed, effects) = level.run("BELL_BUTTON");
    assert!(committed);
    assert_eq!(effects[0], TriggerEffect::BellRang(3));
    let shake = TriggerEffect::Shake { seconds: 2.4, intensity: 90.0, fade_out: true };
    assert!(effects.contains(&shake));
    assert_eq!(level.motion("GLASS1").target, 2.0);
    assert_eq!(level.motion("BELL_3").target, 36.0);
    level.settle();
    // Dial and button are off for good; the windows trigger has fired.
    assert_eq!(level.run("BELL_BUTTON"), (false, vec![]));
    assert_eq!(level.run("BELLRINGSWITCH"), (false, vec![]));
    let windows = level.device("BELLTRIGGER01");
    assert!(level.world.get::<TriggerState>(windows).unwrap().fired);
}

#[test]
fn a_swinging_bell_is_not_restarted() {
    let mut level = bells();
    level.run("BELL_BUTTON");
    for mut motion in level.world.query::<&mut PropMotion>().iter_mut(&mut level.world) {
        motion.frame = (motion.target / 2.0).floor();
    }
    // The button finished; the bell is halfway.
    let button = level.device("BELL_BUTTON");
    let mut motion = level.world.get_mut::<PropMotion>(button).unwrap();
    motion.frame = motion.target;
    let bell = level.motion("BELL_1").frame;
    assert_eq!(level.run("BELL_BUTTON").1, [TriggerEffect::BellRang(1)]);
    assert_eq!(level.motion("BELL_1").frame, bell);
    assert_eq!(level.motion("BELL_1").target, 36.0);
}

#[test]
fn walk_in_triggers_fire_on_entering_when_usable() {
    let teleport = |name: &str, x: f32, usable: bool| {
        let arrival = Arrival::Entrance { entrance: format!("{name}00"), ask: false };
        let mut trigger = trigger(name, TriggerAction::Teleport(arrival));
        trigger.usable = usable;
        trigger.place = Some(Vec3::new(x, 0.0, 0.0));
        trigger.range = (0.0, 2.0);
        trigger
    };
    let triggers = vec![
        teleport("START", 0.0, true),
        teleport("EXIT", 10.0, true),
        teleport("OFF", 20.0, false),
    ];
    let mut level = Scenario::new(triggers, &[], LevelScript::None);
    level.world.get_mut::<LevelState>(level.root).unwrap().arrive(Vec3::X * 5.0);
    let mut walk = |eye: Vec3| level.walk(eye, Duration::from_millis(100));
    // Standing in a trigger when the level is first seen does not fire it.
    assert!(walk(Vec3::ZERO).is_none());
    assert!(walk(Vec3::new(5.0, 0.0, 0.0)).is_none());
    assert!(matches!(
        walk(Vec3::new(10.5, 0.0, 0.0)),
        Some(PartyMove::Teleport(Arrival::Entrance { entrance, .. })) if entrance == "EXIT00"
    ));
    // Staying inside does not fire it again; a trigger the party may not
    // use never fires.
    assert!(walk(Vec3::new(10.0, 0.0, 0.0)).is_none());
    assert!(walk(Vec3::new(20.0, 0.0, 0.0)).is_none());
}

#[test]
fn damage_zones_hurt_each_member_while_stood_in() {
    let mut zone = trigger("ACID", TriggerAction::DamageZone);
    zone.usable = true;
    zone.place = Some(Vec3::ZERO);
    zone.range = (0.0, 2.0);
    let mut level = Scenario::new(vec![zone], &[], LevelScript::None);
    level.add_member("B");
    let full = level.world.get::<Vitals>(level.members[0]).unwrap().hp;
    let tick = Duration::from_millis(100);
    level.walk(Vec3::new(5.0, 0.0, 0.0), tick);
    level.walk(Vec3::ZERO, tick);
    for _ in 0..10 {
        level.walk(Vec3::ZERO, tick);
    }
    let zone_hits = level.damage().len();
    assert_eq!(zone_hits, 2 * 2, "two ticks for each of two members");
    level.walk(Vec3::new(5.0, 0.0, 0.0), tick);
    level.damage();
    level.walk(Vec3::new(5.0, 0.0, 0.0), Duration::from_secs(1));
    assert!(level.damage().is_empty());
    for &member in &level.members {
        level.world.get_mut::<Vitals>(member).unwrap().hp = full;
    }
    level.walk(Vec3::ZERO, tick);
    assert!(level.damage().is_empty(), "reentry starts a fresh hazard interval");
    level.walk(Vec3::ZERO, Duration::from_millis(500));
    assert_eq!(level.damage().len(), 2, "the restarted hazard hurts each member once");
    assert!(level.members.iter().all(|&m| level.world.get::<Vitals>(m).unwrap().hp < full));
}

#[test]
fn linked_hazards_tick_and_reset_in_authored_order() {
    let mut relay = trigger("RELAY", TriggerAction::Relay(-1));
    relay.usable = true;
    relay.place = Some(Vec3::ZERO);
    relay.range = (0.0, 2.0);
    let mut zone = trigger("ZONE", TriggerAction::DamageZone);
    zone.place = Some(Vec3::ZERO);
    zone.range = (0.0, 20.0);
    for zone_first in [false, true] {
        let mut definitions = if zone_first {
            vec![zone.clone(), relay.clone()]
        } else {
            vec![relay.clone(), zone.clone()]
        };
        link(&mut definitions, &[("RELAY", &["ZONE"])]);
        let mut level = Scenario::new(definitions, &[], LevelScript::None);
        level.walk(Vec3::X * 5.0, Duration::ZERO);
        level.walk(Vec3::ZERO, Duration::from_millis(500));
        let hurts = level.damage().len();
        assert_eq!(
            hurts,
            usize::from(!zone_first),
            "a later hazard sees its linked activation this step"
        );
    }
    let mut definitions = vec![relay, zone];
    link(&mut definitions, &[("RELAY", &["ZONE"])]);
    let mut level = Scenario::new(definitions, &[], LevelScript::None);
    level.walk(Vec3::X * 5.0, Duration::ZERO);
    level.run("ZONE");
    level.walk(Vec3::X * 5.0, Duration::from_millis(400));
    level.walk(Vec3::ZERO, Duration::from_millis(100));
    assert!(level.damage().is_empty(), "the earlier relay resets the hazard before it ticks");
}

#[test]
fn environmental_damage_finishes_a_waiting_battle_before_returning() {
    let mut level = Scenario::new(
        vec![trigger("LETHAL", TriggerAction::HurtParty(Dice { base: 10000, ..default() }))],
        &[],
        LevelScript::None,
    );
    level.world.spawn(crate::game::combat::Battle::default());
    let member = level.members[0];
    assert!(level.formation().row_of(member).is_some());
    assert!(level.run("LETHAL").0);
    // Death bookkeeping is synchronous: the seat is gone at once.
    assert!(level.world.get::<Vitals>(member).unwrap().is_dead());
    assert!(level.formation().row_of(member).is_none());
    assert!(
        level.world.query::<&crate::game::combat::Battle>().iter(&level.world).next().is_none()
    );
    assert_eq!(
        level
            .world
            .resource_mut::<Messages<crate::game::combat::CombatNews>>()
            .drain()
            .collect::<Vec<_>>(),
        [crate::game::combat::CombatNews::Lost]
    );
}

#[test]
fn fountains_restore_until_dry_and_refill_with_game_time() {
    let fountain = Fountain {
        restores: Restores::Health,
        amount: 5,
        uses: Some(1),
        refills_after: Some(2 * 720 * 1000),
    };
    let mut level = Scenario::new(
        vec![trigger("WELL", TriggerAction::Fountain(fountain))],
        &[],
        LevelScript::None,
    );
    let full = level.world.get::<Vitals>(level.members[0]).unwrap().hp;
    level.world.get_mut::<Vitals>(level.members[0]).unwrap().hp = 1;
    assert!(level.run("WELL").0);
    assert!(!level.run("WELL").0, "dry after its one use");
    let memory = level.remember();
    level.visit(Some(memory));
    assert!(!level.run("WELL").0, "still dry after returning");
    level.world.resource_mut::<GameClock>().ms = 2 * 720 * 1000;
    assert!(level.run("WELL").0);
    assert_eq!(level.world.get::<Vitals>(level.members[0]).unwrap().hp, 11.min(full));
}

#[test]
fn relay_recipients_complete_in_order_before_the_senders_commit() {
    let mut first = trigger("FIRST", TriggerAction::Nothing);
    first.state_change = Some(("rEaDy".into(), StateChange::Set));
    first.initial.message = Some(1);
    let mut second = trigger("SECOND", TriggerAction::Nothing);
    second.required_states.push("READY".into());
    second.initial.message = Some(2);
    let mut relay = trigger("RELAY", TriggerAction::Relay(-1));
    relay.recipients = vec![0, 1];
    relay.initial.message = Some(3);
    let mut level = Scenario::new(vec![first, second, relay], &[], LevelScript::None);
    assert_eq!(
        level.run("RELAY"),
        (
            true,
            vec![
                TriggerEffect::Message { source: None, id: 1 },
                TriggerEffect::Message { source: None, id: 2 },
                TriggerEffect::Message { source: None, id: 3 }
            ]
        )
    );
    let memory = level.remember();
    level.visit(Some(memory));
    assert_eq!(level.run("SECOND").1, [TriggerEffect::Message { source: None, id: 2 }]);
}

#[test]
fn damage_uses_roster_order_and_leaves_nonmembers_alone() {
    let dice = Dice { count: 2, sides: 4, base: 0 };
    let mut level = Scenario::new(
        vec![trigger("TRAP", TriggerAction::HurtParty(dice))],
        &[],
        LevelScript::None,
    );
    let profile = Character::new("NPC", Race::Human, Gender::Male, Profession::Fighter);
    let outsider = level.world.spawn((Vitals::new(&profile), profile)).id();
    let outsider_hp = level.world.get::<Vitals>(outsider).unwrap().hp;
    let second = level.add_member("B");
    let first = level.members[0];
    crate::game::party::roster::test_reorder(&mut level.world, &[second, first]);
    let before = [second, first].map(|e| level.world.get::<Vitals>(e).unwrap().hp);
    level.run("TRAP");
    let hits: Vec<_> = level.damage().into_iter().map(|hit| (hit.member, hit.amount)).collect();
    assert_eq!(hits.iter().map(|&(member, _)| member).collect::<Vec<_>>(), [second, first]);
    for ((member, amount), hp) in hits.into_iter().zip(before) {
        assert!((2..=8).contains(&amount));
        assert_eq!(level.world.get::<Vitals>(member).unwrap().hp, hp - amount);
    }
    assert_eq!(level.world.get::<Vitals>(outsider).unwrap().hp, outsider_hp);
}

#[test]
fn departed_devices_cannot_run() {
    let mut door = trigger("DOOR", TriggerAction::OpenDoor);
    door.alternate = Some(act(TriggerAction::ToggleDoor));
    let mut chest = trigger("CHEST", TriggerAction::Animate(Animation::Toggle));
    chest.loot_table = Some("Loot".into());
    let mut level = Scenario::new(
        vec![door, chest],
        &[PropSpec("DOOR", 10.0, false, &[]), PropSpec("CHEST", 5.0, false, &[])],
        LevelScript::None,
    );
    assert!(level.run("DOOR").0);
    level.settle();
    let source = level.world.entity_mut(level.root).take::<Level>().unwrap();
    assert!(!level.run("DOOR").0);
    level.world.entity_mut(level.root).insert(source);
    assert!(level.run("DOOR").0);
    assert_eq!(level.motion("DOOR").target, 0.0);
}

#[test]
fn the_production_post_step_applies_trigger_commands_before_moving() {
    use crate::game::party::{
        PartyPlugin,
        body::{EYE_ABOVE_CENTRE, PartyBody},
    };
    use crate::game::world::level::Travel;
    use avian3d::prelude::Position;
    let mut portal =
        trigger("PORTAL", TriggerAction::Teleport(Arrival::Here { eye: Vec3::X * 10.0, yaw: 0.0 }));
    portal.usable = true;
    portal.place = Some(Vec3::ZERO);
    portal.range = (0.0, 2.0);
    let mut destination = portal.clone();
    destination.name = "DESTINATION".into();
    destination.place = Some(Vec3::X * 10.0);
    destination.initial.action =
        TriggerAction::Teleport(Arrival::Here { eye: Vec3::X * 20.0, yaw: 0.0 });
    let geometry = LevelDefinition { triggers: vec![portal, destination], ..default() };
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::input::InputPlugin,
        bevy_enhanced_input::EnhancedInputPlugin,
        PartyPlugin,
    ))
    .init_asset::<LevelDefinition>()
    .add_message::<Travel>();
    let world = app.world_mut();
    let handle = world.resource_mut::<Assets<LevelDefinition>>().add(geometry.clone());
    let root = world.spawn(Level::test(handle)).id();
    let mut spawning = SystemState::<Commands>::new(world);
    spawn_devices(
        &mut spawning.get_mut(world).unwrap(),
        root,
        &geometry,
        &[],
        LevelScript::None,
        &LootTables::default(),
        None,
    );
    spawning.apply(world);
    let party = world
        .spawn((
            PartyBody::default(),
            Position(Vec3::X * 5.0 - Vec3::Y * EYE_ABOVE_CENTRE),
            Transform::default(),
        ))
        .id();
    // Exercise the real registered command/arrival boundary, not physics integration.
    app.world_mut().run_schedule(FixedPostUpdate);
    app.world_mut().get_mut::<Position>(party).unwrap().0 = -Vec3::Y * EYE_ABOVE_CENTRE;
    app.world_mut().run_schedule(FixedPostUpdate);
    assert_eq!(
        app.world().get::<Position>(party).unwrap().0,
        Vec3::X * 10.0 - Vec3::Y * EYE_ABOVE_CENTRE
    );
    assert!(app.world_mut().query::<&PendingPartyMove>().iter(app.world()).next().is_none());
    app.world_mut().run_schedule(FixedPostUpdate);
    assert_eq!(
        PartyBody::eye(app.world().get::<Position>(party).unwrap()),
        Vec3::X * 10.0,
        "teleport arrival does not activate the destination portal"
    );
    app.world_mut().get_mut::<Position>(party).unwrap().0 =
        Vec3::X * 5.0 - Vec3::Y * EYE_ABOVE_CENTRE;
    app.world_mut().run_schedule(FixedPostUpdate);
    app.world_mut().get_mut::<Position>(party).unwrap().0 =
        Vec3::X * 10.0 - Vec3::Y * EYE_ABOVE_CENTRE;
    app.world_mut().run_schedule(FixedPostUpdate);
    assert_eq!(
        PartyBody::eye(app.world().get::<Position>(party).unwrap()),
        Vec3::X * 20.0,
        "leaving and reentering activates the destination portal normally"
    );
}

#[test]
fn same_named_props_keep_their_own_device_state_and_animation() {
    let mut level = Scenario::new(
        vec![trigger("FIRST", TriggerAction::OpenDoor), trigger("SECOND", TriggerAction::OpenDoor)],
        &[PropSpec("FIRST", 10.0, false, &[]), PropSpec("SECOND", 20.0, false, &[])],
        LevelScript::None,
    );
    for definition in &mut level.geometry.triggers {
        definition.sound = Some(TriggerSound { path: definition.name.clone(), volume: 1.0 });
        definition.sound_on_commit = true;
        definition.name = "SAME".into();
    }
    level.geometry.props[1].parts[0].transform.translation = Vec3::X;
    level.visit(None);
    let [first, second] = level.devices[..] else { panic!("two devices") };
    assert!(level.world.get::<Prop>(first).is_some());
    assert!(activate(&mut level.world, first, false));
    assert_eq!(
        level.effects(),
        [TriggerEffect::Sound { path: "FIRST".into(), volume: 1.0, at: Some(Vec3::ZERO) }]
    );
    assert_eq!(level.world.get::<PropMotion>(first).unwrap().target, 10.0);
    assert_eq!(level.world.get::<PropMotion>(second).unwrap().target, 0.0);
    assert!(!level.world.get::<TriggerState>(second).unwrap().fired);
    assert!(activate(&mut level.world, second, false));
    assert_eq!(
        level.effects(),
        [TriggerEffect::Sound { path: "SECOND".into(), volume: 1.0, at: Some(Vec3::X) }]
    );
    assert_eq!(level.world.get::<PropMotion>(second).unwrap().target, 20.0);
    let memory = level.remember();
    level.visit(Some(memory));
    for old in [first, second] {
        assert!(level.world.get_entity(old).is_err());
    }
    for &device in &level.devices {
        assert!(level.world.get::<DoorState>(device).unwrap().open);
    }
}
/// A level whose trigger, by any of its uses, would fly a camera path
/// without keys is refused before it is entered; paths with keys pass.
#[test]
fn camera_paths_without_keys_refuse_the_level() {
    use crate::content::level::CameraPathModel;
    let paths = [0, 1, 3].map(|keys| CameraPathModel {
        name: format!("PATH{keys}"),
        keys: (0..keys).map(|x| Transform::from_xyz(x as f32, 0.0, 0.0)).collect(),
        keys_per_second: 1.0,
    });
    let level = |trigger: TriggerDefinition| LevelDefinition {
        triggers: vec![trigger],
        camera_paths: paths.clone().into(),
        ..default()
    };
    for path in [1, 2] {
        assert!(check(&level(trigger("FLY", TriggerAction::CameraPath(path)))).is_ok());
    }
    assert!(check(&level(trigger("EMPTY", TriggerAction::CameraPath(0)))).is_err());
    assert!(check(&level(trigger("MISSING", TriggerAction::CameraPath(3)))).is_err());
    let unnamed = TriggerAction::MissingCameraPath("NOWHERE".into());
    let why = check(&level(trigger("UNNAMED", unnamed))).unwrap_err();
    assert!(why.contains("UNNAMED") && why.contains("NOWHERE"), "{why}");
    // An empty path behind the first use is refused too.
    let mut alternate = trigger("LATER", TriggerAction::CameraPath(1));
    alternate.alternate = Some(act(TriggerAction::CameraPath(0)));
    assert!(check(&level(alternate)).is_err());
}

#[test]
fn a_pending_move_cannot_be_applied_to_a_later_visit() {
    let arrival = Arrival::Here { eye: Vec3::X, yaw: 0.0 };
    let mut level = Scenario::new(
        vec![trigger("PORTAL", TriggerAction::Teleport(arrival))],
        &[],
        LevelScript::None,
    );
    assert!(level.run("PORTAL").0);
    level.visit(None);
    assert!(level.world.query::<&PendingPartyMove>().iter(&level.world).next().is_none());
    assert!(level.run("PORTAL").0);
    assert!(level.world.get::<PendingPartyMove>(level.root).is_some());
}

#[test]
fn a_refused_take_leaves_container_receiver_and_identities_untouched() {
    use super::super::containers::{Recipient, take_from_container};
    use crate::content::items::QuantityKind;
    use crate::game::inventory::{Carried, Refusal};
    let mut level = chest_level(None, 0);
    let mut arrows = ItemDefinition { id: ItemId(0), name: "arrow".into(), ..default() };
    arrows.quantity = QuantityKind::Stack;
    arrows.stack_limit = 99;
    let dagger = ItemDefinition { id: ItemId(1), name: "dagger".into(), ..default() };
    let catalog = ItemCatalog(vec![arrows.clone(), dagger.clone()]);
    let chest = level.device("CHEST");
    let quiver = {
        let mut ids = level.world.resource_mut::<ItemIds>();
        ItemStack::new(&arrows, &mut ids, 10, true)
    };
    let mut second = quiver.clone();
    second.id = level.world.resource_mut::<ItemIds>().allocate();
    let mut container = level.world.get_mut::<Container>(chest).unwrap();
    container.table = None;
    container.contents.items = vec![quiver.clone(), second];
    assert!(level.run("CHEST").0);
    let quiver = quiver.id;
    let contents = |level: &Scenario| {
        level.world.get::<Container>(level.device("CHEST")).unwrap().contents.items.clone()
    };
    let next_id = |level: &Scenario| level.world.resource::<ItemIds>().clone().allocate();
    let before = contents(&level);
    level.world.resource_mut::<crate::game::data::Catalogs>().items = catalog;

    // A member with no room at all: nothing moves and no identity is used.
    let member = level.members[0];
    let mut carried = Carried::default();
    {
        let catalog = ItemCatalog(vec![arrows.clone(), dagger.clone()]);
        let mut ids = level.world.resource_mut::<ItemIds>();
        for _ in 0..7 {
            assert!(
                carried.receive(ItemStack::new(&dagger, &mut ids, 1, true), &catalog).is_none()
            );
        }
        assert!(carried.receive(ItemStack::new(&arrows, &mut ids, 95, true), &catalog).is_none());
    }
    let full = carried.clone();
    level.world.entity_mut(member).insert(carried.clone());
    // Fill the last arrows too, so only a new stack could take them.
    let ids_before = next_id(&level);
    let mut stuffed = full.clone();
    {
        let catalog = ItemCatalog(vec![arrows.clone(), dagger.clone()]);
        let mut ids = level.world.resource::<ItemIds>().clone();
        stuffed.receive(ItemStack::new(&arrows, &mut ids, 4, true), &catalog);
    }
    level.world.entity_mut(member).insert(stuffed.clone());
    let on = attempt(&level);
    assert_eq!(
        take_from_container(&mut level.world, on, &[(quiver, 4)], Recipient::Member(member)),
        Err(Refusal::NoRoom),
        "a take that moves nothing is refused"
    );
    // Two picks of one stack would each be checked against its whole count.
    assert_eq!(
        take_from_container(&mut level.world, on, &[(quiver, 6), (quiver, 6)], Recipient::Party),
        Err(Refusal::Nothing)
    );
    assert_eq!(contents(&level), before);
    assert_eq!(level.world.get::<Carried>(member), Some(&stuffed));
    assert_eq!(next_id(&level), ids_before);

    // Room only to top up a stack: units merge, the rest stays, no new identity.
    level.world.entity_mut(member).insert(full);
    let on = attempt(&level);
    take_from_container(&mut level.world, on, &[(quiver, 6)], Recipient::Member(member)).unwrap();
    assert_eq!(contents(&level).iter().map(ItemStack::count).collect::<Vec<_>>(), [6, 10]);
    assert_eq!(contents(&level)[0].id, quiver);
    assert_eq!(next_id(&level), ids_before);

    // A part kept as a stack of its own takes exactly one new identity.
    let on = attempt(&level);
    take_from_container(&mut level.world, on, &[(quiver, 2)], Recipient::Party).unwrap();
    let pool = level.inventory().pool.stacks.clone();
    assert_eq!(pool.iter().map(|s| (s.id, s.count())).collect::<Vec<_>>(), [(ids_before, 2)]);
    assert_eq!(contents(&level)[0].count(), 4);
}

/// Retail `SelectAction` advances the sequence before the action runs, and
/// `Run` returns from an action that cannot run without undoing it; only
/// the commit fires, sets variables and shows messages.
#[test]
fn a_declined_action_advances_the_sequence_but_commits_nothing() {
    for action in [TriggerAction::OpenDoor, TriggerAction::NoRuntimeAction] {
        let mut lever = trigger("LEVER", action);
        lever.initial.message = Some(1);
        lever.alternate = Some(TriggerUse { action: TriggerAction::Nothing, message: Some(2) });
        lever.state_change = Some(("PULLED".into(), StateChange::Set));
        let mut level = Scenario::new(vec![lever], &[], LevelScript::None);
        assert_eq!(level.run("LEVER"), (false, vec![]));
        let device = level.device("LEVER");
        let state = level.world.get::<TriggerState>(device).unwrap();
        assert!(state.use_alternate && !state.fired);
        assert!(!level.variable("PULLED"));
        // The retry runs the alternate, which commits.
        assert_eq!(
            level.run("LEVER"),
            (true, vec![TriggerEffect::Message { source: None, id: 2 }])
        );
        assert!(level.variable("PULLED"));
        assert!(level.world.get::<TriggerState>(device).unwrap().fired);
    }
}

#[test]
fn a_door_waits_for_the_doors_it_runs_before_its_key_is_tried() {
    let key = ItemId(7);
    let mut door = trigger("DOOR", TriggerAction::OpenDoor);
    door.door = Some(DoorLock { locked: true, key: Some(key) });
    door.fire_linked = true;
    let gate = trigger("GATE", TriggerAction::OpenDoor);
    let mut triggers = vec![door, gate];
    link(&mut triggers, &[("DOOR", &["GATE"])]);
    let mut level = Scenario::new(
        triggers,
        &[PropSpec("DOOR", 15.0, false, &[]), PropSpec("GATE", 10.0, false, &[])],
        LevelScript::None,
    );
    let definition = ItemDefinition { id: key, ..default() };
    let item =
        make_item(&definition, &mut ItemIds::default(), &mut GameRng::seeded(5), false, true);
    level.inventory().hand.held = Some(item);
    assert!(level.run("GATE").0);
    // The gate is still opening: the door does not take the key.
    assert!(!level.run("DOOR").0);
    assert!(level.world.get::<DoorState>(level.device("DOOR")).unwrap().locked);
    assert_eq!(level.motion("DOOR").target, 0.0);
    level.settle();
    // Once it has settled, the key opens the door, which runs the gate.
    assert!(level.run("DOOR").0);
    assert!(!level.world.get::<DoorState>(level.device("DOOR")).unwrap().locked);
    assert_eq!(level.motion("DOOR").target, 15.0);
}

#[test]
fn player_reach_uses_only_prop_colliders_then_one_centre_fallback() {
    use crate::game::party::body::PartyBody;
    use crate::game::world::props::PropPart;
    use avian3d::prelude::{Collider, Position};
    let mut world = World::new();
    let position = Position::default();
    let eye = PartyBody::eye(&position);
    world.spawn((PartyBody::default(), position));
    let geometry = LevelDefinition {
        props: vec![PropModel {
            name: "PROP".into(),
            collidable: true,
            face_camera: false,
            frames_per_second: 0.0,
            parts: vec![crate::content::level::PropModelPart {
                transform: Transform::from_translation(eye + Vec3::X * 20.0),
                keys: vec![],
                collision: default(),
            }],
            wraps: false,
            slots: vec![],
            trigger: None,
        }],
        ..default()
    };
    let prop = world.spawn(Prop { definition: 0 }).id();
    let surface = world
        .spawn((
            PropPart { part: 0 },
            Transform::from_translation(eye + Vec3::X * 10.0),
            Collider::sphere(1.0),
            ChildOf(prop),
        ))
        .id();
    // Neither a colliderless part nor a helper collider is gameplay reach geometry.
    world.spawn((PropPart { part: 1 }, Transform::from_translation(eye), ChildOf(prop)));
    world.spawn((Collider::sphere(1.0), Transform::from_translation(eye), ChildOf(prop)));
    assert!((player_distance(&mut world, &geometry, prop).unwrap() - 9.0).abs() < 0.001);
    world.entity_mut(surface).remove::<Collider>();
    assert!((player_distance(&mut world, &geometry, prop).unwrap() - 20.0).abs() < 0.001);
}

#[test]
fn missing_actions_preserve_selection_and_keys_until_supported() {
    let mut lever = trigger("MISSING", TriggerAction::Unsupported(0x7f));
    lever.alternate = Some(act(TriggerAction::Nothing));
    lever.door = Some(DoorLock { locked: true, key: Some(ItemId(7)) });
    let mut level = Scenario::new(vec![lever], &[], LevelScript::None);
    let item = make_item(
        &ItemDefinition { id: ItemId(7), ..default() },
        &mut ItemIds::default(),
        &mut GameRng::seeded(5),
        false,
        true,
    );
    level.inventory().hand.held = Some(item);
    for _ in 0..2 {
        assert_eq!(
            level.run("MISSING"),
            (false, vec![TriggerEffect::Unsupported { device: "MISSING".into(), code: 0x7f }])
        );
        let entity = level.device("MISSING");
        let state = level.world.get::<TriggerState>(entity).unwrap();
        assert!(!state.fired && !state.use_alternate);
        assert!(level.world.get::<DoorState>(entity).unwrap().locked);
    }
}

#[test]
fn linked_device_switches_use_the_persisted_device_state() {
    let mut definitions = vec![
        trigger("OFF", TriggerAction::SetDevices(StateChange::Clear)),
        trigger("ON", TriggerAction::SetDevices(StateChange::Set)),
        trigger("TOGGLE", TriggerAction::SetDevices(StateChange::Toggle)),
        trigger("TARGET", TriggerAction::Nothing),
    ];
    link(&mut definitions, &[("OFF", &["TARGET"]), ("ON", &["TARGET"]), ("TOGGLE", &["TARGET"])]);
    let mut level = Scenario::new(definitions, &[], LevelScript::None);
    assert!(level.run("OFF").0);
    assert!(!level.run("TARGET").0);
    let memory = level.remember();
    level.visit(Some(memory));
    assert!(!level.run("TARGET").0);
    assert!(level.run("ON").0);
    assert!(level.run("TARGET").0);
    assert!(level.run("TOGGLE").0);
    assert!(!level.run("TARGET").0);
}

#[test]
fn mana_fountains_share_use_and_refill_history_and_restore_deficits() {
    use crate::content::characters::Realm;
    use crate::game::magic::SpellPoints;
    let mut scene = Scenario::new(
        vec![trigger(
            "MANA",
            TriggerAction::Fountain(Fountain {
                restores: Restores::Magic,
                amount: 3,
                uses: Some(1),
                refills_after: Some(1000),
            }),
        )],
        &[],
        LevelScript::None,
    );
    let member = scene.members[0];
    let mut points = SpellPoints::default();
    use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
    use crate::game::magic::LearnedSpells;
    let spells = SpellCatalog(vec![
        SpellDefinition::default(),
        SpellDefinition { id: SpellId(1), realm: Some(Realm::Fire), cost: 8, ..default() },
        SpellDefinition { id: SpellId(2), realm: Some(Realm::Water), cost: 6, ..default() },
    ]);
    scene.world.resource_mut::<crate::game::data::Catalogs>().spells = spells;
    scene.world.entity_mut(member).insert(LearnedSpells([SpellId(1), SpellId(2)].into()));
    points.fraction[Realm::Fire] = 0.25;
    scene.world.entity_mut(member).insert(points);
    for realm in enumset::EnumSet::<Realm>::all() {
        crate::game::magic::test_set_points(&mut scene.world, member, realm, 0);
    }
    assert!(scene.run("MANA").0);
    let points = scene.world.get::<SpellPoints>(member).unwrap();
    assert_eq!(
        enumset::EnumSet::<Realm>::all()
            .iter()
            .map(|r| crate::game::magic::test_points(&scene.world, member, r))
            .sum::<i64>(),
        3
    );
    assert_eq!(points.fraction[Realm::Fire], 0.25);
    let memory = scene.remember();
    scene.visit(Some(memory));
    assert!(!scene.run("MANA").0);
    scene.world.resource_mut::<GameClock>().ms = 1000;
    assert!(scene.run("MANA").0);
    assert_eq!(
        enumset::EnumSet::<Realm>::all()
            .iter()
            .map(|r| crate::game::magic::test_points(&scene.world, member, r))
            .sum::<i64>(),
        6
    );
}

#[test]
fn restorative_fountains_require_spell_content_before_consuming_a_use() {
    use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
    use crate::game::characters::{Condition, Lasting};
    let mut scene = Scenario::new(
        vec![trigger(
            "TEMPLE",
            TriggerAction::Fountain(Fountain {
                restores: Restores::Restoration,
                amount: 15,
                uses: Some(1),
                refills_after: None,
            }),
        )],
        &[],
        LevelScript::None,
    );
    let member = scene.members[0];
    {
        let mut vitals = scene.world.get_mut::<Vitals>(member).unwrap();
        vitals.hp = 1;
        vitals.inflict(Condition::Poisoned, Lasting::Rounds(10));
        vitals.inflict(Condition::Infatuated, Lasting::Indefinite);
    }
    assert!(!scene.run("TEMPLE").0);
    assert!(scene.world.get::<FountainState>(scene.device("TEMPLE")).unwrap().used.is_none());
    let mut definitions = vec![SpellDefinition::default(); 69];
    definitions[68] = SpellDefinition {
        id: SpellId::HEAL_ALL,
        dice: Some(Dice { count: 1, sides: 1, base: 0 }),
        ..default()
    };
    let spells = SpellCatalog(definitions);
    scene.world.resource_mut::<crate::game::data::Catalogs>().spells = spells;
    assert!(scene.run("TEMPLE").0);
    let vitals = scene.world.get::<Vitals>(member).unwrap();
    assert_eq!(vitals.hp, 2, "out-of-range authored power becomes one, not seven");
    assert_eq!(
        vitals.conditions,
        crate::game::characters::Conditions::from([(Condition::Infatuated, Lasting::Indefinite)])
    );
    assert!(!scene.run("TEMPLE").0);
}

#[test]
fn random_teleports_choose_at_execution_and_disabled_devices_do_not_draw() {
    let destinations: Vec<_> = [10.0, 20.0, 30.0]
        .into_iter()
        .map(|x| Arrival::Here { eye: Vec3::X * x, yaw: 0.0 })
        .collect();
    let mut scene = Scenario::new(
        vec![trigger("RANDOM", TriggerAction::RandomTeleport(destinations.clone()))],
        &[],
        LevelScript::None,
    );
    let mut expected = scene.world.resource::<GameRng>().clone();
    let index = expected.below(3) as usize;
    assert!(scene.run("RANDOM").0);
    let pending = scene.world.entity_mut(scene.root).take::<PendingPartyMove>().unwrap();
    assert!(matches!(pending.0, PartyMove::Teleport(ref actual) if actual == &destinations[index]));
    let entity = scene.device("RANDOM");
    scene.world.get_mut::<TriggerState>(entity).unwrap().on = false;
    assert!(!scene.run("RANDOM").0);
    assert_eq!(scene.world.resource::<GameRng>().clone().below(1000), expected.below(1000));
}

#[test]
fn named_ambient_sound_switches_survive_revisit() {
    let mut scene = Scenario::new(
        vec![
            trigger(
                "STOP",
                TriggerAction::AmbientSounds {
                    names: vec!["SoundHum".into()],
                    change: StateChange::Clear,
                },
            ),
            trigger(
                "TOGGLE",
                TriggerAction::AmbientSounds {
                    names: vec!["SOUNDHUM".into()],
                    change: StateChange::Toggle,
                },
            ),
        ],
        &[],
        LevelScript::None,
    );
    assert!(scene.run("STOP").0);
    let memory = scene.remember();
    scene.visit(Some(memory));
    assert!(!scene.world.get::<LevelState>(scene.root).unwrap().ambient_sound_enabled("soundhum"));
    assert!(scene.run("TOGGLE").0);
    assert!(scene.world.get::<LevelState>(scene.root).unwrap().ambient_sound_enabled("SoundHum"));
}

#[test]
fn light_switches_use_authored_initial_state_and_survive_revisit() {
    let mut scene = Scenario::new(
        vec![trigger(
            "TOGGLE",
            TriggerAction::Lights {
                sources: vec![("LASER".into(), false)],
                change: StateChange::Toggle,
            },
        )],
        &[],
        LevelScript::None,
    );
    assert!(!scene.world.get::<LightActivation>(scene.root).unwrap().light_enabled("laser", false));
    assert!(scene.run("TOGGLE").0);
    let memory = scene.remember();
    scene.visit(Some(memory));
    assert!(scene.world.get::<LightActivation>(scene.root).unwrap().light_enabled("laser", false));
    assert!(scene.run("TOGGLE").0);
    assert!(!scene.world.get::<LightActivation>(scene.root).unwrap().light_enabled("laser", false));
}

#[test]
fn camera_sway_obeys_signed_relay_calls_without_committing_and_survives_revisit() {
    let mut on = trigger("ON", TriggerAction::Relay(1));
    on.recipients = vec![2];
    let mut off = trigger("OFF", TriggerAction::Relay(-1));
    off.recipients = vec![2];
    let mut sway = trigger("SWAY", TriggerAction::CameraSway);
    sway.fire_linked = true;
    sway.recipients = vec![3];
    sway.state_change = Some(("UNCOMMITTED".into(), StateChange::Set));
    let linked = trigger("LINKED", TriggerAction::Nothing);
    let mut level = Scenario::new(vec![on, off, sway, linked], &[], LevelScript::None);
    assert!(!level.world.get::<LevelState>(level.root).unwrap().camera_sway);
    assert!(level.run("ON").0);
    assert!(level.world.get::<LevelState>(level.root).unwrap().camera_sway);
    assert!(!level.variable("UNCOMMITTED"));
    assert!(!level.world.get::<TriggerState>(level.device("SWAY")).unwrap().fired);
    assert!(!level.world.get::<TriggerState>(level.device("LINKED")).unwrap().fired);
    let memory = level.remember();
    level.visit(Some(memory));
    assert!(level.world.get::<LevelState>(level.root).unwrap().camera_sway);
    assert!(level.run("OFF").0);
    assert!(!level.world.get::<LevelState>(level.root).unwrap().camera_sway);
    level.geometry.camera_sway = true;
    level.visit(None);
    assert!(level.world.get::<LevelState>(level.root).unwrap().camera_sway);
    // A direct player/walk call uses -1, independently of the definition's data.
    assert!(!level.run("SWAY").0);
    assert!(!level.world.get::<LevelState>(level.root).unwrap().camera_sway);
}
