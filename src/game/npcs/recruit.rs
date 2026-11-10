//! Party transitions for NPCs. A recruitable NPC is a character from the start, with or without a
//! seat in the party.
use super::{ActorId, Npc, NpcBodies, NpcBody, NpcLocation};
use crate::game::actors::Attributes;
use crate::{
    content::{
        items::ItemId,
        npcs::{NpcCharacter, NpcId},
    },
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
    let spells = LearnedSpells(
        t.spells
            .iter()
            .copied()
            .filter(|&s| crate::game::magic::can_learn(&c, &content.spells[s]))
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
#[derive(Message)]
pub struct Join {
    pub actor: Entity,
    pub reply_to: Entity,
}
#[derive(Message)]
pub struct Joined {
    pub reply_to: Entity,
    pub result: Result<(), JoinRefusal>,
}
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Recruiting;
pub fn install(app: &mut App) {
    app.add_message::<Join>()
        .add_message::<Joined>()
        .add_systems(Update, (join, retire_fallen).chain().in_set(Recruiting))
        .add_observer(died)
        .add_observer(revived);
}
fn service(flags: u32, region: u8) -> bool {
    let bit = match region {
        2 => 1,
        3 => 2,
        4 => 0x400,
        5 => 4,
        7 => 8,
        8 => 0x80,
        9 => 0x40,
        10 => 0x20,
        11 => 0x10,
        12 => 0x100,
        13 => 0x800,
        14 => 0x200,
        15 => 0x1000,
        6 => 0x2000,
        _ => 0,
    };
    flags & bit != 0
}
fn terms(id: NpcId, story: &super::story::StoryFacts, content: &Catalogs) -> bool {
    let requirement = match id.0 {
        0x2b => Some(("FACT_UMISSION_SCUBA_DONE", false)),
        0x4f => Some(("FACT_SEXUS_PAID", true)),
        0x4a => Some(("FACT_TRYNNIE_MADRAS_WILL_JOIN", true)),
        0x10 | 0x11 => Some(("FACT_PEACE_ACHIEVED", false)),
        _ => None,
    };
    requirement.is_none_or(|(name, expected)| {
        super::story::fact_named(story, &content.facts, name).is_ok_and(|v| (v != 0) == expected)
    })
}
fn join(
    mut requests: MessageReader<Join>,
    mut results: MessageWriter<Joined>,
    records: Query<(&ActorId, &Npc, Option<&NpcBodies>), With<Character>>,
    party: Single<(Entity, &PartyRoster)>,
    members: Query<(&Character, &Vitals, Has<Npc>)>,
    bodies: Query<&Vitals, With<Monster>>,
    level: Single<&Level>,
    content: Res<Catalogs>,
    story: Res<super::story::StoryFacts>,
    mut commands: Commands,
) {
    let (party_id, roster) = *party;
    let mut occupied = roster.iter().filter(|&e| members.get(e).is_ok_and(|r| r.2)).count();
    let mut admitted = std::collections::HashSet::new();
    for request in requests.read() {
        let result = (|| {
            let (id, npc, binding) =
                records.get(request.actor).map_err(|_| JoinRefusal::Declines)?;
            if npc.dead || roster.contains(&request.actor) || admitted.contains(&request.actor) {
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
                .filter(|(_, v, _)| v.can_fight())
                .fold((0u32, 0u32), |(sum, count), (c, _, _)| {
                    (sum + u32::from(c.level()), count + 1)
                });
            if service(d.services, region)
                || !terms(id.0, &story, &content)
                || sum / count.max(1) < u32::from(d.min_party_level)
            {
                return Err(JoinRefusal::Declines);
            }
            // The body they stood in the level as hands its conditions to
            // the character and is gone.
            if let Some(binding) = binding {
                for b in binding.iter() {
                    if let Ok(v) = bodies.get(b) {
                        let conditions = v.conditions.clone();
                        commands
                            .entity(request.actor)
                            .entry::<Vitals>()
                            .and_modify(move |mut vitals| vitals.conditions = conditions);
                    }
                    commands.entity(b).despawn();
                }
            }
            commands.entity(request.actor).insert(PartyMember(party_id));
            occupied += 1;
            admitted.insert(request.actor);
            Ok(())
        })();
        results.write(Joined { reply_to: request.reply_to, result });
    }
}
/// Dismissal is a complete ownership transfer; the caller does not edit formation.
pub fn dismiss(
    In(actor): In<Entity>,
    party: Single<(Entity, &PartyRoster, &Position, &PartyBody)>,
    records: Query<(&ActorId, &NpcLocation), With<Npc>>,
    profiles: Query<(&Character, &Vitals)>,
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
    let Ok((c, v)) = profiles.get(actor) else { return false };
    let (root, played, navigation, handles) = *level;
    let Some(shape) = handles.get(location.species, &body_assets) else { return false };
    let feet = PartyBody::feet(at)
        + Quat::from_rotation_y(facing.yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
    let wanted = content.npcs[id.0].character.as_ref().map(|t| &t.wanted);
    let wants = |item: crate::content::items::ItemId| {
        wanted.is_some_and(|list| {
            list.iter().any(|&id| {
                id == item
                    || (content.items[id].kind.0 == 0x83 && content.items[item].kind.0 == 0x83)
            })
        })
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
    if !v.is_dead() {
        let kind = &content.species[location.species];
        let mut kept = Newborn::rolled(
            (-1, location.species, None),
            (feet, facing.yaw),
            (kind, shape),
            &mut rng,
        );
        kept.vitals.conditions = v.conditions.clone();
        if v.conditions.exhausted() {
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
fn died(event: On<Add<Dead>>, mut records: Query<&mut Npc, With<PartyMember>>) {
    if let Ok(mut npc) = records.get_mut(event.entity) {
        npc.dead = true;
    }
}
fn revived(event: On<Remove<Dead>>, mut records: Query<&mut Npc, With<PartyMember>>) {
    if let Ok(mut npc) = records.get_mut(event.entity) {
        npc.dead = false;
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
    use crate::content::species::{MonsterCatalog, Species};
    use crate::game::characters::{Condition, Gender, Lasting, Profession};
    use crate::game::items::{Burden, Packed, Worn};
    use crate::game::party::body::PartyBody;
    use crate::game::party::recovery::{Elapsed, Pace, pass_time};
    use crate::game::party::roster::Party;
    use crate::game::time::Ticks;
    use crate::game::world::level::{LevelOwned, SetAside, set_aside};
    use crate::game::world::population::{MonsterBodies, ScaledBody};
    use bevy::ecs::{entity_disabling::Disabled, system::RunSystemOnce};

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
        level: Entity,
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
            .add_message::<crate::game::hits::Damaged>()
            .add_message::<Join>()
            .add_message::<Joined>()
            .add_systems(Update, join)
            .add_observer(set_aside);
        let world = app.world_mut();
        let party = world.spawn((Party, Position(Vec3::ZERO), PartyBody::default())).id();
        let level = world
            .spawn(Level {
                path: String::new(),
                definition: default(),
                navmeshes: vec![],
                script: default(),
                messages: None,
            })
            .insert(MonsterBodies([(0, body)].into()))
            .id();
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
        Scene { app, party, level, veteran, record }
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
        let reply_to = scene.veteran;
        scene.app.world_mut().write_message(Join { actor: scene.record, reply_to });
        scene.app.update();
        let mut results = scene.app.world_mut().resource_mut::<Messages<Joined>>();
        results.drain().next().expect("an answer").result
    }

    fn definitions(world: &mut World, things: &[Entity]) -> Vec<ItemId> {
        let things = things.to_vec();
        world
            .run_system_once(move |items: Items| {
                things.iter().map(|&thing| items.id(thing).unwrap()).collect::<Vec<_>>()
            })
            .unwrap()
    }

    /// A recruit is a character from the moment they are made: their sheet,
    /// their health, and what they wear and pack, each where it is authored,
    /// as things in the world, with no seat in the party.
    #[test]
    fn a_recruit_wears_and_packs_what_is_authored_before_joining() {
        let mut scene = scene();
        let world = scene.app.world_mut();
        let record = world.entity(scene.record);
        assert_eq!(record.get::<Character>().unwrap().name, "Rosa");
        assert_eq!(record.get::<Character>().unwrap().level(), 3);
        assert!(record.contains::<Vitals>());
        assert!(!record.contains::<PartyMember>());
        assert!(!record.contains::<LevelOwned>());
        let (worn, packed) = carried(world, scene.record);
        let slots: Vec<_> = worn.iter().map(|&(slot, _)| slot).collect();
        assert_eq!(slots, [EquipSlot::Misc1, EquipSlot::Misc2, EquipSlot::Primary]);
        let things: Vec<_> = worn.iter().map(|&(_, thing)| thing).collect();
        assert_eq!(definitions(world, &things), [RING, CHARM, SWORD]);
        assert_eq!(definitions(world, &packed), [POTION, ARROWS]);
        let packs: Vec<_> =
            packed.iter().map(|&thing| world.get::<Packed>(thing).unwrap().slot).collect();
        assert_eq!(packs, [0, 2]);
        // Nothing is weighed against a character who is not in the party.
        assert_eq!(*world.get::<Burden>(scene.record).unwrap(), Burden::default());
    }

    /// Joining gives a seat and nothing else: the same sheet and the same
    /// things, worn and packed as they were; the body they stood in goes and
    /// leaves its conditions with them.
    #[test]
    fn joining_gives_a_seat_and_keeps_the_same_things() {
        let mut scene = scene();
        let before = carried(scene.app.world_mut(), scene.record);
        let world = scene.app.world_mut();
        let mut ailing = Vitals::full(10, 10);
        ailing.afflict(Condition::Poisoned, Lasting::Rounds(4));
        let body = world
            .spawn((
                NpcBody(scene.record),
                crate::game::world::population::Monster {
                    location: 0,
                    species: 0,
                    body: ScaledBody::new(&MonsterBody::default(), 0),
                },
                ailing,
            ))
            .id();
        assert_eq!(join_with(&mut scene), Ok(()));
        let world = scene.app.world_mut();
        assert_eq!(world.get::<PartyMember>(scene.record), Some(&PartyMember(scene.party)));
        assert!(world.get_entity(body).is_err(), "their body is gone");
        let vitals = world.get::<Vitals>(scene.record).unwrap();
        assert!(vitals.conditions.has(Condition::Poisoned));
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
        assert!(world.run_system_once_with(dismiss, scene.record).unwrap());
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
            let mut vitals = world.get_mut::<Vitals>(who).unwrap();
            vitals.health.current = 1;
            vitals.afflict(Condition::Poisoned, Lasting::Rounds(50));
        }
        let mut seatless = world.get::<Vitals>(record).unwrap().clone();
        world.write_message(Elapsed {
            ticks: Ticks { stamina: 30, aging: 30 },
            pace: Pace::Camping,
        });
        world.run_system_once(pass_time).unwrap();
        let after = world.get::<Vitals>(record).unwrap();
        seatless.regeneration = after.regeneration;
        assert_eq!(*after, seatless, "nothing passed for them");
        assert_ne!(world.get::<Vitals>(veteran).unwrap().conditions, after.conditions);
        // Burden: the pool's weight goes to the seated one only.
        let heavy = world
            .run_system_once(|mut commands: Commands, content: Res<Catalogs>| {
                make(&mut commands, &content.items[SWORD], 1, true)
            })
            .unwrap();
        world.trigger(Put { item: heavy, at: Place::Pool { party, row: 0 } });
        world.flush();
        let own = world.get::<Burden>(veteran).unwrap().weight;
        assert!(own > 0, "the seated member carries the pool's share");
        assert_eq!(*world.get::<Burden>(record).unwrap(), Burden::default());
    }

    /// A level that is left is set aside with its monsters but not with the
    /// character who stood in it as one of them, nor with a seated member's
    /// things.
    #[test]
    fn setting_a_level_aside_does_not_sweep_characters_or_their_things() {
        let mut scene = scene();
        assert_eq!(join_with(&mut scene), Ok(()));
        assert!(scene.app.world_mut().run_system_once_with(dismiss, scene.record).unwrap());
        let world = scene.app.world_mut();
        world.flush();
        let body = world.get::<NpcBodies>(scene.record).unwrap().iter().next().unwrap();
        let (worn, packed) = carried(world, scene.record);
        let (veteran, level) = (scene.veteran, scene.level);
        world.entity_mut(level).insert(SetAside { at_ms: 0 });
        world.flush();
        assert!(world.entity(body).contains::<Disabled>(), "their body is set aside");
        let untouched = [scene.record, veteran]
            .into_iter()
            .chain(worn.into_iter().map(|(_, thing)| thing))
            .chain(packed);
        for thing in untouched {
            assert!(!world.entity(thing).contains::<Disabled>());
        }
    }
}
