//! Hypnotic Lure and Summon Elemental (`docs/rules/casting.md` section 8): once
//! the cast comes off, a lure stands at the spot and holds the monsters that
//! fail to resist it, or an elemental answers the summoner. The cast is paid
//! for and rolled as any other ([`super::casting`]); the arithmetic of what it
//! then does is [`super::workings`].
use super::Outcome;
use super::casting::{Allowed, CastBy, CastSpell, Chosen, MagicNews, Worked, marked};
use super::may::Refusal;
use super::reach::Aim;
use super::reach::Marked;
use super::workings::{self, Response};
use crate::content::characters::{Realm, Skill};
use crate::content::monster::MonsterBody;
use crate::content::species::MonsterKind;
use crate::game::characters::Vitals;
use crate::game::data::Catalogs;
use crate::game::world::level::InPlay;
use crate::game::{
    actors::{CanFight, Resistances, Skills},
    characters::{Calling, Condition, Conditions},
    combat::{Battle, Combatant},
    rng::GameRng,
    time::{AGING_TICK_MS, GameClock},
    world::{
        hostility::{SetStance, Stance, Standing},
        identify::{GroupMember, GroupMembers, MonsterGroup},
        level::{Level, LevelOwned},
        navigation::{LevelNavigation, WalkerMotionConfig},
        obstacles::WorldObstacles,
        population::{self, Monster, MonsterBodies, Unborn},
        triggers::LevelState,
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
/// A Hypnotic Lure standing at its spot until it expires, calling the
/// monsters of its level as it is set ([`calls`]) at the strength of its
/// cast.
#[derive(Component)]
pub struct Lure {
    expires: u64,
    against: u32,
}
/// A lured monster's hold. Losing it, however the lure goes, frees a
/// monster outside the fight to walk where it would.
#[derive(Component, Clone, Copy)]
#[relationship(relationship_target = LureTargets)]
#[component(on_remove = release_lured)]
pub struct LureInfluence {
    #[relationship]
    pub lure: Entity,
    pub response: Response,
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

/// Where a point spell may be cast: a spot within the spell's reach and an
/// eye's height of the caster's eye that the caster sees.
fn spot(aim: Aim, eye: Vec3, reach: f32, obstacles: &WorldObstacles) -> Result<Marked, Refusal> {
    match aim {
        Aim::Point(at)
            if eye.distance(at) <= reach + crate::content::EYE_HEIGHT
                && obstacles.clear(eye, at) =>
        {
            Ok(Marked::Spot(at))
        }
        _ => Err(Refusal::NoTarget),
    }
}

/// Hypnotic Lure is cast at a spot the caster sees.
pub fn mark_lure(
    allowed: On<Allowed>,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
    places: crate::game::actors::Places,
    obstacles: WorldObstacles,
    mut commands: Commands,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Chosen(choice)) = casts.get(allowed.cast)?;
    let at = spot(choice.aim, places.of(caster).eye, spell.reach.metres(), &obstacles);
    marked(&mut commands, (allowed.cast, caster), at);
    Ok(())
}

/// Summon Elemental is cast at a spot the caster sees, while no elemental the
/// caster summoned still stands.
pub fn mark_summon(
    allowed: On<Allowed>,
    casts: Query<(&CastBy, &CastSpell, &Chosen)>,
    elementals: Query<&Elementals>,
    alive: Query<&Vitals>,
    places: crate::game::actors::Places,
    obstacles: WorldObstacles,
    mut commands: Commands,
) -> Result {
    let (&CastBy(caster), CastSpell(spell), &Chosen(choice)) = casts.get(allowed.cast)?;
    let standing = elementals.get(caster).is_ok_and(|summoned| {
        summoned.iter().any(|elemental| alive.get(elemental).is_ok_and(|v| !v.is_dead()))
    });
    let at = match standing {
        true => Err(Refusal::NoTarget),
        false => spot(choice.aim, places.of(caster).eye, spell.reach.metres(), &obstacles),
    };
    marked(&mut commands, (allowed.cast, caster), at);
    Ok(())
}

/// A Hypnotic Lure, once paid for and rolled, short of a fizzle, stands at
/// its spot for the turns the spell lasts.
pub fn lure(
    worked: On<Worked>,
    mut commands: Commands,
    level: Single<Entity, With<InPlay>>,
    spells: Res<Catalogs>,
    clock: Res<GameClock>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) {
    let Worked { report, strength, marked, .. } = worked.event().clone();
    let Marked::Spot(at) = marked else { return };
    let (outcome, spell) = (report.outcome, &spells.spells[report.spell]);
    news.write(MagicNews::Done(report));
    if outcome == Outcome::Fizzled {
        return;
    }
    let turns = crate::game::hits::definitions::duration(
        &mut rng,
        spell.duration_base,
        spell.duration_per_power,
        strength,
    );
    let expires = workings::lure_ends(clock.ms, turns, AGING_TICK_MS);
    commands.spawn((
        Lure { expires, against: strength.against },
        Transform::from_translation(at),
        LevelOwned(*level),
    ));
}

/// A monster is tempted by a lure, at the strength of its cast.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Tempted {
    #[event_target]
    pub monster: Entity,
    pub lure: Entity,
    pub against: u32,
}

/// A lure set takes the place of any before it, and puts to the test, in the
/// order of their places in the level, each hostile monster not resisting a
/// lure already, able to fight and not blind, within the far clip of its
/// spot on every axis, with a clear line to it.
#[allow(clippy::type_complexity)]
pub fn calls(
    set: On<Add<Lure>>,
    lures: Query<(&Lure, &Transform, &LevelOwned)>,
    others: Query<Entity, With<Lure>>,
    minds: Query<(&Standing, Has<CanFight>, &Conditions)>,
    level: Single<&Level, With<InPlay>>,
    monsters: Query<(
        Entity,
        &Monster,
        &LevelOwned,
        crate::game::world::ActorPose,
        Option<&LureInfluence>,
    )>,
    obstacles: WorldObstacles,
    level_assets: Res<Assets<crate::content::level::LevelDefinition>>,
    mut commands: Commands,
) -> Result {
    let lure = set.entity;
    let (&Lure { against, .. }, spot, &LevelOwned(root)) = lures.get(lure)?;
    let at = spot.translation;
    let far =
        level_assets.get(&level.definition).ok_or("the level played is loaded")?.view_distance();
    let mut order: Vec<_> = monsters
        .iter()
        .filter(|(_, _, owner, ..)| owner.0 == root)
        .map(|(entity, monster, ..)| (monster.location, entity))
        .collect();
    order.sort_unstable();
    // Who resists a lure already is judged before those before this one go.
    let mut tested = Vec::new();
    for (_, monster) in order {
        let (_, kind, _, pose, influence) = monsters.get(monster)?;
        let (standing, can_fight, conditions) = minds.get(monster)?;
        let here = pose.translation();
        let put_to_test = standing.stance == Stance::Hostile
            && !influence.is_some_and(|held| held.response == Response::Resisted)
            && can_fight
            && !conditions.has(Condition::Blind)
            && (here - at).abs().max_element() <= far
            && obstacles.clear(kind.eye(here), at);
        if put_to_test {
            tested.push(monster);
        }
    }
    for old in others.iter().filter(|&old| old != lure) {
        commands.entity(old).despawn();
    }
    for monster in tested {
        commands.trigger(Tempted { monster, lure, against });
    }
    Ok(())
}

/// A monster tempted resists or is held (an undead one always resists);
/// outside a fight its whole group follows its choice (retail
/// `ApplyMonsterControlToNearbyMonsters`).
pub fn tempted(
    tempted: On<Tempted>,
    minds: Query<(&crate::game::actors::Level, &Resistances, Option<&crate::game::actors::Kind>)>,
    members: Query<Option<&GroupMember>>,
    groups: Query<&GroupMembers>,
    battle: Option<Res<Battle>>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) -> Result {
    let Tempted { monster, lure, against } = *tempted;
    let (level, resistances, kind) = minds.get(monster)?;
    let response = workings::lure_response(
        &mut rng,
        (level.0, resistances.realms[Realm::Mental]),
        kind.is_some_and(|kind| kind.0 == MonsterKind::Undead),
        against,
    );
    let group = members.get(monster)?.and_then(|group| groups.get(group.0).ok());
    let held = match group {
        Some(group) if battle.is_none() => group.iter().collect(),
        _ => vec![monster],
    };
    for monster in held {
        commands.entity(monster).insert(LureInfluence { lure, response });
    }
    Ok(())
}

/// Summon Elemental, once paid for and rolled, short of a fizzle: an
/// elemental answers the summoner at the spot ([`Answers`]), its element
/// weighted by the summoner's realm skills (none of fire under water), its
/// tier by the power, friendly unless the summoning backfired.
pub fn summon(
    worked: On<Worked>,
    mut commands: Commands,
    realm_skills: Query<&Skills, With<Calling>>,
    level: Single<&LevelState, With<InPlay>>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) {
    let Worked { caster, report, strength, marked, .. } = worked.event().clone();
    let Marked::Spot(at) = marked else { return };
    let outcome = report.outcome;
    news.write(MagicNews::Done(report));
    if outcome == Outcome::Fizzled {
        return;
    }
    let skills = realm_skills.get(caster).ok().map(|skills| {
        [Skill::FireMagic, Skill::WaterMagic, Skill::AirMagic, Skill::EarthMagic]
            .map(|skill| skills[skill])
    });
    let summoned = workings::summon(&mut rng, skills, level.camera_sway, outcome, strength);
    commands.trigger(Answers { caster, at, summoned, strength });
}

/// An elemental answers a summoner, at a spot, by the strength of the cast.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Answers {
    #[event_target]
    pub caster: Entity,
    pub at: Vec3,
    pub summoned: workings::Summoned,
    pub strength: super::Strength,
}

