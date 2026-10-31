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
use crate::game::party::roster::{Party, PartyMember, PartyRoster};
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
/// offer the service of the party's `region`; its authored terms hold, by
/// the story's facts; and the party's living, able members average at least
/// its minimum level.
pub(super) fn willing(
    id: NpcId,
    definition: &NpcDefinition,
    region: u8,
    fact: impl Fn(&str) -> u8,
    average: u32,
) -> bool {
    if offers_service(definition, region) {
        return false;
    }
    let terms = match id {
        GLUMPH => fact("FACT_UMISSION_SCUBA_DONE") == 0,
        SEXUS => fact("FACT_SEXUS_PAID") != 0,
        MADRAS => fact("FACT_TRYNNIE_MADRAS_WILL_JOIN") != 0,
        DRAZIC | RODAN => fact("FACT_PEACE_ACHIEVED") == 0,
        _ => true,
    };
    terms && average >= u32::from(definition.min_party_level)
}

/// [`super::conversation::Dialogue::joinable`], for operations that hold
/// the whole world.
pub fn joinable(world: &mut World, actor: Entity) -> Result<(), JoinRefusal> {
    world
        .run_system_cached_with(
            |In(actor): In<Entity>, dialogue: super::conversation::Dialogue| {
                dialogue.joinable(actor)
            },
            actor,
        )
        .unwrap()
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

/// What joining and leaving touch: the party with its pool, roster and
/// formation, the NPC actors that are or may become members, their world
/// bodies, dormant or standing, and the played level.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Recruits<'w, 's> {
    party: Single<
        'w,
        's,
        (
            Entity,
            &'static mut PartyInventory,
            &'static PartyRoster,
            &'static mut Formation,
            &'static avian3d::prelude::Position,
            &'static PartyBody,
        ),
        With<Party>,
    >,
    actors: Query<
        'w,
        's,
        (
            &'static mut Npc,
            &'static super::ActorId,
            Option<&'static NpcBodies>,
            Option<&'static PartyMember>,
            Option<&'static Character>,
            Option<&'static mut Vitals>,
            Option<&'static mut Carried>,
        ),
    >,
    bodies: Query<
        'w,
        's,
        (
            &'static mut Monster,
            &'static mut NpcWorldRole,
            crate::game::world::ActorPose,
            Option<&'static crate::game::world::navigation::Walker>,
            Has<LevelOwned>,
            Has<Disabled>,
        ),
        Allow<Disabled>,
    >,
    levels: Query<
        'w,
        's,
        (Entity, &'static Level, Option<&'static crate::game::world::navigation::LevelNavigation>),
    >,
    content: If<Res<'w, Catalogs>>,
    ids: ResMut<'w, ItemIds>,
    rng: ResMut<'w, GameRng>,
    configs: ResMut<'w, Assets<crate::game::world::navigation::WalkerMotionConfig>>,
    commands: Commands<'w, 's>,
}

impl Recruits<'_, '_> {
    /// Take the NPC into the party (retail `RecruitNpcIntoParty`): it brings
    /// its character (made the first time), the conditions its world body
    /// had, and takes an NPC place; its world body goes dormant. Whether it
    /// would join is the caller's to have asked.
    fn join(&mut self, actor: Entity) {
        let party = self.party.0;
        let (_, id, bodies, _, character, ..) = self.actors.get(actor).unwrap();
        // The NPC's world body that stands in a played level.
        let standing = bodies.into_iter().flat_map(|b| b.iter()).find(|&body| {
            self.bodies.get(body).is_ok_and(|(.., owned, disabled)| owned && !disabled)
        });
        let conditions = standing.map(|body| self.bodies.get(body).unwrap().0.conditions.clone());
        if character.is_none() {
            let template = self.content.npcs[id.npc()].character.as_ref().unwrap();
            let catalogs = (&self.content.items, &self.content.spells);
            let (character, mut vitals, learned, points, carried) =
                first_character(template, catalogs, (&mut self.ids, &mut self.rng));
            if let Some(conditions) = conditions {
                vitals.conditions = conditions;
            }
            self.commands.entity(actor).insert((
                Name::new(character.name.clone()),
                character,
                vitals,
                learned,
                points,
                carried,
            ));
        } else if let Some(conditions) = conditions {
            self.actors.get_mut(actor).unwrap().5.unwrap().conditions = conditions;
        }
        if let Some(body) = standing {
            let (_, _, pose, walker, ..) = self.bodies.get(body).unwrap();
            let at = Transform::from_translation(crate::game::world::navigation::feet(
                pose.translation(),
                walker,
            ))
            .with_rotation(pose.rotation());
            super::retire(&mut self.commands, body, at);
        }
        self.commands.entity(actor).insert((PartyMember(party), crate::game::party::Order::Attack));
        self.party.3.seat(actor);
    }

