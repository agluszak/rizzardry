//! NPCs joining and leaving the party (retail `CanNpcJoinParty`,
//! `RecruitNpcIntoParty`, `DismissNpcFromParty`, `UpdateNpcPartyMember`).
//! The NPC's actor is the member: its identity, character and belongings
//! last through leaving and rejoining. Its world body waits, dormant, while
//! it travels with the party, and stands again where it leaves.

use bevy::ecs::entity_disabling::Disabled;
use bevy::prelude::*;

use super::{Npc, NpcBodies, NpcWorldRole};
use crate::content::items::ItemCatalog;
use crate::content::npcs::{NpcCharacter, NpcDefinition, NpcId};
use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::{Carried, EquipSlot, Equipped, ItemIds, ItemStack, PartyInventory};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::body::PartyBody;
use crate::game::party::formation::Formation;
use crate::game::party::roster::{Party, PartyMember, PartyRoster, the_party};
use crate::game::rng::GameRng;
use crate::game::world::level::{Level, LevelOwned};
use crate::game::world::population::Monster;

/// The party's places for NPCs (retail party rows 0 and 1).
pub const NPC_PLACES: usize = 2;

/// Why an NPC does not join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinRefusal {
    /// Both of the party's NPC places are taken.
    NoRoom,
    /// The NPC will not, or cannot, join.
    Declines,
}

/// Retail's service table (`g_npc_services`): each region whose service an
/// NPC may offer, and the NPC's flag for it.
const SERVICES: [(u8, u32); 14] = [
    (2, 0x1),
    (3, 0x2),
    (4, 0x400),
    (5, 0x4),
    (7, 0x8),
    (8, 0x80),
    (9, 0x40),
    (10, 0x20),
    (11, 0x10),
    (12, 0x100),
    (13, 0x800),
    (14, 0x200),
    (15, 0x1000),
    (6, 0x2000),
];

/// Whether the NPC offers the service of the region `band` (retail
/// `NpcOffersService`).
fn offers_service(definition: &NpcDefinition, band: u8) -> bool {
    SERVICES
        .iter()
        .find(|&&(region, _)| region == band)
        .is_some_and(|&(_, flag)| definition.services & flag != 0)
}

/// Retail's NPCs with authored joining terms (`W8NpcId`).
const DRAZIC: NpcId = NpcId(0x10);
const RODAN: NpcId = NpcId(0x11);
const GLUMPH: NpcId = NpcId(0x2b);
const MADRAS: NpcId = NpcId(0x4a);
const SEXUS: NpcId = NpcId(0x4f);

/// Whether the NPC would join now (retail `CanNpcJoinParty`): it does not
/// offer the service of the region the party is in; its authored terms
/// hold; and the party's living, able members average at least its minimum
/// level.
fn willing(world: &mut World, id: NpcId, definition: &NpcDefinition) -> bool {
    let level = world.query::<&Level>().single(world).unwrap();
    let band = crate::content::levels::LevelInfo::for_path(&level.path).map_or(0, |info| info.band);
    if offers_service(definition, band) {
        return false;
    }
    let fact = |world: &mut World, symbol: &str| {
        super::conversation::fact_named(world, symbol).expect("a ported fact")
    };
    let terms = match id {
        GLUMPH => fact(world, "FACT_UMISSION_SCUBA_DONE") == 0,
        SEXUS => fact(world, "FACT_SEXUS_PAID") != 0,
        MADRAS => fact(world, "FACT_TRYNNIE_MADRAS_WILL_JOIN") != 0,
        DRAZIC | RODAN => fact(world, "FACT_PEACE_ACHIEVED") == 0,
        _ => true,
    };
    if !terms {
        return false;
    }
    if definition.min_party_level > 0 {
        // Retail counts no able member as an average of nothing.
        let levels = super::able_levels(world);
        let average = if levels.is_empty() { 0 } else { crate::game::party::average_level(levels) };
        if average < u32::from(definition.min_party_level) {
            return false;
        }
    }
    true
}