/// The elemental that answers is made at the spot as a group of one of the
/// level's, joins the fight and lasts no longer than it; Power Cast may lay
/// its element's buff on it, by the same rules of slot and duration as the
/// spell itself.
#[allow(clippy::type_complexity)]
pub fn answer(
    answers: On<Answers>,
    mut commands: Commands,
    level: Single<(Entity, &Unborn, &MonsterBodies, &LevelNavigation), With<InPlay>>,
    battle: Single<Entity, With<Battle>>,
    monsters: Query<(&Monster, &LevelOwned)>,
    groups: Query<(&MonsterGroup, &LevelOwned)>,
    mut laying: crate::game::actors::Laying,
    (spells, body_assets, mut configs): (
        Res<Catalogs>,
        Res<Assets<MonsterBody>>,
        ResMut<Assets<WalkerMotionConfig>>,
    ),
    clock: Res<GameClock>,
    mut rng: ResMut<GameRng>,
) {
    let Answers { caster, at, summoned, strength } = *answers;
    let (root, unborn, level_bodies, navigation) = *level;
    let battle = *battle;
    let locations =
        monsters.iter().filter(|(_, owner)| owner.0 == root).map(|(monster, _)| monster.location);
    let authored =
        groups.iter().filter(|(_, owner)| owner.0 == root).map(|(group, _)| group.authored);
    let ids = population::next_ids(locations, authored, unborn);
    let kind = &spells.species[summoned.species];
    let body = level_bodies.get(summoned.species, &body_assets).unwrap();
    let mut members = population::roll_group(ids, (summoned.species, 1), (kind, body), &mut rng);
    members[0].feet = at;
    let stance = if summoned.backfired { Stance::Hostile } else { Stance::Friendly };
    // The buff's duration is drawn before the elemental is made, as the
    // original draws it.
    let element = workings::element(summoned.element);
    let buff = summoned
        .buffed
        .then(|| {
            spells.spells.0.iter().find(|candidate| {
                crate::game::hits::definitions::row(candidate)
                    .is_some_and(|row| row.buffs_elemental == Some(element))
            })
        })
        .flatten()
        .map(|buff| {
            let turns = crate::game::hits::definitions::duration(
                &mut rng,
                buff.duration_base,
                buff.duration_per_power,
                strength,
            );
            let laid = crate::game::actors::Enchantment {
                spell: buff.id,
                power: strength.power,
                boost: 0,
            };
            (laid, turns)
        });
    let (group, spawned) = population::spawn_group(
        &mut commands,
        root,
        members,
        (ids.1, Some(SetStance { stance, at: clock.ms })),
        &spells,
        (navigation, &mut configs),
    );
    // The summoning lasts no longer than the fight.
    commands.entity(group).insert(ChildOf(battle));
    let elemental = spawned[0];
    commands.entity(elemental).insert((Combatant(battle), SummonedBy(caster), ChildOf(battle)));
    if let Some((buff, turns)) = buff {
        laying.lay(elemental, buff, turns, 0);
    }
}

