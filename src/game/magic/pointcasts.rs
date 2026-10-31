//! Point effects commit source payment and their actual scene actors together.
use super::{
    Outcome,
    casting::{Aim, MagicChoice, MagicNews, Refusal, UseContext},
};
use crate::game::data::Catalogs;
use crate::{
    content::{characters::Skill, monster::MonsterBody, species::MonsterKind, spells::SpellId},
    game::{
        characters::{Character, Condition},
        combat::{Battle, Combatant},
        rng::GameRng,
        time::{AGING_TICK_MS, GameClock},
        world::{
            hostility::{self, SetStance, Stance},
            identify::{GroupMember, GroupMembers, MonsterGroup},
            level::{Level, LevelOwned},
            navigation::{LevelNavigation, WalkerMotionConfig},
            obstacles::WorldObstacles,
            population::{self, Monster, MonsterBodies, Unborn},
        },
    },
};
use bevy::{
    ecs::{relationship::RelationshipTarget, system::SystemState},
    prelude::*,
};
use bevy_landmass::AgentTarget3d;

#[derive(Component)]
#[relationship(relationship_target = Elementals)]
pub struct SummonedBy(pub Entity);
#[derive(Component)]
#[relationship_target(relationship = SummonedBy)]
pub struct Elementals(Vec<Entity>);
#[derive(Component)]
pub struct Lure {
    expires: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LureResponse {
    Controlled,
    #[default]
    Resisted,
}
/// A lured monster's hold. Losing it, however the lure goes, frees a
/// monster outside the fight to walk where it would.
#[derive(Component, Clone, Copy)]
#[relationship(relationship_target = LureTargets)]
#[component(on_remove = release_lured)]
pub struct LureInfluence {
    #[relationship]
    pub lure: Entity,
    pub response: LureResponse,
}
#[derive(Component)]
#[relationship_target(relationship = LureInfluence)]
pub struct LureTargets(Vec<Entity>);

fn release_lured(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    if world.get::<Combatant>(context.entity).is_none()
        && let Some(mut target) = world.get_mut::<AgentTarget3d>(context.entity)
    {
        *target = AgentTarget3d::None;
    }
}

pub fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::HYPNOTIC_LURE | SpellId::SUMMON_ELEMENTAL)
}
fn tier(power: u8, realm: usize) -> u32 {
    // Retail g_elemental_summon_group_ids: air, earth, water, fire per tier.
    21 + u32::from(power.saturating_sub(1).min(6)) * 4 + [3, 2, 0, 1][realm]
}
/// The elementals Summon Elemental may call, four elements to a tier
/// (retail `g_elemental_summon_group_ids`). Their bodies load with every
/// level ([`MonsterBodies::for_level`]).
pub const ELEMENTALS: std::ops::RangeInclusive<u32> = 21..=48;

pub(crate) fn cast(
    world: &mut World,
    caster: Entity,
    choice: MagicChoice,
    id: SpellId,
) -> Result<(), Refusal> {
    if !crate::game::freedom::free_in(world) {
        return Err(Refusal::WrongTime);
    }
    let Aim::Point(at) = choice.aim else { return Err(Refusal::NoTarget) };
    let root = world.query_filtered::<Entity, With<Level>>().single(world).unwrap();
    let battle = world.query_filtered::<Entity, With<Battle>>().single(world).ok();
    let (spell, far) = {
        let spell = world.resource::<Catalogs>().spells[id].clone();
        let level = world.get::<Level>(root).unwrap();
        let far = world
            .resource::<Assets<crate::content::level::LevelDefinition>>()
            .get(&level.definition)
            .unwrap()
            .view_distance();
        (spell, far)
    };
    let eye = crate::game::party::body::PartyBody::eye_in(world);
    if eye.distance(at) > spell.reach.metres() + crate::content::EYE_HEIGHT {
        return Err(Refusal::NoTarget);
    }
    let mut obstacles = SystemState::<WorldObstacles>::new(world);
    let view = obstacles.get(world).unwrap();
    if !view.clear(eye, at) {
        return Err(Refusal::NoTarget);
    }
    if id == SpellId::SUMMON_ELEMENTAL {
        if battle.is_none() {
            return Err(Refusal::WrongTime);
        }
        if world.get::<Elementals>(caster).is_some_and(|summons| {
            summons.iter().any(|entity| !world.get::<Monster>(entity).unwrap().is_dead())
        }) {
            return Err(Refusal::NoTarget);
        }
    }
    let underwater = world
        .get::<crate::game::world::triggers::LevelState>(root)
        .is_some_and(|state| state.camera_sway);
    let weights = if id == SpellId::SUMMON_ELEMENTAL {
        let profile = world.get::<Character>(caster).unwrap();
        [Skill::FireMagic, Skill::WaterMagic, Skill::AirMagic, Skill::EarthMagic]
            .map(|skill| profile.skill(skill) + 20)
    } else {
        [0; 4]
    };
    let (report, strength) = super::casting::cast_source(
        world,
        caster,
        choice.source,
        if battle.is_some() { UseContext::Combat } else { UseContext::World },
    )?;
    if report.outcome != Outcome::Fizzled {
        let now = world.resource::<GameClock>().ms;
        match battle {
            Some(battle) if id == SpellId::SUMMON_ELEMENTAL => summon_elemental(
                world,
                (caster, battle, root),
                at,
                (weights, underwater),
                (strength, report.outcome == Outcome::Backfired, now),
            ),
            _ => lure(world, root, (at, far), (&spell, strength, now), battle.is_some()),
        }
    }
    world.write_message(MagicNews::Done(report));
    Ok(())
}