/// Whether the NPC would join the party now, and if not, why (retail
/// `RequestNpcJoinParty`'s order: an NPC without a character declines
/// before the party's room is counted).
pub fn joinable(world: &mut World, actor: Entity) -> Result<(), JoinRefusal> {
    let id = world.get::<super::ActorId>(actor).unwrap().npc();
    let definition = super::definition(world, actor).clone();
    let gone = world.get::<Npc>(actor).is_some_and(|npc| npc.dead);
    if definition.character.is_none() || gone || world.get::<PartyMember>(actor).is_some() {
        return Err(JoinRefusal::Declines);
    }
    let party = the_party(world);
    let npcs = world
        .get::<PartyRoster>(party)
        .map_or(0, |roster| roster.iter().filter(|&m| world.get::<Npc>(m).is_some()).count());
    if npcs >= NPC_PLACES {
        return Err(JoinRefusal::NoRoom);
    }
    if !willing(world, id, &definition) {
        return Err(JoinRefusal::Declines);
    }
    Ok(())
}

/// The character an NPC brings the first time it joins (retail
/// `InitializeNpcCharacter`): the template's identity, attributes, skills
/// and level, with the experience that level took; the spells it lists
/// that it can learn; its items made and identified; and full health,
/// stamina and spell points.
fn first_character(
    template: &NpcCharacter,
    (items, spells): (&ItemCatalog, &SpellCatalog),
    (ids, rng): (&mut ItemIds, &mut GameRng),
) -> (Character, Vitals, LearnedSpells, SpellPoints, Carried) {
    use enum_map::Enum;
    let mut character =
        Character::new(&template.name, template.race, template.gender, template.profession);
    character.attributes = template.attributes;
    character.skill_points = template.skills;
    character.profession_levels = default();
    character.profession_levels[template.profession] = 1;
    // Walked up from level 1, as retail `AdvanceCharacterToLevel` does.
    character.experience_previous_goal = 0;
    while character.level() < template.level {
        character.experience_previous_goal = character.experience_goal();
        character.profession_levels[template.profession] += 1;
    }
    character.experience = character.experience_previous_goal;
    character.portrait = crate::game::characters::creation::identity::PORTRAITS
        .get(template.portrait)
        .map_or_else(String::new, |&portrait| portrait.to_owned());
    let learned = LearnedSpells(
        template
            .spells
            .iter()
            .copied()
            .filter(|&id| crate::game::magic::can_learn(&character, &spells[id]))
            .collect(),
    );
    let mut carried = Carried::default();
    let mut make = |id| {
        items
            .get(id)
            .map(|definition| crate::game::inventory::make_item(definition, ids, rng, true, true))
    };
    for (slot, id) in template.equipment.iter().enumerate() {
        if let Some(stack) = id.and_then(&mut make) {
            carried.equipment[EquipSlot::from_usize(slot)] = Some(Equipped::new(stack));
        }
    }
    for (slot, id) in template.backpack.iter().enumerate() {
        carried.backpack[slot] = id.and_then(&mut make);
    }
    let points = SpellPoints::default();
    let vitals = Vitals::new(&character);
    (character, vitals, learned, points, carried)
}

/// The NPC's world body that stands in a played level.
fn standing_body(world: &World, actor: Entity) -> Option<Entity> {
    world.get::<NpcBodies>(actor)?.iter().find(|&body| {
        world.get::<LevelOwned>(body).is_some() && world.get::<Disabled>(body).is_none()
    })
}

