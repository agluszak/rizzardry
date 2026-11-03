//! NPCs joining and leaving the party (retail `CanNpcJoinParty`,
//! `RecruitNpcIntoParty`, `DismissNpcFromParty`, `UpdateNpcPartyMember`).
//! The NPC's actor is the member: its identity, character and belongings
//! last through leaving and rejoining. Its world body waits, dormant, while
//! it travels with the party, and stands again where it leaves.

use avian3d::prelude::Position;
use bevy::ecs::entity_disabling::Disabled;
use bevy::prelude::*;

use super::{Npc, NpcBodies, NpcWorldRole};
use super::{
    access::NpcActors,
    story::{StoryFacts, fact_named},
};
use crate::content::items::ItemCatalog;
use crate::content::npcs::{NpcCharacter, NpcDefinition, NpcId};
use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::inventory::{Carried, EquipSlot, Equipped, ItemIds, ItemStack, PartyInventory};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::level::{Level, LevelOwned};
use crate::game::world::navigation::{LevelNavigation, WalkerMotionConfig};
use crate::game::world::population::Monster;
use bevy::ecs::system::SystemParam;

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
pub fn eligibility(
    id: NpcId,
    definition: &NpcDefinition,
    dead: bool,
    already_member: bool,
    occupied: usize,
    region: u8,
    levels: impl Iterator<Item = u8>,
    fact: impl Fn(&str) -> u8,
) -> Result<(), JoinRefusal> {
    if definition.character.is_none() || dead || already_member {
        return Err(JoinRefusal::Declines);
    }
    if occupied >= NPC_PLACES {
        return Err(JoinRefusal::NoRoom);
    }
    let terms = match id {
        GLUMPH => fact("FACT_UMISSION_SCUBA_DONE") == 0,
        SEXUS => fact("FACT_SEXUS_PAID") != 0,
        MADRAS => fact("FACT_TRYNNIE_MADRAS_WILL_JOIN") != 0,
        DRAZIC | RODAN => fact("FACT_PEACE_ACHIEVED") == 0,
        _ => true,
    };
    let (sum, count) =
        levels.fold((0u32, 0u32), |(sum, count), level| (sum + u32::from(level), count + 1));
    if !terms
        || offers_service(definition, region)
        || sum / count.max(1) < u32::from(definition.min_party_level)
    {
        return Err(JoinRefusal::Declines);
    }
    Ok(())
}

