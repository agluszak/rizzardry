//! Party transitions for NPCs. A stored profile is inactive data, not an off-party Character.
use super::{ActorId, Npc, NpcBodies, NpcBody, NpcLocation};
use crate::{
    content::npcs::{NpcCharacter, NpcId},
    game::{
        characters::{Character, Dead, Vitals},
        data::Catalogs,
        inventory::{Carried, EquipSlot, Equipped, ItemIds, PartyInventory, make_item},
        magic::{LearnedSpells, SpellPoints},
        party::{
            body::PartyBody,
            roster::{PartyMember, PartyRoster},
        },
        rng::GameRng,
        world::{
            level::{KeptMonster, Level},
            navigation::{LevelNavigation, WalkerMotionConfig},
            population::Monster,
        },
    },
};
use avian3d::prelude::Position;
use bevy::prelude::*;
#[derive(Bundle, Clone)]
pub struct Profile {
    character: Character,
    vitals: Vitals,
    carried: Carried,
    spells: LearnedSpells,
    points: SpellPoints,
}
#[derive(Component, Clone)]
pub struct StoredCharacter(Profile);
pub fn character(
    t: &NpcCharacter,
    content: &Catalogs,
    ids: &mut ItemIds,
    rng: &mut GameRng,
) -> StoredCharacter {
    use enum_map::Enum;
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
    let mut carried = Carried::default();
    for (slot, item) in t.equipment.iter().enumerate() {
        if let Some(item) = item {
            carried.equipment[EquipSlot::from_usize(slot)] =
                Some(Equipped::new(make_item(&content.items[*item], ids, rng, true, true)));
        }
    }
    for (slot, item) in t.backpack.iter().enumerate() {
        if let Some(item) = item {
            carried.backpack[slot] = Some(make_item(&content.items[*item], ids, rng, true, true));
        }
    }
    StoredCharacter(Profile {
        vitals: Vitals::new(&c),
        character: c,
        carried,
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
            let name = profile.character.name.clone();
            commands.entity(request.actor).remove::<StoredCharacter>().insert((
                Name::new(name),
                profile,
                PartyMember(party_id),
            ));
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
    mut party: Single<(&PartyRoster, &mut PartyInventory, &Position, &PartyBody)>,
    records: Query<(&ActorId, &NpcLocation), With<Npc>>,
    profiles: Query<(&Character, &Vitals, &Carried, &LearnedSpells, &SpellPoints)>,
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
    let (roster, inventory, at, facing) = &mut *party;
    if !roster.contains(&actor) {
        return false;
    }
    let Ok((id, location)) = records.get(actor) else { return false };
    let Ok((c, v, carried, spells, points)) = profiles.get(actor) else { return false };
    let (root, played, navigation, handles) = *level;
    let Some(shape) = handles.get(location.species, &body_assets) else { return false };
    let feet = PartyBody::feet(at)
        + Quat::from_rotation_y(facing.yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
    let mut profile = Profile {
        character: c.clone(),
        vitals: v.clone(),
        carried: carried.clone(),
        spells: spells.clone(),
        points: points.clone(),
    };
    let wanted = content.npcs[id.0].character.as_ref().map(|t| &t.wanted);
    let wants = |item: crate::content::items::ItemId| {
        wanted.is_some_and(|list| {
            list.iter().any(|&id| {
                id == item
                    || (content.items[id].kind.0 == 0x83 && content.items[item].kind.0 == 0x83)
            })
        })
    };
    let mut returned = vec![];
    for slot in profile.carried.equipment.values_mut() {
        if slot.as_ref().is_some_and(|e| {
            !wants(e.stack.item) && (!content.items[e.stack.item].cursed || e.curse_released)
        }) && let Some(e) = slot.take()
        {
            returned.push(e.stack);
        }
    }
    for slot in &mut profile.carried.backpack {
        if slot.as_ref().is_some_and(|s| !wants(s.item))
            && let Some(s) = slot.take()
        {
            returned.push(s);
        }
    }
    let mut overflow = vec![];
    for item in returned {
        overflow.extend(inventory.pool.receive(item, &content.items));
    }
    crate::game::world::items::drop_loot(
        &mut commands,
        root,
        PartyBody::feet(at),
        overflow,
        &content.items,
    );
    if !v.is_dead() {
        let kind = &content.species[location.species];
        let mut kept = KeptMonster::rolled(
            (-1, location.species, None),
            (feet, facing.yaw),
            (kind, shape),
            &mut rng,
        );
        kept.vitals.conditions = v.conditions.clone();
        if v.conditions.exhausted() {
            kept.vitals.stamina.current = v.stamina.current.min(c.base_stats().max_stamina());
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
        commands.entity(actor).insert(NpcLocation {
            map: played.path.clone(),
            species: location.species,
            dormant: None,
        });
    }
    commands
        .entity(actor)
        .remove::<(PartyMember, Character, Vitals, Carried, LearnedSpells, SpellPoints)>()
        .insert(StoredCharacter(profile));
    true
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
        (Entity, &Character, &Vitals, &Carried, &LearnedSpells, &SpellPoints),
        (With<Npc>, With<PartyMember>, With<Dead>),
    >,
    mut party: Single<(&mut PartyInventory, &Position)>,
    level: Single<Entity, With<Level>>,
    content: If<Res<Catalogs>>,
    mut commands: Commands,
) {
    if !battles.is_empty() {
        return;
    }
    for (entity, character, vitals, carried, spells, points) in &members {
        let mut carried = carried.clone();
        let mut items = Vec::new();
        for slot in carried.equipment.values_mut() {
            if let Some(item) = slot.take() {
                items.push(item.stack);
            }
        }
        for slot in &mut carried.backpack {
            items.extend(slot.take());
        }
        let (inventory, at) = &mut *party;
        let mut overflow = vec![];
        for item in items {
            overflow.extend(inventory.pool.receive(item, &content.items));
        }
        crate::game::world::items::drop_loot(
            &mut commands,
            *level,
            PartyBody::feet(at),
            overflow,
            &content.items,
        );
        commands
            .entity(entity)
            .remove::<(PartyMember, Character, Vitals, Carried, LearnedSpells, SpellPoints)>()
            .insert(StoredCharacter(Profile {
                character: character.clone(),
                vitals: vitals.clone(),
                carried,
                spells: spells.clone(),
                points: points.clone(),
            }));
    }
}