/// Take the NPC into the party (retail `RecruitNpcIntoParty`): it brings
/// its character (made the first time), the conditions its world body had,
/// and takes an NPC place; its world body goes dormant.
pub fn join(world: &mut World, actor: Entity) -> Result<(), JoinRefusal> {
    joinable(world, actor)?;
    let party = the_party(world);
    if world.get::<Character>(actor).is_none() {
        let template = super::definition(world, actor).character.clone().unwrap();
        let (character, vitals, learned, points, carried) =
            world.resource_scope(|world, mut ids: Mut<ItemIds>| {
                world.resource_scope(|world, mut rng: Mut<GameRng>| {
                    let catalogs = (
                        &world.resource::<crate::game::data::Catalogs>().items,
                        &world.resource::<crate::game::data::Catalogs>().spells,
                    );
                    first_character(&template, catalogs, (&mut ids, &mut rng))
                })
            });
        world.entity_mut(actor).insert((
            Name::new(character.name.clone()),
            character,
            vitals,
            learned,
            points,
            carried,
        ));
    }
    if let Some(body) = standing_body(world, actor) {
        let conditions =
            world.get::<Monster>(body).map(|m| m.conditions.clone()).unwrap_or_default();
        world.get_mut::<Vitals>(actor).unwrap().conditions = conditions;
        let at = world
            .query::<(crate::game::world::ActorPose, Option<&crate::game::world::navigation::Walker>)>()
            .get(world, body)
            .map(|(pose, walker)| {
                Transform::from_translation(crate::game::world::navigation::feet(
                    pose.translation(),
                    walker,
                ))
                .with_rotation(pose.rotation())
            })
            .unwrap_or_default();
        super::retire(&mut world.commands(), body, at);
        world.flush();
    }
    world.entity_mut(actor).insert((PartyMember(party), crate::game::party::Order::Attack));
    world.get_mut::<Formation>(party).unwrap().seat(actor);
    Ok(())
}

/// Give a departing member's items to the party, those the pool has no room
/// for dropped at its feet. `keep` says which the member keeps.
fn hand_over(world: &mut World, actor: Entity, level: Entity, keep: impl Fn(&ItemStack) -> bool) {
    let party = the_party(world);
    let feet = PartyBody::feet_in(world);
    Catalogs::scope(world, |world, catalogs| {
        let items = &catalogs.items;
        let given: Vec<ItemStack> = {
            let mut carried = world.get_mut::<Carried>(actor).unwrap();
            let carried = &mut *carried;
            let worn = carried
                .equipment
                .values_mut()
                .filter_map(|worn| worn.take_if(|e| !keep(&e.stack)).map(|e| e.stack));
            let mut given: Vec<_> = worn.collect();
            given.extend(
                carried.backpack.iter_mut().filter_map(|place| place.take_if(|s| !keep(s))),
            );
            given
        };
        let mut left = Vec::new();
        {
            let mut inventory = world.get_mut::<PartyInventory>(party).unwrap();
            for stack in given {
                left.extend(inventory.pool.receive(stack, items));
            }
        }
        crate::game::world::items::drop_loot(&mut world.commands(), level, feet, left, items);
        world.flush();
    });
}

/// The level being played, where items a parting member leaves that the
/// party's pool cannot hold are dropped.
fn played_level(world: &mut World) -> Entity {
    world.query_filtered::<Entity, With<Level>>().single(world).unwrap()
}

/// Let the NPC leave the party (retail `DismissNpcFromParty`, its body not
/// held back): it gives up its NPC place; alive, it stands again beside the
/// party with the conditions it has, at a fresh roll of its species' health;
/// and it gives the party every item it neither wants nor has cursed on.
pub fn dismiss(world: &mut World, actor: Entity) -> bool {
    let party = the_party(world);
    if world.get::<PartyMember>(actor).is_none_or(|member| member.0 != party)
        || world.get::<Npc>(actor).is_none()
    {
        return false;
    }
    let alive = !world.get::<Vitals>(actor).unwrap().is_dead();
    let body = world.get::<NpcBodies>(actor).and_then(|bodies| bodies.first().copied());
    let level = played_level(world);
    world.entity_mut(actor).remove::<PartyMember>();
    world.get_mut::<Formation>(party).unwrap().unseat(actor);
    if alive && let Some(body) = body {
        stand_beside_party(world, actor, body, (party, level));
    }
    let wanted = super::definition(world, actor)
        .character
        .as_ref()
        .map_or_else(Vec::new, |character| character.wanted.clone());
    let cursed = |stack: &ItemStack| {
        world.resource::<crate::game::data::Catalogs>().items[stack.item].cursed
    };
    let held: Vec<_> = world
        .get::<Carried>(actor)
        .map(|carried| {
            carried
                .active_equipment()
                .filter(|(_, e)| !e.curse_released && cursed(&e.stack))
                .map(|(_, e)| e.stack.id)
                .collect()
        })
        .unwrap_or_default();
    hand_over(world, actor, level, |stack| {
        wanted.contains(&stack.item) || held.contains(&stack.id)
    });
    true
}

