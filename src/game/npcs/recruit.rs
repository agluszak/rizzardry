//! Party transitions for NPCs. A recruitable NPC is a character from the start, with or without a
//! seat in the party.
use super::{ActorId, Npc, NpcBodies, NpcBody, NpcLocation};
use crate::game::actors::Attributes;
use crate::{
    content::{items::ItemId, npcs::NpcCharacter},
    game::{
        characters::{Character, Dead, Vitals},
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
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use enum_map::Enum;
use enumset::EnumSet;
/// What a recruitable NPC is made with, on their record: a character with
/// their sheet, health, spells and spell points, wearing and packing what is
/// authored for them, each in the slot it is authored for. They have no seat
/// in the party until they join.
pub fn character(
    record: &mut EntityCommands,
    t: &NpcCharacter,
    content: &Catalogs,
    rng: &mut GameRng,
) {
    let mut c = Character::new(&t.name, t.race, t.gender, t.profession);
    c.attributes = t.attributes;
    c.skill_points = t.skills;
    c.profession_levels = default();
    c.profession_levels[t.profession] = 1;
    c.experience_previous_goal = 0;
    while c.level() < t.level {
        c.experience_previous_goal = c.experience_goal();
        c.profession_levels[t.profession] += 1;
    }
    c.experience = c.experience_previous_goal;
    c.portrait = crate::game::characters::creation::identity::PORTRAITS
        .get(t.portrait)
        .copied()
        .unwrap_or_default()
        .into();
    let skills = crate::game::actors::Skills::of_sheet(&c);
    let spells = LearnedSpells(
        t.spells
            .iter()
            .copied()
            .filter(|&s| crate::game::magic::can_learn(&c, &skills, &content.spells[s]))
            .collect(),
    );
    let vitals = Vitals::new(&c);
    record.insert((Name::new(c.name.clone()), c, vitals, spells, SpellPoints::default()));
    let member = record.id();
    let commands = record.commands_mut();
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
    records: Query<
        'w,
        's,
        (&'static ActorId, Has<super::Killed>, Option<&'static NpcBodies>),
        (With<Npc>, With<Character>),
    >,
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    members: Query<'w, 's, (&'static Character, Has<crate::game::actors::CanFight>, Has<Npc>)>,
    bodies: Query<'w, 's, &'static crate::game::characters::Conditions, With<Monster>>,
    dead: Query<'w, 's, (), With<Dead>>,
    level: Single<'w, 's, &'static Level>,
    content: Res<'w, Catalogs>,
    story: Res<'w, super::story::StoryFacts>,
    commands: Commands<'w, 's>,
}
impl Recruiter<'_, '_> {
    /// Someone asked to join does, or says why not: the party has room for
    /// two such; they keep to a post in a region they serve, wait on what
    /// the story owes them, and want a party of their level.
    pub fn admit(&mut self, actor: Entity) -> Result<(), JoinRefusal> {
        let Self { records, party, members, bodies, dead, level, content, story, commands } = self;
        let (party_id, roster) = **party;
        let occupied = roster.iter().filter(|&e| members.get(e).is_ok_and(|r| r.2)).count();
        {
            let (id, killed, binding) = records.get(actor).map_err(|_| JoinRefusal::Declines)?;
            if killed || dead.contains(actor) || roster.contains(&actor) {
                return Err(JoinRefusal::Declines);
            }
            let d = &content.npcs[id.0];
            if occupied >= 2 {
                return Err(JoinRefusal::NoRoom);
            }
            let region =
                crate::content::levels::LevelInfo::for_path(&level.path).map_or(0, |l| l.band);
            let (sum, count) = roster
                .iter()
                .filter_map(|e| members.get(e).ok())
                .filter(|(_, can_fight, _)| *can_fight)
                .fold((0u32, 0u32), |(sum, count), (c, _, _)| {
                    (sum + u32::from(c.level()), count + 1)
                });
            let waiting = d.joins_when.is_some_and(|(fact, set)| {
                !super::story::fact_named(story, &content.facts, fact)
                    .is_ok_and(|value| (value != 0) == set)
            });
            if d.posts.contains(&region)
                || waiting
                || sum / count.max(1) < u32::from(d.min_party_level)
            {
                return Err(JoinRefusal::Declines);
            }
            // The body they stood in the level as hands its conditions to
            // the character and is gone.
            if let Some(binding) = binding {
                for b in binding.iter() {
                    if let Ok(conditions) = bodies.get(b) {
                        commands.entity(actor).insert(conditions.clone());
                    }
                    commands.entity(b).despawn();
                }
            }
            commands.entity(actor).insert(PartyMember(party_id));
            Ok(())
        }
    }
}
/// Dismissal is a complete ownership transfer; the caller does not edit
/// formation. One who `stands` is left beside the party in a body of their
/// own; the story may take someone with no body left behind.
pub fn dismiss(
    In((actor, stands)): In<(Entity, bool)>,
    party: Single<(Entity, &PartyRoster, &Position, &PartyBody)>,
    records: Query<(&ActorId, &NpcLocation), With<Npc>>,
    profiles: Query<(&Character, &Vitals, &crate::game::characters::Conditions)>,
    (held, cursed): (Items, Query<&Cursed>),
    level: Single<(
        Entity,
        &Level,
        &LevelNavigation,
        &crate::game::world::population::MonsterBodies,
    )>,
    body_assets: Res<Assets<crate::content::monster::MonsterBody>>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    mut commands: Commands,
) -> bool {
    let (party, roster, at, facing) = *party;
    if !roster.contains(&actor) {
        return false;
    }
    let Ok((id, location)) = records.get(actor) else { return false };
    let Ok((c, v, conditions)) = profiles.get(actor) else { return false };
    let (root, played, navigation, handles) = *level;
    let Some(shape) = handles.get(location.species, &body_assets) else { return false };
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
        let Some(item) = held.worn(actor, slot) else { continue };
        let binds = cursed.get(item).is_ok_and(|curse| !curse.released);
        if !(binds || held.id(item).is_some_and(wants)) {
            returned.push(item);
        }
    }
    for item in held.pack(actor).into_iter().flatten() {
        if !held.id(item).is_some_and(wants) {
            returned.push(item);
        }
    }
    return_to_pool(party, &returned, (root, PartyBody::feet(at)), &mut commands);
    if stands && !v.is_dead() {
        let kind = &content.species[location.species];
        let mut kept = Newborn::rolled(
            (-1, location.species, None),
            (feet, facing.yaw),
            (kind, shape),
            &mut rng,
        );
        kept.conditions = conditions.clone();
        if conditions.exhausted() {
            kept.vitals.stamina.current =
                v.stamina.current.min(c.max_stamina(&Attributes::of_sheet(c)));
        }
        let mut body = commands.spawn((
            NpcBody(actor),
            kept.monster.clone(),
            kept.vitals.clone(),
            kept.budget,
        ));
        crate::game::world::population::embody(
            &mut body,
            root,
            (feet, kept.yaw, kept.monster.body),
            (None, kind),
            (navigation, &mut configs),
        );
        commands
            .entity(actor)
            .insert(NpcLocation { map: played.path.clone(), species: location.species });
    }
    commands.entity(actor).remove::<PartyMember>();
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
/// Retail removes fallen RPCs outside combat. Until then resurrection can clear
/// the death mark through the normal character condition lifecycle.
fn retire_fallen(
    battles: Query<(), With<crate::game::combat::Battle>>,
    members: Query<Entity, (With<Npc>, With<PartyMember>, With<Dead>)>,
    party: Single<(Entity, &Position)>,
    held: Items,
    level: Single<Entity, With<Level>>,
    mut commands: Commands,
) {
    if !battles.is_empty() {
        return;
    }
    let (party, at) = *party;
    for entity in &members {
        // Everything they carried returns to the party; they keep their seat
        // no longer.
        let returned: Vec<_> = held.carried(entity).collect();
        return_to_pool(party, &returned, (*level, PartyBody::feet(at)), &mut commands);
        commands.entity(entity).remove::<PartyMember>();
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
    use crate::game::characters::{Condition, Gender, Lasting, Profession};
    use crate::game::items::{Burden, Packed, Worn};
    use crate::game::party::body::PartyBody;
    use crate::game::party::recovery::{Elapsed, Pace, pass_time};
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
        record: Entity,
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
                    let mut record = commands
                        .spawn((ActorId(NpcId(0)), Npc::from_definition(&content.npcs[NpcId(0)])));
                    character(
                        &mut record,
                        content.npcs[NpcId(0)].character.as_ref().unwrap(),
                        &content,
                        &mut rng,
                    );
                    record.insert(NpcLocation { map: String::new(), species: 0 });
                    record.id()
                },
            )
            .unwrap();
        app.update();
        Scene { app, party, veteran, record }
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

    /// Joining gives a seat and nothing else: the same sheet and the same
    /// things, worn and packed as they were; the body they stood in goes and
    /// leaves its conditions with them.
    #[test]
    fn joining_gives_a_seat_and_keeps_the_same_things() {
        let mut scene = scene();
        let before = carried(scene.app.world_mut(), scene.record);
        let world = scene.app.world_mut();
        let ailing =
            crate::game::characters::Conditions::from([(Condition::Poisoned, Lasting::Rounds(4))]);
        let body = world
            .spawn((
                NpcBody(scene.record),
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
        assert_eq!(world.get::<PartyMember>(scene.record), Some(&PartyMember(scene.party)));
        assert!(world.get_entity(body).is_err(), "their body is gone");
        assert!(
            world
                .get::<crate::game::characters::Conditions>(scene.record)
                .unwrap()
                .has(Condition::Poisoned)
        );
        assert_eq!(carried(world, scene.record), before, "the same things in the same places");
        let roster: Vec<_> = world.get::<PartyRoster>(scene.party).unwrap().iter().collect();
        assert_eq!(roster, [scene.veteran, scene.record]);
    }

    /// Dismissal takes the seat and nothing else: they keep what they want
    /// and what a curse binds, and the rest goes to the party's pool.
    #[test]
    fn dismissal_keeps_what_is_wanted_or_cursed_and_returns_the_rest() {
        let mut scene = scene();
        assert_eq!(join_with(&mut scene), Ok(()));
        let world = scene.app.world_mut();
        let (worn, packed) = carried(world, scene.record);
        let thing = |of: &[(EquipSlot, Entity)], slot| of.iter().find(|p| p.0 == slot).unwrap().1;
        let (ring, charm, sword) = (
            thing(&worn, EquipSlot::Misc1),
            thing(&worn, EquipSlot::Misc2),
            thing(&worn, EquipSlot::Primary),
        );
        let (potion, arrows) = (packed[0], packed[1]);
        assert!(world.run_system_once_with(dismiss, (scene.record, true)).unwrap());
        world.flush();
        assert!(!world.entity(scene.record).contains::<PartyMember>());
        assert!(world.entity(scene.record).contains::<Character>());
        assert_eq!(world.get::<Worn>(ring).map(|worn| worn.by), Some(scene.record), "bound");
        assert_eq!(world.get::<Worn>(charm).map(|worn| worn.by), Some(scene.record), "wanted");
        assert_eq!(world.get::<Packed>(arrows).map(|packed| packed.by), Some(scene.record));
        assert!(world.get::<Worn>(sword).is_none() && world.get::<Packed>(potion).is_none());
        let party = scene.party;
        let pooled = world.run_system_once(move |items: Items| items.pool(party).to_vec()).unwrap();
        assert_eq!(pooled.len(), 2);
        assert!(pooled.contains(&sword) && pooled.contains(&potion));
        // They stand in the level again, in a body of their own.
        let bodies = world.get::<NpcBodies>(scene.record).expect("a body").iter().count();
        assert_eq!(bodies, 1);
    }

    /// A character without a seat takes no part in the party's rules: time
    /// does not pass for them, the pool's weight is not shared with them, and
    /// they are given no experience.
    #[test]
    fn a_character_without_a_seat_is_not_in_the_party_rules() {
        let mut scene = scene();
        let (record, veteran, party) = (scene.record, scene.veteran, scene.party);
        let world = scene.app.world_mut();
        // Time: both are hurt and poisoned; only the seated one is healed.
        for who in [record, veteran] {
            world.get_mut::<Vitals>(who).unwrap().health.current = 1;
            world
                .get_mut::<crate::game::characters::Conditions>(who)
                .unwrap()
                .afflict(Condition::Poisoned, Lasting::Rounds(50));
        }
        let mut seatless = world.get::<Vitals>(record).unwrap().clone();
        let ailing = world.get::<crate::game::characters::Conditions>(record).unwrap().clone();
        world.add_observer(pass_time);
        world.trigger(Elapsed { ticks: Ticks { stamina: 30, aging: 30 }, pace: Pace::Camping });
        let after = world.get::<Vitals>(record).unwrap();
        seatless.regeneration = after.regeneration;
        assert_eq!(*after, seatless, "nothing passed for them");
        assert_eq!(world.get::<crate::game::characters::Conditions>(record).unwrap(), &ailing);
        assert_ne!(world.get::<crate::game::characters::Conditions>(veteran).unwrap(), &ailing);
        // Burden: the pool's weight goes to the seated one only.
        let heavy = world
            .run_system_once(|mut commands: Commands, content: Res<Catalogs>| {
                make(&mut commands, &content.items[SWORD], 1, true)
            })
            .unwrap();
        world.trigger(Put { item: heavy, at: Place::Pool { party, row: 0 } });
        world.run_schedule(crate::game::actors::Settle);
        let own = world.get::<Burden>(veteran).unwrap().weight;
        assert!(own > 0, "the seated member carries the pool's share");
        assert_eq!(*world.get::<Burden>(record).unwrap(), Burden::default());
    }
}
