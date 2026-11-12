//! People who join the party and leave it. One who can join is, in the
//! party, a character like any other: their companion self, made with them
//! and set aside until they take a seat.
use super::{ActorId, CompanionOf, Embodies, Npc, People, Shape, Whereabouts};
use crate::{
    content::{items::ItemId, npcs::NpcCharacter},
    game::{
        actors::{CanFight, Nature},
        characters::{Calling, Character, Conditions, Dead, Vitals},
        data::Catalogs,
        inventory::EquipSlot,
        items::{Cursed, Give, Items, Place, Put, make, roll_amount},
        magic::{LearnedSpells, SpellPoints},
        party::{
            body::PartyBody,
            roster::{PartyMember, PartyRoster},
        },
        rng::GameRng,
        world::{
            level::Level,
            navigation::{LevelNavigation, WalkerMotionConfig},
            population::{Monster, Newborn},
        },
    },
};
use avian3d::prelude::Position;
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use enum_map::Enum;
use enumset::EnumSet;

/// The character a person is in the party, made with them: their sheet,
/// health, spells and spell points, wearing and packing what is authored
/// for them, each in the slot it is authored for. Set aside until they
/// join.
pub fn companion(
    commands: &mut Commands,
    person: Entity,
    t: &NpcCharacter,
    content: &Catalogs,
    rng: &mut GameRng,
) {
    let mut c = Character::new(&t.name, t.race, t.gender, t.profession);
    c.abilities.0 = t.attributes;
    c.training.0 = t.skills;
    c.calling = Calling::begun(t.profession);
    while c.calling.level() < t.level {
        c.experience.last_goal = c.calling.experience_goal(&c.experience);
        c.calling.levels[t.profession] += 1;
    }
    c.experience.earned = c.experience.last_goal;
    c.identity.portrait = crate::game::characters::creation::identity::PORTRAITS
        .get(t.portrait)
        .copied()
        .unwrap_or_default()
        .into();
    let skills = crate::game::actors::Skills::of_sheet(&c);
    let spells = LearnedSpells(
        t.spells
            .iter()
            .copied()
            .filter(|&s| {
                crate::game::magic::can_learn(
                    (c.trained(), &c.training),
                    &skills,
                    &content.spells[s],
                )
            })
            .collect(),
    );
    let vitals = Vitals::new(&c);
    let member = commands
        .spawn((
            Name::new(c.identity.name.clone()),
            CompanionOf(person),
            Nature::of_race(t.race),
            c,
            vitals,
            spells,
            SpellPoints::default(),
            Disabled,
        ))
        .id();
    let mut made = |item: &ItemId, at: Place, rng: &mut GameRng| {
        let definition = &content.items[*item];
        let item = make(commands, definition, roll_amount(definition, rng, true), true);
        commands.trigger(Put { item, at });
    };
    for (slot, item) in t.equipment.iter().enumerate() {
        if let Some(item) = item {
            made(item, Place::Slot { member, slot: EquipSlot::from_usize(slot) }, rng);
        }
    }
    for (slot, item) in t.backpack.iter().enumerate() {
        if let Some(item) = item {
            made(item, Place::Pack { member, slot }, rng);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinRefusal {
    NoRoom,
    Declines,
}
pub fn install(app: &mut App) {
    app.add_systems(Update, retire_fallen);
}
/// Who may join the party, and their joining it.
#[derive(SystemParam)]
pub struct Recruiter<'w, 's> {
    persons: Query<'w, 's, (&'static ActorId, &'static Whereabouts), With<Npc>>,
    people: People<'w, 's>,
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    members: Query<'w, 's, (&'static Calling, Has<CanFight>, Has<CompanionOf>)>,
    bodies: Query<'w, 's, &'static Conditions, With<Monster>>,
    level: Single<'w, 's, &'static Level>,
    content: Res<'w, Catalogs>,
    story: Res<'w, super::story::StoryFacts>,
    commands: Commands<'w, 's>,
}
impl Recruiter<'_, '_> {
    /// Someone asked to join does, or says why not: the party has room for
    /// two such; they keep to a post in a region they serve, wait on what
    /// the story owes them, and want a party of their level.
    pub fn admit(&mut self, person: Entity) -> Result<(), JoinRefusal> {
        let Self { persons, people, party, members, bodies, level, content, story, commands } =
            self;
        let (party_id, roster) = **party;
        let (id, whereabouts) = persons.get(person).map_err(|_| JoinRefusal::Declines)?;
        let companion = people.companion(person).ok_or(JoinRefusal::Declines)?;
        if *whereabouts == Whereabouts::Killed || roster.contains(&companion) {
            return Err(JoinRefusal::Declines);
        }
        let occupied = roster.iter().filter(|&e| members.get(e).is_ok_and(|r| r.2)).count();
        if occupied >= 2 {
            return Err(JoinRefusal::NoRoom);
        }
        let d = &content.npcs[id.0];
        let region = crate::content::levels::LevelInfo::for_path(&level.path).map_or(0, |l| l.band);
        let (sum, count) = roster
            .iter()
            .filter_map(|e| members.get(e).ok())
            .filter(|(_, can_fight, _)| *can_fight)
            .fold((0u32, 0u32), |(sum, count), (c, _, _)| (sum + u32::from(c.level()), count + 1));
        let waiting = d.joins_when.is_some_and(|(fact, set)| {
            !super::story::fact_named(story, &content.facts, fact)
                .is_ok_and(|value| (value != 0) == set)
        });
        if d.posts.contains(&region) || waiting || sum / count.max(1) < u32::from(d.min_party_level)
        {
            return Err(JoinRefusal::Declines);
        }
        // The body they stood in the level as hands what ails it to the
        // character, and is gone.
        if let Some(body) = people.body(person) {
            if let Ok(conditions) = bodies.get(body) {
                commands.entity(companion).insert(conditions.clone());
            }
            commands.entity(body).despawn();
        }
        commands.entity(companion).remove::<Disabled>().insert(PartyMember(party_id));
        Ok(())
    }
}
/// A companion leaves the party, wholly; the caller does not edit the
/// formation. One who `stands` is left beside the party in a body of their
/// own; the story may take someone with no body left behind.
#[allow(clippy::type_complexity)]
pub fn dismiss(
    In((person, stands)): In<(Entity, bool)>,
    party: Single<(Entity, &PartyRoster, &Position, &PartyBody)>,
    persons: Query<(&ActorId, &Shape), With<Npc>>,
    people: People,
    companions: Query<(&Vitals, &Conditions)>,
    (held, cursed): (Items, Query<&Cursed>),
    level: Single<(Entity, &LevelNavigation, &crate::game::world::population::MonsterBodies)>,
    body_assets: Res<Assets<crate::content::monster::MonsterBody>>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    mut commands: Commands,
) -> bool {
    let (party, roster, at, facing) = *party;
    let Some(companion) = people.companion(person).filter(|member| roster.contains(member)) else {
        return false;
    };
    let Ok((id, &Shape(species))) = persons.get(person) else { return false };
    let Ok((v, conditions)) = companions.get(companion) else { return false };
    let (root, navigation, handles) = *level;
    let Some(shape) = handles.get(species, &body_assets) else { return false };
    let feet = PartyBody::feet(at)
        + Quat::from_rotation_y(facing.yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
    let wanted = content.npcs[id.0].character.as_ref().map(|t| &t.wanted);
    // They want what is on their list; an omnigun on it is any omnigun.
    let omnigun = |id| content.items[id].kind == crate::content::items::ItemKind::OMNIGUN;
    let wants = |item: crate::content::items::ItemId| {
        wanted
            .is_some_and(|list| list.iter().any(|&id| id == item || (omnigun(id) && omnigun(item))))
    };
    // What they want, and what a curse binds to them, stays with them; the
    // rest lies where the party stands and is taken into the pool, staying
    // there if the pool is full.
    let mut returned = vec![];
    for slot in EnumSet::<EquipSlot>::all() {
        let Some(item) = held.worn(companion, slot) else { continue };
        let binds = cursed.get(item).is_ok_and(|curse| !curse.released);
        if !(binds || held.id(item).is_some_and(wants)) {
            returned.push(item);
        }
    }
    for item in held.pack(companion).into_iter().flatten() {
        if !held.id(item).is_some_and(wants) {
            returned.push(item);
        }
    }
    return_to_pool(party, &returned, (root, PartyBody::feet(at)), &mut commands);
    // Their seat is given up, and their companion self is set aside.
    commands.entity(companion).remove::<PartyMember>().insert(Disabled);
    if stands && !v.is_dead() {
        let kind = &content.species[species];
        let mut kept =
            Newborn::rolled((-1, species, None), (feet, facing.yaw), (kind, shape), &mut rng);
        kept.conditions = conditions.clone();
        // One out cold with spent stamina stands as spent as they were.
        if conditions.exhausted() {
            kept.vitals.stamina.current = v.stamina.current.min(kept.vitals.stamina.max);
        }
        let mut body = commands.spawn((
            Embodies(person),
            kept.monster.clone(),
            kept.vitals.clone(),
            kept.frame,
            kept.conditions.clone(),
            kept.budget,
        ));
        crate::game::world::population::embody(
            &mut body,
            root,
            (feet, kept.yaw, kept.monster.body),
            (None, kind),
            (navigation, &mut configs),
        );
    }
    true
}

/// What someone leaving gives back to the party's pool: it lies where the
/// party stands, and is taken from there as far as the pool has room.
fn return_to_pool(
    party: Entity,
    returned: &[Entity],
    (level, feet): (Entity, Vec3),
    commands: &mut Commands,
) {
    let at = feet + Vec3::Y * crate::game::world::items::CLEARANCE;
    for &item in returned {
        commands.trigger(Put { item, at: Place::Ground { level, at } });
        commands.trigger(Give { item, to: None, party, pool_first: true, told: false });
    }
}
/// The original removes fallen companions outside combat. Until then
/// resurrection can clear the death mark as it does any member's. One
/// left behind dead is dead for good.
fn retire_fallen(
    battles: Query<(), With<crate::game::combat::Battle>>,
    fallen: Query<(Entity, &CompanionOf), (With<PartyMember>, With<Dead>)>,
    party: Single<(Entity, &Position)>,
    held: Items,
    level: Single<Entity, With<Level>>,
    mut commands: Commands,
) {
    if !battles.is_empty() {
        return;
    }
    let (party, at) = *party;
    for (companion, &CompanionOf(person)) in &fallen {
        // Everything they carried returns to the party; they keep their
        // seat no longer.
        let returned: Vec<_> = held.carried(companion).collect();
        return_to_pool(party, &returned, (*level, PartyBody::feet(at)), &mut commands);
        commands.entity(companion).remove::<PartyMember>().insert(Disabled);
        commands.entity(person).insert(Whereabouts::Killed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Race;
    use crate::content::items::{EquipClass, ItemCatalog, ItemDefinition, ItemKind, QuantityKind};
    use crate::content::monster::MonsterBody;
    use crate::content::npcs::NpcCatalog;
    use crate::content::npcs::NpcId;
    use crate::content::species::{MonsterCatalog, Species};
    use crate::game::actors::time::{Elapsed, Pace};
    use crate::game::characters::{Condition, Gender, Lasting, Profession};
    use crate::game::items::{Burden, Packed, Worn};
    use crate::game::party::body::PartyBody;
    use crate::game::party::roster::Party;
    use crate::game::time::Ticks;
    use crate::game::world::population::{MonsterBodies, ScaledBody};
    use bevy::ecs::system::RunSystemOnce;

    const SWORD: ItemId = ItemId(0);
    const RING: ItemId = ItemId(1);
    const CHARM: ItemId = ItemId(2);
    const POTION: ItemId = ItemId(3);
    const ARROWS: ItemId = ItemId(4);
    const WORN_SWORD: usize = EquipSlot::Primary as usize;
    const WORN_RING: usize = EquipSlot::Misc1 as usize;
    const WORN_CHARM: usize = EquipSlot::Misc2 as usize;

    /// A world with a party of one living in a level, and a recruitable NPC
    /// who wears a sword, a cursed ring and a charm, and packs a potion and
    /// some arrows, and wants the charm and the arrows.
    struct Scene {
        app: App,
        party: Entity,
        veteran: Entity,
        /// The person, and the character they are in the party.
        record: Entity,
        companion: Entity,
    }

    fn scene() -> Scene {
        let anyone = |name: &str, class, kind| ItemDefinition {
            name: name.to_owned(),
            class,
            kind,
            professions: EnumSet::all(),
            races: EnumSet::all(),
            genders: EnumSet::all(),
            weight: 5,
            ..default()
        };
        let mut items = vec![
            anyone("Sword", EquipClass::ShortWeapon, ItemKind::DAGGER),
            ItemDefinition { cursed: true, ..anyone("Ring", EquipClass::Misc, ItemKind::DAGGER) },
            anyone("Charm", EquipClass::Misc, ItemKind::DAGGER),
            anyone("Potion", EquipClass::Drink, ItemKind::DAGGER),
            ItemDefinition {
                quantity: QuantityKind::Stack,
                stack_limit: 100,
                ..anyone("Arrows", EquipClass::Ammunition, ItemKind::ARROW)
            },
        ];
        for (id, definition) in items.iter_mut().enumerate() {
            definition.id = ItemId(id as u16);
        }
        let mut template = NpcCharacter {
            name: "Rosa".to_owned(),
            profession: Profession::Fighter,
            race: Race::Human,
            gender: Gender::Female,
            portrait: 0,
            level: 3,
            attributes: enum_map::EnumMap::from_fn(|_| 40),
            skills: default(),
            spells: vec![],
            equipment: default(),
            backpack: default(),
            wanted: vec![CHARM, ARROWS],
        };
        template.equipment[WORN_SWORD] = Some(SWORD);
        template.equipment[WORN_RING] = Some(RING);
        template.equipment[WORN_CHARM] = Some(CHARM);
        template.backpack[0] = Some(POTION);
        template.backpack[2] = Some(ARROWS);
        let definition =
            crate::content::npcs::NpcDefinition { character: Some(template), ..default() };
        let mut app = App::new();
        crate::game::items::plugin(&mut app);
        crate::game::actors::plugin(&mut app);
        let mut bodies = Assets::<MonsterBody>::default();
        let body = bodies.add(MonsterBody { movement_rate: 0.0, ..default() });
        app.insert_resource(bodies)
            .insert_resource(Assets::<WalkerMotionConfig>::default())
            .insert_resource(Catalogs {
                items: ItemCatalog(items),
                npcs: NpcCatalog(vec![definition]),
                species: MonsterCatalog(vec![Species::default()]),
                ..default()
            })
            .insert_resource(GameRng::seeded(1))
            .init_resource::<super::super::story::StoryFacts>()
            .add_message::<crate::game::hits::Damaged>();
        let world = app.world_mut();
        let party = world.spawn((Party, Position(Vec3::ZERO), PartyBody::default())).id();
        world
            .spawn(Level {
                path: String::new(),
                definition: default(),
                navmeshes: vec![],
                messages: None,
            })
            .insert(MonsterBodies([(0, body)].into()));
        let sheet = Character::new("Vi", Race::Human, Gender::Female, Profession::Fighter);
        let vitals = Vitals::new(&sheet);
        let veteran = world.spawn((sheet, vitals, PartyMember(party))).id();
        let record = world
            .run_system_once(
                |mut commands: Commands, content: Res<Catalogs>, mut rng: ResMut<GameRng>| {
                    let record = commands
                        .spawn((
                            ActorId(NpcId(0)),
                            Npc::from_definition(&content.npcs[NpcId(0)]),
                            Whereabouts::AsPlaced,
                            Shape(0),
                        ))
                        .id();
                    companion(
                        &mut commands,
                        record,
                        content.npcs[NpcId(0)].character.as_ref().unwrap(),
                        &content,
                        &mut rng,
                    );
                    record
                },
            )
            .unwrap();
        app.update();
        let companion = app
            .world_mut()
            .run_system_once(move |people: People| people.companion(record))
            .unwrap()
            .expect("one who can join has a companion self");
        Scene { app, party, veteran, record, companion }
    }

    /// What a character wears, with its slot, and packs.
    fn carried(world: &mut World, who: Entity) -> (Vec<(EquipSlot, Entity)>, Vec<Entity>) {
        world
            .run_system_once(move |items: Items| {
                let mut worn: Vec<_> = items.wearing(who).collect();
                worn.sort_by_key(|&(slot, _)| slot as usize);
                (worn, items.pack(who).into_iter().flatten().collect())
            })
            .unwrap()
    }

    fn join_with(scene: &mut Scene) -> Result<(), JoinRefusal> {
        let actor = scene.record;
        let result = scene
            .app
            .world_mut()
            .run_system_once(move |mut recruiter: Recruiter| recruiter.admit(actor))
            .unwrap();
        scene.app.update();
        result
    }

    /// Joining gives a seat and nothing else: the same character and the
    /// same things, worn and packed as they were; the body they stood in
    /// goes and leaves what ailed it with them.
    #[test]
    fn joining_gives_a_seat_and_keeps_the_same_things() {
        let mut scene = scene();
        let world = scene.app.world_mut();
        assert!(world.entity(scene.companion).contains::<Disabled>(), "set aside until they join");
        // One set aside is out of every query; look at them as one of the world.
        world.entity_mut(scene.companion).remove::<Disabled>();
        let before = carried(world, scene.companion);
        world.entity_mut(scene.companion).insert(Disabled);
        let ailing = Conditions::from([(Condition::Poisoned, Lasting::Rounds(4))]);
        let body = world
            .spawn((
                Embodies(scene.record),
                crate::game::world::population::Monster {
                    location: 0,
                    species: 0,
                    body: ScaledBody::new(&MonsterBody::default(), 0),
                },
                Vitals::full(10, 10),
                ailing,
            ))
            .id();
        assert_eq!(join_with(&mut scene), Ok(()));
        let world = scene.app.world_mut();
        assert_eq!(world.get::<PartyMember>(scene.companion), Some(&PartyMember(scene.party)));
        assert!(!world.entity(scene.companion).contains::<Disabled>());
        assert!(world.get_entity(body).is_err(), "their body is gone");
        assert!(world.get::<Conditions>(scene.companion).unwrap().has(Condition::Poisoned));
        assert_eq!(carried(world, scene.companion), before, "the same things in the same places");
        let roster: Vec<_> = world.get::<PartyRoster>(scene.party).unwrap().iter().collect();
        assert_eq!(roster, [scene.veteran, scene.companion]);
    }

    /// Leaving takes the seat and nothing else: they keep what they want
    /// and what a curse binds, the rest goes to the party's pool, their
    /// companion self is set aside and they stand in a body of their own.
    #[test]
    fn dismissal_keeps_what_is_wanted_or_cursed_and_returns_the_rest() {
        let mut scene = scene();
        assert_eq!(join_with(&mut scene), Ok(()));
        let (record, companion, party) = (scene.record, scene.companion, scene.party);
        let world = scene.app.world_mut();
        let (worn, packed) = carried(world, companion);
        let thing = |of: &[(EquipSlot, Entity)], slot| of.iter().find(|p| p.0 == slot).unwrap().1;
        let (ring, charm, sword) = (
            thing(&worn, EquipSlot::Misc1),
            thing(&worn, EquipSlot::Misc2),
            thing(&worn, EquipSlot::Primary),
        );
        let (potion, arrows) = (packed[0], packed[1]);
        assert!(world.run_system_once_with(dismiss, (record, true)).unwrap());
        world.flush();
        assert!(!world.entity(companion).contains::<PartyMember>());
        assert!(world.entity(companion).contains::<Disabled>(), "set aside again");
        assert_eq!(world.get::<Worn>(ring).map(|worn| worn.by), Some(companion), "bound");
        assert_eq!(world.get::<Worn>(charm).map(|worn| worn.by), Some(companion), "wanted");
        assert_eq!(world.get::<Packed>(arrows).map(|packed| packed.by), Some(companion));
        assert!(world.get::<Worn>(sword).is_none() && world.get::<Packed>(potion).is_none());
        let pooled = world.run_system_once(move |items: Items| items.pool(party).to_vec()).unwrap();
        assert_eq!(pooled.len(), 2);
        assert!(pooled.contains(&sword) && pooled.contains(&potion));
        // They stand in the level again, in a body of their own.
        let body = world.run_system_once(move |people: People| people.body(record)).unwrap();
        assert!(body.is_some_and(|body| world.entity(body).contains::<Monster>()));
    }

    /// One with no seat is set aside: time does not pass for them and the
    /// pool's weight is not shared with them.
    #[test]
    fn a_companion_without_a_seat_is_out_of_the_world() {
        let mut scene = scene();
        let (companion, veteran, party) = (scene.companion, scene.veteran, scene.party);
        let world = scene.app.world_mut();
        for who in [companion, veteran] {
            world.get_mut::<Vitals>(who).unwrap().health.current = 1;
            world
                .get_mut::<Conditions>(who)
                .unwrap()
                .afflict(Condition::Poisoned, Lasting::Rounds(50));
        }
        let before = (
            world.get::<Vitals>(companion).unwrap().clone(),
            world.get::<Conditions>(companion).unwrap().clone(),
        );
        world.trigger(Elapsed { ticks: Ticks { stamina: 30, aging: 30 }, pace: Pace::Camping });
        world.flush();
        assert_eq!(*world.get::<Vitals>(companion).unwrap(), before.0, "nothing passed for them");
        assert_eq!(*world.get::<Conditions>(companion).unwrap(), before.1);
        assert_ne!(*world.get::<Conditions>(veteran).unwrap(), before.1);
        let heavy = world
            .run_system_once(|mut commands: Commands, content: Res<Catalogs>| {
                make(&mut commands, &content.items[SWORD], 1, true)
            })
            .unwrap();
        world.trigger(Put { item: heavy, at: Place::Pool { party, row: 0 } });
        world.run_schedule(crate::game::actors::Settle);
        assert!(world.get::<Burden>(veteran).unwrap().weight > 0, "the seated one carries it");
        assert_eq!(*world.get::<Burden>(companion).unwrap(), Burden::default());
    }
}
