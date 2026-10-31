//! Point effects commit source payment and their actual scene actors together.
use super::{
    Outcome,
    casting::{Aim, MagicChoice, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{characters::Skill, monster::MonsterBody, species::MonsterKind, spells::SpellId},
    game::{
        characters::Condition,
        combat::{Battle, Combatant},
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
use bevy::{ecs::relationship::RelationshipTarget, prelude::*};
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

/// What a point cast reads of the level it lands in.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Ground<'w, 's> {
    level: Single<
        'w,
        's,
        (
            Entity,
            &'static Level,
            &'static crate::game::world::triggers::LevelState,
            &'static Unborn,
            &'static MonsterBodies,
            &'static LevelNavigation,
        ),
    >,
    definitions: Res<'w, Assets<crate::content::level::LevelDefinition>>,
    bodies: Res<'w, Assets<MonsterBody>>,
    configs: ResMut<'w, Assets<WalkerMotionConfig>>,
    sight: WorldObstacles<'w, 's>,
    party: Single<
        'w,
        's,
        &'static avian3d::prelude::Position,
        With<crate::game::party::body::PartyBody>,
    >,
}

/// The monsters a point cast may move or join.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Crowd<'w, 's> {
    monsters: Query<
        'w,
        's,
        (
            Entity,
            &'static Monster,
            &'static LevelOwned,
            Option<&'static LureInfluence>,
            Option<&'static GroupMember>,
            Option<&'static avian3d::prelude::Position>,
            Option<&'static Transform>,
        ),
    >,
    members: Query<'w, 's, &'static GroupMembers>,
    groups: Query<'w, 's, (&'static MonsterGroup, &'static LevelOwned)>,
    summons: Query<'w, 's, &'static Elementals>,
    lures: Query<'w, 's, Entity, With<Lure>>,
    stances: hostility::Stances<'w, 's>,
}

/// Hypnotic Lure and Summon Elemental land where the caster points, in
/// sight and reach. A summoning refused in a fight leaves the caster
/// defending (retail `ExecuteCharacterAction`), the turn spent.
pub(crate) fn cast_at_points(
    mut casts: MessageReader<super::utilities::UtilityCast>,
    freedom: crate::game::freedom::Freedom,
    mut battle: Option<Single<(Entity, &mut Battle)>>,
    mut ground: Ground,
    crowd: Crowd,
    mut payment: super::casting::Payment,
    mut news: MessageWriter<MagicNews>,
    clock: Res<GameClock>,
    mut commands: Commands,
) {
    for &super::utilities::UtilityCast { caster, choice, spell: id } in casts.read() {
        if !spell(id) {
            continue;
        }
        let fight = battle.as_ref().map(|battle| battle.0);
        let result = point_cast(
            (caster, choice, id, fight),
            &freedom,
            &mut ground,
            &crowd,
            &mut payment,
            (clock.ms, &mut commands),
        );
        match result {
            Ok(report) => {
                news.write(MagicNews::Done(report));
            }
            Err(refusal) => {
                if let Some(battle) = battle.as_mut() {
                    battle.1.falls_back_to_defending(caster);
                }
                news.write(MagicNews::Refused { caster, refusal });
            }
        }
    }
}

fn point_cast(
    (caster, choice, id, battle): (Entity, MagicChoice, SpellId, Option<Entity>),
    freedom: &crate::game::freedom::Freedom,
    ground: &mut Ground,
    crowd: &Crowd,
    payment: &mut super::casting::Payment,
    (now, commands): (u64, &mut Commands),
) -> Result<super::casting::Report, Refusal> {
    if !freedom.free() {
        return Err(Refusal::WrongTime);
    }
    let Aim::Point(at) = choice.aim else { return Err(Refusal::NoTarget) };
    let (root, level, state, ..) = *ground.level;
    let spell = payment.content.spells[id].clone();
    let far = ground.definitions.get(&level.definition).unwrap().view_distance();
    let eye = crate::game::party::body::PartyBody::eye(&ground.party);
    if eye.distance(at) > spell.reach.metres() + crate::content::EYE_HEIGHT
        || !ground.sight.clear(eye, at)
    {
        return Err(Refusal::NoTarget);
    }
    if id == SpellId::SUMMON_ELEMENTAL {
        if battle.is_none() {
            return Err(Refusal::WrongTime);
        }
        if crowd.summons.get(caster).is_ok_and(|summons| {
            summons.iter().any(|entity| !crowd.monsters.get(entity).unwrap().1.is_dead())
        }) {
            return Err(Refusal::NoTarget);
        }
    }
    let underwater = state.camera_sway;
    let weights = if id == SpellId::SUMMON_ELEMENTAL {
        let profile = payment.members.get(caster).unwrap().0;
        [Skill::FireMagic, Skill::WaterMagic, Skill::AirMagic, Skill::EarthMagic]
            .map(|skill| profile.skill(skill) + 20)
    } else {
        [0; 4]
    };
    let (report, strength) = payment.pay(
        caster,
        choice.source,
        if battle.is_some() { UseContext::Combat } else { UseContext::World },
    )?;
    if report.outcome != Outcome::Fizzled {
        match battle {
            Some(battle) if id == SpellId::SUMMON_ELEMENTAL => summon_elemental(
                (caster, battle, root),
                at,
                (weights, underwater),
                (strength, report.outcome == Outcome::Backfired, now),
                (ground, crowd, payment, commands),
            ),
            _ => lure(
                root,
                (at, far),
                (&spell, strength, now),
                battle.is_some(),
                (ground, crowd, payment, commands),
            ),
        }
    }
    Ok(report)
}

/// A Hypnotic Lure stands at `at` in place of any before it. Each hostile
/// monster able to see it, within the far clip and not undead, may resist;
/// outside a fight its whole group follows its choice.
fn lure(
    root: Entity,
    (at, far): (Vec3, f32),
    (spell, strength, now): (&crate::content::spells::SpellDefinition, super::Strength, u64),
    fighting: bool,
    (ground, crowd, payment, commands): (
        &Ground,
        &Crowd,
        &mut super::casting::Payment,
        &mut Commands,
    ),
) {
    for old in &crowd.lures {
        commands.entity(old).despawn();
    }
    let turns = super::effects::duration(&mut payment.rng, spell, strength);
    let lure = commands
        .spawn((
            Lure { expires: now + u64::from(turns) * AGING_TICK_MS },
            Transform::from_translation(at),
            LevelOwned(root),
        ))
        .id();
    let mut order: Vec<_> = crowd
        .monsters
        .iter()
        .filter(|(_, monster, owner, ..)| owner.0 == root && !monster.is_dead())
        .collect();
    order.sort_unstable_by_key(|(_, body, ..)| body.location);
    let species = &payment.content.species;
    for (entity, body, _, influence, group, position, transform) in order {
        if crowd.stances.own(entity, species, now) != Some(Stance::Hostile)
            || influence.is_some_and(|influence| influence.response == LureResponse::Resisted)
            || !body.can_fight()
            || body.conditions.has(Condition::Blind)
        {
            continue;
        }
        let here = position.map_or_else(|| transform.unwrap().translation, |position| position.0);
        if (here - at).abs().max_element() > far {
            continue;
        }
        let kind = &species[body.species];
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
        if !ground.sight.clear(body.eye(here), at) {
            continue;
        }
        let response = if super::effects::save_roll(&mut payment.rng, chance) {
            LureResponse::Resisted
        } else {
            LureResponse::Controlled
        };
        let group = group.and_then(|group| crowd.members.get(group.0).ok());
        let affected = match group {
            Some(members) if !fighting => members.iter().collect(),
            _ => vec![entity],
        };
        for entity in affected {
            commands.entity(entity).insert(LureInfluence { lure, response });
        }
    }
}

/// An elemental answers the summoner: its element weighted by the
/// summoner's realm skills (none of fire under water), its tier by the
/// power, friendly unless the summoning backfired. It joins the fight and
/// lasts no longer; Power Cast may add its element's buff.
fn summon_elemental(
    (caster, battle, root): (Entity, Entity, Entity),
    at: Vec3,
    (mut weights, underwater): ([u32; 4], bool),
    (strength, backfired, now): (super::Strength, bool, u64),
    (ground, crowd, payment, commands): (
        &mut Ground,
        &Crowd,
        &mut super::casting::Payment,
        &mut Commands,
    ),
) {
    if underwater {
        weights[0] = 0;
    }
    let rng = &mut payment.rng;
    let roll = rng.below(weights.iter().sum());
    let realm = crate::game::rng::weighted(roll, weights).unwrap_or(3);
    let species_id = tier(strength.power, realm);
    let (_, _, _, unborn, bodies, navigation) = *ground.level;
    let locations = crowd
        .monsters
        .iter()
        .filter(|(_, _, owner, ..)| owner.0 == root)
        .map(|(_, body, ..)| body.location);
    let groups =
        crowd.groups.iter().filter(|(_, owner)| owner.0 == root).map(|(group, _)| group.authored);
    let ids = population::next_ids(locations, groups, unborn);
    let catalogs = &payment.content;
    let body = bodies.get(species_id, &ground.bodies).unwrap();
    let mut members =
        population::roll_group(ids, (species_id, 1), (&catalogs.species[species_id], body), rng);
    let elemental = &mut members[0];
    elemental.feet = at;
    // Power Cast may add the element's associated buff; use the same slot and duration rules.
    if strength.boost != 0 && rng.below(100) < strength.boost {
        let buff =
            [SpellId::HASTE, SpellId::BODY_OF_STONE, SpellId::MISSILE_SHIELD, SpellId::ARMORPLATE]
                [realm];
        let turns = super::effects::duration(rng, &catalogs.spells[buff], strength);
        elemental.monster.enchantments.apply_to_monster(super::enchantments::Enchantment {
            spell: buff,
            power: strength.power,
            boost: 0,
            turns,
            absorption: 0,
        });
    }
    let stance = if backfired { Stance::Hostile } else { Stance::Friendly };
    let (group, spawned) = population::spawn_group(
        commands,
        root,
        members,
        (ids.1, Some(SetStance { stance, at: now })),
        &catalogs.species,
        (navigation, &mut ground.configs),
    );
    // The summoning lasts no longer than the fight.
    commands.entity(group).insert(ChildOf(battle));
    commands.entity(spawned[0]).insert((Combatant(battle), SummonedBy(caster), ChildOf(battle)));
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
