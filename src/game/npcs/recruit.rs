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
use crate::content::species::MonsterCatalog;
use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Condition, Lasting, Vitals};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot, Equipped, ItemIds, ItemStack, PartyInventory};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::body::PartyBody;
use crate::game::party::formation::Formation;
use crate::game::party::roster::{PartyMember, PartyRoster, the_party};
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

fn definition(world: &World, actor: Entity) -> Option<NpcDefinition> {
    let id = world.get::<super::ActorId>(actor)?.npc();
    let assets = world.resource::<super::NpcAssets>();
    world
        .resource::<Assets<crate::content::npcs::NpcCatalog>>()
        .get(&assets.catalog)?
        .get(id)
        .cloned()
}

/// Whether the NPC would join now (retail `CanNpcJoinParty`): it has a
/// character to bring; it does not offer the service of the region the party
/// is in; its authored terms hold (a term whose fact is not ported refuses);
/// and the party's living, able members average at least its minimum level.
fn willing(world: &mut World, id: NpcId, definition: &NpcDefinition) -> bool {
    if definition.character.is_none() {
        return false;
    }
    let band = world
        .query::<&Level>()
        .iter(world)
        .next()
        .and_then(|level| crate::content::levels::LevelInfo::for_path(&level.path))
        .map_or(0, |info| info.band);
    if offers_service(definition, band) {
        return false;
    }
    let fact = |world: &mut World, symbol: &str| super::conversation::fact_named(world, symbol);
    let terms = match id {
        GLUMPH => fact(world, "FACT_UMISSION_SCUBA_DONE").map(|done| done == 0),
        SEXUS => fact(world, "FACT_SEXUS_PAID").map(|paid| paid != 0),
        MADRAS => fact(world, "FACT_TRYNNIE_MADRAS_WILL_JOIN").map(|will| will != 0),
        DRAZIC | RODAN => fact(world, "FACT_PEACE_ACHIEVED").map(|peace| peace == 0),
        _ => Ok(true),
    };
    match terms {
        Ok(true) => {}
        Ok(false) => return false,
        Err(error) => {
            warn!("NPC {id:?} cannot weigh joining: {error}");
            return false;
        }
    }
    if definition.min_party_level > 0 {
        let Some(party) = the_party(world) else { return false };
        let levels: Vec<u32> = world
            .get::<PartyRoster>(party)
            .map(|roster| {
                roster
                    .iter()
                    .filter(|&member| world.get::<Vitals>(member).is_some_and(Vitals::can_fight))
                    .filter_map(|member| world.get::<Character>(member))
                    .map(|character| u32::from(character.level()))
                    .collect()
            })
            .unwrap_or_default();
        let average = levels.iter().sum::<u32>().checked_div(levels.len() as u32).unwrap_or(0);
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
    let id = world.get::<super::ActorId>(actor).ok_or(JoinRefusal::Declines)?.npc();
    let definition = definition(world, actor).ok_or(JoinRefusal::Declines)?;
    let gone = world.get::<Npc>(actor).is_some_and(|npc| npc.dead);
    if definition.character.is_none() || gone || world.get::<PartyMember>(actor).is_some() {
        return Err(JoinRefusal::Declines);
    }
    let party = the_party(world).ok_or(JoinRefusal::Declines)?;
    let npcs = world
        .get::<PartyRoster>(party)
        .map_or(0, |roster| roster.iter().filter(|&m| world.get::<Npc>(m).is_some()).count());
    if npcs >= NPC_PLACES {
        return Err(JoinRefusal::NoRoom);
    }
    if !willing(world, id, &definition) {
        return Err(JoinRefusal::Declines);
    }
    // The items it brings are part of its character: one the catalog lacks
    // is missing content, and it does not join without it.
    let template = definition.character.as_ref().unwrap();
    let complete = with_catalogs(world, |_, items, _, _| {
        template
            .equipment
            .iter()
            .chain(&template.backpack)
            .flatten()
            .all(|&id| items.get(id).is_some())
    });
    if complete != Some(true) {
        warn!("NPC {id:?} cannot join: its items are missing from the catalog");
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
            .filter(|&id| {
                spells.get(id).is_some_and(|spell| crate::game::magic::can_learn(&character, spell))
            })
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

/// Run `f` with the item, spell and species catalogs.
fn with_catalogs<T>(
    world: &mut World,
    f: impl FnOnce(&mut World, &ItemCatalog, &SpellCatalog, &MonsterCatalog) -> T,
) -> Option<T> {
    let data = world.get_resource::<GameData>()?;
    let (items, spells, species) = (data.items.clone(), data.spells.clone(), data.species.clone());
    world.resource_scope(|world, item_assets: Mut<Assets<ItemCatalog>>| {
        world.resource_scope(|world, spell_assets: Mut<Assets<SpellCatalog>>| {
            world.resource_scope(|world, species_assets: Mut<Assets<MonsterCatalog>>| {
                Some(f(
                    world,
                    item_assets.get(&items)?,
                    spell_assets.get(&spells)?,
                    species_assets.get(&species)?,
                ))
            })
        })
    })
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
    let party = the_party(world).ok_or(JoinRefusal::Declines)?;
    if world.get::<Character>(actor).is_none() {
        let template = definition(world, actor)
            .and_then(|definition| definition.character)
            .ok_or(JoinRefusal::Declines)?;
        let made = world.resource_scope(|world, mut ids: Mut<ItemIds>| {
            world.resource_scope(|world, mut rng: Mut<GameRng>| {
                with_catalogs(world, |_, items, spells, _| {
                    first_character(&template, (items, spells), (&mut ids, &mut rng))
                })
            })
        });
        let (character, vitals, learned, points, carried) = made.ok_or(JoinRefusal::Declines)?;
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
    let Some(party) = the_party(world) else { return };
    let feet = world.get::<avian3d::prelude::Position>(party).map_or(Vec3::ZERO, |at| {
        PartyBody::eye(at) - Vec3::Y * crate::game::party::body::EYE_HEIGHT
    });
    // Nothing leaves the member unless the catalog to place it with is there.
    with_catalogs(world, |world, items, _, _| {
        let given: Vec<ItemStack> = {
            let Some(mut carried) = world.get_mut::<Carried>(actor) else { return };
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

/// What a member's parting needs before anything about it changes: the
/// catalogs, and the level being played, where items the party's pool cannot
/// hold are dropped. A body that will stand again also needs its species.
fn ready_to_part(world: &mut World, body: Option<Entity>) -> Option<Entity> {
    let species = body.and_then(|body| world.get::<Monster>(body)).map(|m| m.species);
    with_catalogs(world, |_, _, _, catalog| species.is_none_or(|s| catalog.kind(s).is_some()))?
        .then_some(())?;
    world.query_filtered::<Entity, With<Level>>().iter(world).next()
}

/// Let the NPC leave the party (retail `DismissNpcFromParty`, its body not
/// held back): it gives up its NPC place; alive, it stands again beside the
/// party with the conditions it has, at a fresh roll of its species' health;
/// and it gives the party every item it neither wants nor has cursed on.
pub fn dismiss(world: &mut World, actor: Entity) -> bool {
    let Some(party) = the_party(world) else { return false };
    if world.get::<PartyMember>(actor).is_none_or(|member| member.0 != party)
        || world.get::<Npc>(actor).is_none()
    {
        return false;
    }
    let alive = world.get::<Vitals>(actor).is_some_and(|vitals| !vitals.is_dead());
    let body = world.get::<NpcBodies>(actor).and_then(|bodies| bodies.first().copied());
    let Some(level) = ready_to_part(world, body.filter(|_| alive)) else { return false };
    world.entity_mut(actor).remove::<PartyMember>();
    world.get_mut::<Formation>(party).unwrap().unseat(actor);
    if alive && let Some(body) = body {
        stand_beside_party(world, actor, body, party);
    }
    let wanted = definition(world, actor)
        .and_then(|definition| definition.character)
        .map_or_else(Vec::new, |character| character.wanted);
    let cursed = |stack: &ItemStack| {
        world.get_resource::<GameData>().and_then(|data| {
            world
                .resource::<Assets<ItemCatalog>>()
                .get(&data.items)?
                .get(stack.item)
                .map(|d| d.cursed)
        })
    };
    let held: Vec<_> = world
        .get::<Carried>(actor)
        .map(|carried| {
            carried
                .active_equipment()
                .filter(|(_, e)| !e.curse_released && cursed(&e.stack).unwrap_or(false))
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
fn stand_beside_party(world: &mut World, actor: Entity, body: Entity, party: Entity) {
    let Some((root, map)) =
        world.query::<(Entity, &Level)>().iter(world).next().map(|(e, l)| (e, l.path.clone()))
    else {
        return;
    };
    let Some((at, yaw)) = world
        .get::<avian3d::prelude::Position>(party)
        .zip(world.get::<PartyBody>(party))
        .map(|(at, facing)| (*at, facing.yaw))
    else {
        return;
    };
    // Retail asks the navigator for a place out from the party along its
    // heading turned by an eighth of a turn; this stands it two metres out
    // that way, without the navigator's search.
    let feet = PartyBody::eye(&at) - Vec3::Y * crate::game::party::body::EYE_HEIGHT
        + Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
    let (conditions, stamina) = {
        let vitals = world.get::<Vitals>(actor).unwrap();
        (vitals.conditions.clone(), vitals.stamina)
    };
    let stamina_max = world.get::<Character>(actor).map_or(stamina, Character::max_stamina);
    let Some(species) = world.get::<Monster>(body).map(|m| m.species) else { return };
    let rolled = world.resource_scope(|world, mut rng: Mut<GameRng>| {
        with_catalogs(world, |_, _, _, catalog| {
            let kind = catalog.kind(species)?;
            Some((rng.roll(kind.hit_points).max(1), rng.roll(kind.stamina)))
        })
        .flatten()
    });
    let Some((max_hp, max_stamina)) = rolled else { return };
    {
        let mut monster = world.get_mut::<Monster>(body).unwrap();
        monster.hp = max_hp;
        monster.max_hp = max_hp;
        monster.max_stamina = max_stamina;
        monster.stamina = max_stamina;
        // An NPC out cold stays as spent as it was.
        if conditions.contains(&(Condition::Unconscious, Lasting::Indefinite)) {
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
    let Some(party) = the_party(world) else { return };
    let fallen: Vec<Entity> = world
        .get::<PartyRoster>(party)
        .map(|roster| {
            roster
                .iter()
                .filter(|&member| {
                    world.get::<Npc>(member).is_some()
                        && world.get::<Vitals>(member).is_some_and(Vitals::is_dead)
                })
                .collect()
        })
        .unwrap_or_default();
    if fallen.is_empty() {
        return;
    }
    let Some(level) = ready_to_part(world, None) else { return };
    for member in fallen {
        world.entity_mut(member).remove::<PartyMember>();
        world.get_mut::<Formation>(party).unwrap().unseat(member);
        world.get_mut::<Npc>(member).unwrap().dead = true;
        hand_over(world, member, level, |_| false);
    }
}