/// A Hypnotic Lure stands at `at` in place of any before it. Each hostile
/// monster able to see it, within the far clip and not undead, may resist;
/// outside a fight its whole group follows its choice.
fn lure(
    world: &mut World,
    root: Entity,
    (at, far): (Vec3, f32),
    (spell, strength, now): (&crate::content::spells::SpellDefinition, super::Strength, u64),
    fighting: bool,
) {
    let old: Vec<_> = world.query_filtered::<Entity, With<Lure>>().iter(world).collect();
    for old in old {
        world.despawn(old);
    }
    let turns = super::effects::duration(&mut world.resource_mut::<GameRng>(), spell, strength);
    let lure = world
        .spawn((
            Lure { expires: now + u64::from(turns) * AGING_TICK_MS },
            Transform::from_translation(at),
            LevelOwned(root),
        ))
        .id();
    let mut order: Vec<_> = world
        .query::<(Entity, &Monster, &LevelOwned)>()
        .iter(world)
        .filter(|(_, monster, owner)| owner.0 == root && !monster.is_dead())
        .map(|(entity, body, _)| (body.location, entity))
        .collect();
    order.sort_unstable();
    let mut sight = SystemState::<WorldObstacles>::new(world);
    for (_, entity) in order {
        if hostility::monster_stance(world, entity) != Stance::Hostile {
            continue;
        }
        if world
            .get::<LureInfluence>(entity)
            .is_some_and(|influence| influence.response == LureResponse::Resisted)
        {
            continue;
        }
        let body = world.get::<Monster>(entity).unwrap();
        if !body.can_fight() || body.conditions.has(Condition::Blind) {
            continue;
        }
        let here = crate::game::world::actor_position(world, entity);
        if (here - at).abs().max_element() > far {
            continue;
        }
        let kind = &world.resource::<Catalogs>().species[body.species];
        if kind.kind == MonsterKind::Undead {
            continue;
        }
        let resistance = body.resistance(kind, crate::content::characters::Realm::Mental);
        let chance = super::effects::condition_chance(
            u32::from(kind.level),
            resistance,
            strength.against,
            0,
        );
        let eye = body.eye(here);
        let clear = sight.get(world).unwrap().clear(eye, at);
        if !clear {
            continue;
        }
        let response = if super::effects::save_roll(&mut world.resource_mut::<GameRng>(), chance) {
            LureResponse::Resisted
        } else {
            LureResponse::Controlled
        };
        let group =
            world.get::<GroupMember>(entity).and_then(|group| world.get::<GroupMembers>(group.0));
        let affected = if !fighting {
            group.map_or_else(|| vec![entity], |members| members.iter().collect())
        } else {
            vec![entity]
        };
        for entity in affected {
            world.entity_mut(entity).insert(LureInfluence { lure, response });
        }
    }
}