/// The NPC's dormant body stands again beside the party, on its level.
fn stand_beside_party(
    world: &mut World,
    actor: Entity,
    body: Entity,
    (party, root): (Entity, Entity),
) {
    let map = world.get::<Level>(root).unwrap().path.clone();
    let at = *world.get::<avian3d::prelude::Position>(party).unwrap();
    let yaw = world.get::<PartyBody>(party).unwrap().yaw;
    // Retail asks the navigator for a place out from the party along its
    // heading turned by an eighth of a turn; this stands it two metres out
    // that way, without the navigator's search.
    let feet = PartyBody::feet(&at)
        + Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
    let (conditions, stamina) = {
        let vitals = world.get::<Vitals>(actor).unwrap();
        (vitals.conditions.clone(), vitals.stamina)
    };
    let stamina_max = world.get::<Character>(actor).unwrap().base_stats().max_stamina();
    let species = world.get::<Monster>(body).unwrap().species;
    let (max_hp, max_stamina) = world.resource_scope(|world, mut rng: Mut<GameRng>| {
        let kind = &world.resource::<crate::game::data::Catalogs>().species[species];
        (rng.roll(kind.hit_points).max(1), rng.roll(kind.stamina))
    });
    {
        let mut monster = world.get_mut::<Monster>(body).unwrap();
        monster.hp = max_hp;
        monster.max_hp = max_hp;
        monster.max_stamina = max_stamina;
        monster.stamina = max_stamina;
        // An NPC out cold stays as spent as it was.
        if conditions.exhausted() {
            monster.stamina = stamina.min(stamina_max);
        }
        monster.conditions = conditions;
    }
    {
        let mut role = world.get_mut::<NpcWorldRole>(body).unwrap();
        role.map = map;
        role.group = None;
    }
    let speed = world.get::<Monster>(body).unwrap().body.speed;
    let radius = world.get::<Monster>(body).unwrap().body.radius;
    world.resource_scope(
        |world, mut configs: Mut<Assets<crate::game::world::navigation::WalkerMotionConfig>>| {
            let navigation = world
                .get::<crate::game::world::navigation::LevelNavigation>(root)
                .map(|n| n.archipelagos.clone());
            let mut commands = world.commands();
            let mut entity = commands.entity(body);
            entity.remove::<Disabled>().insert((
                Transform::from_translation(
                    feet + Vec3::Y * crate::game::world::navigation::float_height(speed),
                )
                .with_rotation(Quat::from_rotation_y(yaw)),
                LevelOwned(root),
            ));
            if speed > 0.0
                && let Some(archipelagos) = navigation
            {
                crate::game::world::navigation::insert_walker(
                    &mut entity,
                    (feet, yaw),
                    (radius, speed),
                    &crate::game::world::navigation::LevelNavigation { archipelagos },
                    &mut configs,
                );
            }
        },
    );
    world.flush();
}

/// A member NPC that dies leaves the party at once (retail
/// `UpdateNpcPartyMember`): its items go to the party, it is dead for good
/// (retail `spawned`), and it never stands in the world again.
pub fn settle_npc_members(world: &mut World) {
    // Before a campaign begins there is no party to settle.
    let Ok(party) = world.query_filtered::<Entity, With<Party>>().single(world) else { return };
    let fallen: Vec<Entity> = world
        .get::<PartyRoster>(party)
        .unwrap()
        .iter()
        .filter(|&member| {
            world.get::<Npc>(member).is_some() && world.get::<Vitals>(member).unwrap().is_dead()
        })
        .collect();
    if fallen.is_empty() {
        return;
    }
    let level = played_level(world);
    for member in fallen {
        world.entity_mut(member).remove::<PartyMember>();
        world.get_mut::<Formation>(party).unwrap().unseat(member);
        world.get_mut::<Npc>(member).unwrap().dead = true;
        hand_over(world, member, level, |_| false);
    }
}