/// Lures last their turns, then go, and with them their hold.
pub fn expire_lures(mut commands: Commands, lures: Query<(Entity, &Lure)>, clock: Res<GameClock>) {
    for (entity, lure) in &lures {
        if lure.expires <= clock.ms {
            commands.entity(entity).despawn();
        }
    }
}

/// A controlled monster outside the fight that can see walks to its lure
/// while it is more than five metres off, and faces it.
pub fn follow_lures(
    lures: Query<&Transform, With<Lure>>,
    mut followers: Query<
        (
            &LureInfluence,
            (&crate::game::characters::Conditions, Has<crate::game::actors::CanFight>),
            Option<&mut AgentTarget3d>,
            Option<&avian3d::prelude::Position>,
            &mut Transform,
            Option<&mut avian3d::prelude::Rotation>,
        ),
        (Without<Combatant>, Without<Lure>),
    >,
) {
    for (influence, (conditions, can_fight), walking, position, mut transform, rotation) in
        &mut followers
    {
        let target = lures.get(influence.lure).unwrap().translation;
        let here = position.map_or(transform.translation, |p| p.0);
        // It follows while it is controlled and can see, and walks to the
        // lure while it is more than five metres off.
        let follows = influence.response == Response::Controlled
            && can_fight
            && !conditions.has(Condition::Blind);
        let walks = follows && here.distance(target) > workings::LURE_HELD_AT;
        if let Some(mut walking) = walking {
            *walking =
                if walks { AgentTarget3d::Entity(influence.lure) } else { AgentTarget3d::None };
        }
        if follows && let Some(facing) = crate::game::world::facing(here, target) {
            if let Some(mut rotation) = rotation {
                rotation.0 = facing;
            }
            transform.rotation = facing;
        }
    }
}