/// An elemental answers the summoner: its element weighted by the
/// summoner's realm skills (none of fire under water), its tier by the
/// power, friendly unless the summoning backfired. It joins the fight and
/// lasts no longer; Power Cast may add its element's buff.
fn summon_elemental(
    world: &mut World,
    (caster, battle, root): (Entity, Entity, Entity),
    at: Vec3,
    (mut weights, underwater): ([u32; 4], bool),
    (strength, backfired, now): (super::Strength, bool, u64),
) {
    if underwater {
        weights[0] = 0;
    }
    let roll = world.resource_mut::<GameRng>().below(weights.iter().sum());
    let realm = crate::game::rng::weighted(roll, weights).unwrap_or(3);
    let species_id = tier(strength.power, realm);
    let locations: Vec<i32> = world
        .query::<(&Monster, &LevelOwned)>()
        .iter(world)
        .filter(|(_, owner)| owner.0 == root)
        .map(|(body, _)| body.location)
        .collect();
    let groups: Vec<i32> = world
        .query::<(&MonsterGroup, &LevelOwned)>()
        .iter(world)
        .filter(|(_, owner)| owner.0 == root)
        .map(|(group, _)| group.authored)
        .collect();
    let ids = population::next_ids(locations, groups, world.get::<Unborn>(root).unwrap());
    let mut members = world.resource_scope(|world, mut rng: Mut<GameRng>| {
        let kind = &world.resource::<Catalogs>().species[species_id];
        let body = world
            .get::<MonsterBodies>(root)
            .unwrap()
            .get(species_id, world.resource::<Assets<MonsterBody>>())
            .unwrap();
        population::roll_group(ids, (species_id, 1), (kind, body), &mut rng)
    });
    members[0].feet = at;
    let stance = if backfired { Stance::Hostile } else { Stance::Friendly };
    let navigation = LevelNavigation {
        archipelagos: world.get::<LevelNavigation>(root).unwrap().archipelagos.clone(),
    };
    let (group, spawned) = Catalogs::scope(world, |world, catalogs| {
        world.resource_scope(|world, mut configs: Mut<Assets<WalkerMotionConfig>>| {
            population::spawn_group(
                &mut world.commands(),
                root,
                members,
                (ids.1, Some(SetStance { stance, at: now })),
                &catalogs.species,
                (&navigation, &mut configs),
            )
        })
    });
    world.flush();
    // The summoning lasts no longer than the fight.
    world.entity_mut(group).insert(ChildOf(battle));
    let elemental = spawned[0];
    world.entity_mut(elemental).insert((Combatant(battle), SummonedBy(caster), ChildOf(battle)));
    // Power Cast may add the element's associated buff; use the same slot and duration rules.
    if strength.boost != 0 && world.resource_mut::<GameRng>().below(100) < strength.boost {
        let buff =
            [SpellId::HASTE, SpellId::BODY_OF_STONE, SpellId::MISSILE_SHIELD, SpellId::ARMORPLATE]
                [realm];
        let turns = world.resource_scope(|world, mut rng: Mut<GameRng>| {
            let definition = &world.resource::<Catalogs>().spells[buff];
            super::effects::duration(&mut rng, definition, strength)
        });
        world.get_mut::<Monster>(elemental).unwrap().enchantments.apply_to_monster(
            super::enchantments::Enchantment {
                spell: buff,
                power: strength.power,
                boost: 0,
                turns,
                absorption: 0,
            },
        );
    }
}

/// Lures last their turns, then go, and with them their hold.
pub(crate) fn expire_lures(
    mut commands: Commands,
    lures: Query<(Entity, &Lure)>,
    clock: Res<GameClock>,
) {
    for (entity, lure) in &lures {
        if lure.expires <= clock.ms {
            commands.entity(entity).despawn();
        }
    }
}

/// A controlled monster outside the fight that can see walks to its lure
/// while it is more than five metres off, and faces it.
pub(crate) fn follow_lures(
    lures: Query<&Transform, With<Lure>>,
    mut followers: Query<
        (
            &LureInfluence,
            &Monster,
            Option<&mut AgentTarget3d>,
            Option<&avian3d::prelude::Position>,
            &mut Transform,
            Option<&mut avian3d::prelude::Rotation>,
        ),
        (Without<Combatant>, Without<Lure>),
    >,
) {
    for (influence, body, walking, position, mut transform, rotation) in &mut followers {
        let target = lures.get(influence.lure).unwrap().translation;
        let here = position.map_or(transform.translation, |p| p.0);
        let follows = influence.response == LureResponse::Controlled
            && body.can_fight()
            && !body.conditions.has(Condition::Blind);
        if let Some(mut walking) = walking {
            *walking = if follows && here.distance(target) > 5.0 {
                AgentTarget3d::Entity(influence.lure)
            } else {
                AgentTarget3d::None
            };
        }
        if follows && let Some(facing) = crate::game::world::facing(here, target) {
            if let Some(mut rotation) = rotation {
                rotation.0 = facing;
            }
            transform.rotation = facing;
        }
    }
}
