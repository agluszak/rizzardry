//! Party transitions for NPCs. A stored profile is inactive data, not an off-party Character.
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
        items::{Cursed, Give, ItemStack, Items, Place, Put, unpack},
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
/// A character who is not in the party, with what they wore and packed as
/// values: items are things only while the party has them (see
/// [`ItemStack`]).
#[derive(Clone)]
pub struct Profile {
    character: Character,
    vitals: Vitals,
    worn: Vec<(EquipSlot, ItemStack)>,
    packed: Vec<(usize, ItemStack)>,
    spells: LearnedSpells,
    points: SpellPoints,
}
#[derive(Component, Clone)]
pub struct StoredCharacter(Profile);
pub fn character(t: &NpcCharacter, content: &Catalogs, rng: &mut GameRng) -> StoredCharacter {
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
    let stack = |item: &ItemId, rng: &mut GameRng| {
        ItemStack::rolled(&content.items[*item], rng, true, true)
    };
    let worn = t
        .equipment
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| Some((EquipSlot::from_usize(slot), stack(item.as_ref()?, rng))))
        .collect();
    let packed = t
        .backpack
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| Some((slot, stack(item.as_ref()?, rng))))
        .collect();
    StoredCharacter(Profile {
        vitals: Vitals::new(&c),
        character: c,
        worn,
        packed,
        spells,
        points: default(),
    })
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
    records: Query<(&ActorId, &Npc, &StoredCharacter, Option<&NpcBodies>)>,
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
            let (id, npc, stored, binding) =
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
            let mut profile = stored.0.clone();
            if let Some(binding) = binding {
                for b in binding.iter() {
                    if let Ok(v) = bodies.get(b) {
                        profile.vitals.conditions = v.conditions.clone();
                    }
                    commands.entity(b).despawn();
                }
            }
            let Profile { character, vitals, worn, packed, spells, points } = profile;
            commands.entity(request.actor).remove::<StoredCharacter>().insert((
                Name::new(character.name.clone()),
                character,
                vitals,
                spells,
                points,
                PartyMember(party_id),
            ));
            // What they wore and packed becomes things again.
            let member = request.actor;
            let wearing =
                worn.iter().map(|(slot, stack)| (Place::Slot { member, slot: *slot }, stack));
            let packing =
                packed.iter().map(|(slot, stack)| (Place::Pack { member, slot: *slot }, stack));
            for (at, stack) in wearing.chain(packing) {
                let item = unpack(&mut commands, &content.items[stack.item], stack);
                commands.trigger(Put { item, at });
            }
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
    profiles: Query<(&Character, &Vitals, &LearnedSpells, &SpellPoints)>,
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
    let Ok((c, v, spells, points)) = profiles.get(actor) else { return false };
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
    // What they keep goes into their record; the rest lies where the party
    // stands and is taken into the pool, staying there if the pool is full.
    let (mut worn, mut packed, mut returned) = (vec![], vec![], vec![]);
    for slot in EnumSet::<EquipSlot>::all() {
        let Some(item) = held.worn(actor, slot) else { continue };
        let binds = cursed.get(item).is_ok_and(|curse| !curse.released);
        match held.stack(item, &content.items) {
            Some(stack) if held.id(item).is_some_and(wants) || binds => {
                worn.push((slot, stack));
                commands.entity(item).despawn();
            }
            _ => returned.push(item),
        }
    }
    for (slot, item) in held.pack(actor).into_iter().enumerate() {
        let Some(item) = item else { continue };
        match held.stack(item, &content.items) {
            Some(stack) if held.id(item).is_some_and(wants) => {
                packed.push((slot, stack));
                commands.entity(item).despawn();
            }
            _ => returned.push(item),
        }
    }
    let profile = Profile {
        character: c.clone(),
        vitals: v.clone(),
        worn,
        packed,
        spells: spells.clone(),
        points: points.clone(),
    };
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
    commands
        .entity(actor)
        .remove::<(PartyMember, Character, Vitals, LearnedSpells, SpellPoints)>()
        .insert(StoredCharacter(profile));
    commands.trigger(crate::game::items::Reweigh);
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
    members: Query<
        (Entity, &Character, &Vitals, &LearnedSpells, &SpellPoints),
        (With<Npc>, With<PartyMember>, With<Dead>),
    >,
    party: Single<(Entity, &Position)>,
    held: Items,
    level: Single<Entity, With<Level>>,
    mut commands: Commands,
) {
    if !battles.is_empty() {
        return;
    }
    let (party, at) = *party;
    for (entity, character, vitals, spells, points) in &members {
        // Everything they carried returns to the party.
        let returned: Vec<_> = held.carried(entity).collect();
        return_to_pool(party, &returned, (*level, PartyBody::feet(at)), &mut commands);
        commands
            .entity(entity)
            .remove::<(PartyMember, Character, Vitals, LearnedSpells, SpellPoints)>()
            .insert(StoredCharacter(Profile {
                character: character.clone(),
                vitals: vitals.clone(),
                worn: vec![],
                packed: vec![],
                spells: spells.clone(),
                points: points.clone(),
            }));
    }
}