    /// Let the NPC leave the party (retail `DismissNpcFromParty`, its body
    /// not held back): it gives up its NPC place; alive, it stands again
    /// beside the party with the conditions it has, at a fresh roll of its
    /// species' health; and it gives the party every item it neither wants
    /// nor has cursed on.
    fn dismiss(&mut self, actor: Entity) -> bool {
        let party = self.party.0;
        let Ok((_, id, bodies, member, character, vitals, carried)) = self.actors.get(actor) else {
            return false;
        };
        if member.is_none_or(|member| member.0 != party) {
            return false;
        }
        let wanted = self.content.npcs[id.npc()]
            .character
            .as_ref()
            .map_or_else(Vec::new, |character| character.wanted.clone());
        let items = &self.content.items;
        let held: Vec<_> = carried
            .map(|carried| {
                carried
                    .active_equipment()
                    .filter(|(_, e)| !e.curse_released && items[e.stack.item].cursed)
                    .map(|(_, e)| e.stack.id)
                    .collect()
            })
            .unwrap_or_default();
        let body = bodies.and_then(|bodies| bodies.first().copied());
        let stands = match (body, character, vitals) {
            (Some(body), Some(character), Some(vitals)) if !vitals.is_dead() => Some((
                body,
                vitals.conditions.clone(),
                vitals.stamina.min(character.base_stats().max_stamina()),
            )),
            _ => None,
        };
        self.commands.entity(actor).remove::<PartyMember>();
        self.party.3.unseat(actor);
        if let Some((body, conditions, stamina)) = stands {
            self.stand_beside_party(body, conditions, stamina);
        }
        self.hand_over(actor, |stack| wanted.contains(&stack.item) || held.contains(&stack.id));
        true
    }

    /// The NPC's dormant body stands again beside the party, on its level;
    /// one out cold stays as spent as it was.
    fn stand_beside_party(
        &mut self,
        body: Entity,
        conditions: crate::game::characters::Conditions,
        stamina: u32,
    ) {
        let (at, yaw) = (*self.party.4, self.party.5.yaw);
        let (root, level, navigation) = self.levels.single().unwrap();
        // Retail asks the navigator for a place out from the party along its
        // heading turned by an eighth of a turn; this stands it two metres out
        // that way, without the navigator's search.
        let feet = PartyBody::feet(&at)
            + Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_4) * Vec3::NEG_Z * 2.0;
        let (mut monster, mut role, ..) = self.bodies.get_mut(body).unwrap();
        let kind = &self.content.species[monster.species];
        let (max_hp, max_stamina) =
            (self.rng.roll(kind.hit_points).max(1), self.rng.roll(kind.stamina));
        monster.hp = max_hp;
        monster.max_hp = max_hp;
        monster.max_stamina = max_stamina;
        monster.stamina = if conditions.exhausted() { stamina } else { max_stamina };
        monster.conditions = conditions;
        role.map = level.path.clone();
        role.group = None;
        let (speed, radius) = (monster.body.speed, monster.body.radius);
        let mut entity = self.commands.entity(body);
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
                &mut self.configs,
            );
        }
    }

    /// Give a departing member's items to the party, those the pool has no
    /// room for dropped at its feet. `keep` says which the member keeps.
    fn hand_over(&mut self, actor: Entity, keep: impl Fn(&ItemStack) -> bool) {
        let Some(mut carried) = self.actors.get_mut(actor).unwrap().6 else { return };
        let carried = &mut *carried;
        let worn = carried
            .equipment
            .values_mut()
            .filter_map(|worn| worn.take_if(|e| !keep(&e.stack)).map(|e| e.stack));
        let mut given: Vec<_> = worn.collect();
        given.extend(carried.backpack.iter_mut().filter_map(|place| place.take_if(|s| !keep(s))));
        let items = &self.content.items;
        let pool = &mut self.party.1.pool;
        let left: Vec<_> =
            given.into_iter().filter_map(|stack| pool.receive(stack, items)).collect();
        let (level, ..) = self.levels.single().unwrap();
        let feet = PartyBody::feet(self.party.4);
        crate::game::world::items::drop_loot(&mut self.commands, level, feet, left, items);
    }
}

/// An NPC that accepted the party's offer joins it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinParty {
    pub actor: Entity,
}

/// NPCs that accepted join, if they still would. The dialogue that asks
/// them also says they join, so the request is read inside the set.
pub fn join_party(
    mut recruiting: ParamSet<(MessageReader<JoinParty>, super::conversation::Dialogue, Recruits)>,
) {
    let joins: Vec<_> = recruiting.p0().read().map(|join| join.actor).collect();
    for actor in joins {
        match recruiting.p1().joinable(actor) {
            Ok(()) => recruiting.p2().join(actor),
            Err(refusal) => warn!("NPC did not join after all: {refusal:?}"),
        }
    }
}

/// [`Recruits::join`] once the NPC would join, for operations that hold the
/// whole world.
pub fn join(world: &mut World, actor: Entity) -> Result<(), JoinRefusal> {
    joinable(world, actor)?;
    world
        .run_system_cached_with(|In(actor): In<Entity>, mut r: Recruits| r.join(actor), actor)
        .unwrap();
    Ok(())
}

/// [`Recruits::dismiss`], for operations that hold the whole world.
pub fn dismiss(world: &mut World, actor: Entity) -> bool {
    world
        .run_system_cached_with(|In(actor): In<Entity>, mut r: Recruits| r.dismiss(actor), actor)
        .unwrap()
}

/// A member NPC that dies leaves the party at once (retail
/// `UpdateNpcPartyMember`): its items go to the party, it is dead for good
/// (retail `spawned`), and it never stands in the world again. Before a
/// campaign begins there is no party to settle.
pub fn settle_npc_members(mut recruits: Recruits) {
    let fallen: Vec<Entity> = recruits
        .party
        .2
        .iter()
        .filter(|&member| {
            recruits
                .actors
                .get(member)
                .is_ok_and(|(npc, .., vitals, _)| !npc.dead && vitals.is_some_and(|v| v.is_dead()))
        })
        .collect();
    for member in fallen {
        recruits.commands.entity(member).remove::<PartyMember>();
        recruits.party.3.unseat(member);
        recruits.actors.get_mut(member).unwrap().0.dead = true;
        recruits.hand_over(member, |_| false);
    }
}