/// Only the inputs to recruitment policy. No conversation is required.
#[derive(SystemParam)]
pub struct Candidates<'w, 's> {
    npcs: Query<'w, 's, (&'static super::ActorId, &'static Npc)>,
    content: Res<'w, Catalogs>,
    level: Single<'w, 's, &'static Level>,
    party: Single<'w, 's, (Entity, &'static PartyRoster)>,
    members: Query<'w, 's, (&'static Character, &'static Vitals), Without<Monster>>,
    story: Res<'w, StoryFacts>,
}
impl Candidates<'_, '_> {
    pub fn check(&self, actor: Entity) -> Result<(), JoinRefusal> {
        let (id, npc) = self.npcs.get(actor).map_err(|_| JoinRefusal::Declines)?;
        let (_, roster) = *self.party;
        let region =
            crate::content::levels::LevelInfo::for_path(&self.level.path).map_or(0, |l| l.band);
        eligibility(
            id.npc(),
            &self.content.npcs[id.npc()],
            npc.dead,
            roster.contains(&actor),
            roster.iter().filter(|&m| self.npcs.contains(m)).count(),
            region,
            roster
                .iter()
                .filter_map(|m| self.members.get(m).ok())
                .filter(|(_, v)| v.can_fight())
                .map(|(c, _)| c.level()),
            |name| {
                fact_named(&self.story, &self.content.facts, name)
                    .expect("a ported recruitment fact")
            },
        )
    }
}
pub fn joinable(In(actor): In<Entity>, candidates: Candidates) -> Result<(), JoinRefusal> {
    candidates.check(actor)
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
    let mut make = |id| crate::game::inventory::make_item(&items[id], ids, rng, true, true);
    for (slot, id) in template.equipment.iter().enumerate() {
        if let Some(stack) = id.map(&mut make) {
            carried.equipment[EquipSlot::from_usize(slot)] = Some(Equipped::new(stack));
        }
    }
    for (slot, id) in template.backpack.iter().enumerate() {
        carried.backpack[slot] = id.map(&mut make);
    }
    let points = SpellPoints::default();
    let vitals = Vitals::new(&character);
    (character, vitals, learned, points, carried)
}

/// Take the NPC into the party (retail `RecruitNpcIntoParty`): it brings
/// its character (made the first time), the conditions its world body had,
/// and takes an NPC place; its world body goes dormant.
pub fn join(
    In(actor): In<Entity>,
    candidates: Candidates,
    actors: NpcActors,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) -> Result<(), JoinRefusal> {
    candidates.check(actor)?;
    let body = actors.active_body(actor);
    let conditions = body.and_then(|body| actors.vitals(body)).map(|v| v.conditions.clone());
    if let Ok((_, vitals)) = candidates.members.get(actor) {
        let mut vitals = vitals.clone();
        if let Some(conditions) = conditions {
            vitals.conditions = conditions;
        }
        commands.entity(actor).insert(vitals);
    } else {
        let content = &candidates.content;
        let template = actors.definition(actor).character.as_ref().unwrap();
        let (character, mut vitals, learned, points, carried) =
            first_character(template, (&content.items, &content.spells), (&mut ids, &mut rng));
        if let Some(conditions) = conditions {
            vitals.conditions = conditions;
        }
        commands.entity(actor).insert((
            Name::new(character.name.clone()),
            character,
            vitals,
            learned,
            points,
            carried,
        ));
    }
    if let Some(body) = body
        && let Some(at) = actors.standing_pose(body)
    {
        super::retire(&mut commands, body, at);
    }
    commands
        .entity(actor)
        .insert((PartyMember(candidates.party.0), crate::game::party::Order::Attack));
    Ok(())
}

/// Give a departing member's items to the party, those the pool has no room
/// for dropped at its feet. `keep` says which the member keeps.
fn hand_over(
    carried: &mut Carried,
    inventory: &mut PartyInventory,
    items: &ItemCatalog,
    commands: &mut Commands,
    (level, feet): (Entity, Vec3),
    keep: impl Fn(&ItemStack) -> bool,
) {
    let worn = carried
        .equipment
        .values_mut()
        .filter_map(|worn| worn.take_if(|e| !keep(&e.stack)).map(|e| e.stack));
    let mut given: Vec<_> = worn.collect();
    given.extend(carried.backpack.iter_mut().filter_map(|place| place.take_if(|s| !keep(s))));
    let mut left = Vec::new();
    for stack in given {
        left.extend(inventory.pool.receive(stack, items));
    }
    crate::game::world::items::drop_loot(commands, level, feet, left, items);
}

/// Let the NPC leave the party (retail `DismissNpcFromParty`, its body not
/// held back): it gives up its NPC place; alive, it stands again beside the
/// party with the conditions it has, at a fresh roll of its species' health;
/// and it gives the party every item it neither wants nor has cursed on.
/// Whether it was a member to dismiss.
#[allow(clippy::too_many_arguments)]
pub fn dismiss(
    In(actor): In<Entity>,
    mut party: Single<(&PartyRoster, &mut PartyInventory, &Position, &PartyBody)>,
    actors: Query<(&NpcBodies, &super::ActorId), With<Npc>>,
    mut characters: Query<(&Character, &Vitals, &mut Carried), Without<Monster>>,
    mut bodies: Query<(&mut Vitals, &Monster, &mut NpcWorldRole), (With<Monster>, Allow<Disabled>)>,
    level: Single<(Entity, &Level, Option<&LevelNavigation>)>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    mut commands: Commands,
) -> bool {
    let (roster, inventory, at, facing) = &mut *party;
    let Ok((npc_bodies, id)) = actors.get(actor) else { return false };
    if !roster.contains(&actor) {
        return false;
    }
    let Ok((character, vitals, _)) = characters.get(actor) else { return false };
    let alive = !vitals.is_dead();
    let (conditions, stamina) = (vitals.conditions.clone(), vitals.stamina.current);
    let stamina_max = character.base_stats().max_stamina();
    let (root, played, navigation) = *level;
    commands.entity(actor).remove::<PartyMember>();
    if alive
        && let Some(&body) = npc_bodies.first()
        && let Ok((mut monster_vitals, monster, mut role)) = bodies.get_mut(body)
    {
        // Retail asks the navigator for a place out from the party along its
        // heading turned by an eighth of a turn; this stands it two metres out
        // that way, without the navigator's search.
        let yaw = facing.yaw;
        let feet = PartyBody::feet(at)
            + Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
        let kind = &content.species[monster.species];
        monster_vitals.health =
            crate::game::characters::Pool::full(rng.roll(kind.hit_points).max(1));
        monster_vitals.stamina = crate::game::characters::Pool::full(rng.roll(kind.stamina));
        // An NPC out cold stays as spent as it was.
        if conditions.exhausted() {
            monster_vitals.stamina.current = stamina.min(stamina_max);
        }
        monster_vitals.conditions = conditions;
        role.map = played.path.clone();
        role.group = None;
        let (speed, radius) = (monster.body.speed, monster.body.radius);
        let mut entity = commands.entity(body);
        entity.remove::<Disabled>().insert((
            Transform::from_translation(
                feet + Vec3::Y * crate::game::world::navigation::float_height(speed),
            )
            .with_rotation(Quat::from_rotation_y(yaw)),
            LevelOwned(root),
        ));
        if speed > 0.0
            && let Some(navigation) = navigation
        {
            crate::game::world::navigation::insert_walker(
                &mut entity,
                (feet, yaw),
                (radius, speed),
                navigation,
                &mut configs,
            );
        }
    }
    let definition = &content.npcs[id.npc()];
    let wanted =
        definition.character.as_ref().map_or_else(Vec::new, |character| character.wanted.clone());
    let items = &content.items;
    let Ok((_, _, mut carried)) = characters.get_mut(actor) else { return true };
    let held: Vec<_> = carried
        .active_equipment()
        .filter(|(_, e)| !e.curse_released && items[e.stack.item].cursed)
        .map(|(_, e)| e.stack.id)
        .collect();
    hand_over(
        &mut carried,
        inventory,
        items,
        &mut commands,
        (root, PartyBody::feet(at)),
        |stack| wanted.contains(&stack.item) || held.contains(&stack.id),
    );
    true
}

/// A member NPC that dies leaves the party at once (retail
/// `UpdateNpcPartyMember`): its items go to the party, it is dead for good
/// (retail `spawned`), and it never stands in the world again.
pub fn npc_member_falls(
    death: On<Add<crate::game::characters::Dead>>,
    mut members: Query<(&PartyMember, &mut Npc, &mut Carried)>,
    mut parties: Query<(&mut PartyInventory, &Position)>,
    level: Single<Entity, With<Level>>,
    content: Res<Catalogs>,
    mut commands: Commands,
) {
    let member = death.entity;
    let Ok((&PartyMember(party), mut npc, mut carried)) = members.get_mut(member) else { return };
    let Ok((mut inventory, at)) = parties.get_mut(party) else { return };
    commands.entity(member).remove::<PartyMember>();
    npc.dead = true;
    let feet = PartyBody::feet(at);
    hand_over(&mut carried, &mut inventory, &content.items, &mut commands, (*level, feet), |_| {
        false
    });
}
