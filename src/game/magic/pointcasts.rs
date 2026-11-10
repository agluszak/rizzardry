//! Hypnotic Lure and Summon Elemental (`docs/rules/casting.md` section 8): once
//! the cast comes off, a lure stands at the spot and holds the monsters that
//! fail to resist it, or an elemental answers the summoner. The cast is paid
//! for and rolled as any other ([`super::casting`]); the arithmetic of what it
//! then does is [`super::workings`].
use super::Outcome;
use super::casting::{MagicNews, Worked};
use super::reach::Marked;
use super::workings::{self, Response};
use crate::content::characters::{Realm, Skill};
use crate::content::monster::MonsterBody;
use crate::content::spells::Working;
use crate::game::data::Catalogs;
use crate::game::{
    characters::Condition,
    combat::{Battle, Combatant},
    hits::definitions::Definitions,
    hits::effects::BodyReadOnly,
    rng::GameRng,
    time::{AGING_TICK_MS, GameClock},
    world::{
        hostility::{SetStance, Stance},
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
#[derive(Component)]
pub struct Lure {
    expires: u64,
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

/// A point cast (retail's Hypnotic Lure and Summon Elemental), once it is paid
/// for and rolled as any other ([`super::casting`]), its spot settled before:
/// whatever the roll short of a fizzle, something comes of it.
#[allow(clippy::type_complexity)]
pub(crate) fn cast(
    worked: On<Worked>,
    mut commands: Commands,
    bodies: Query<BodyReadOnly>,
    level: Single<(Entity, &Level, &LevelState, &Unborn, &MonsterBodies, &LevelNavigation)>,
    battle: Option<Single<Entity, With<Battle>>>,
    lures: Query<Entity, With<Lure>>,
    monsters: Query<(
        Entity,
        &Monster,
        &LevelOwned,
        crate::game::world::ActorPose,
        Option<&GroupMember>,
        Option<&LureInfluence>,
    )>,
    groups: Query<(&MonsterGroup, &LevelOwned, &GroupMembers)>,
    obstacles: WorldObstacles,
    mut laying: crate::game::actors::Laying,
    (definitions, spells, body_assets, level_assets, mut configs): (
        Res<Definitions>,
        Res<Catalogs>,
        Res<Assets<MonsterBody>>,
        Res<Assets<crate::content::level::LevelDefinition>>,
        ResMut<Assets<WalkerMotionConfig>>,
    ),
    clock: Res<GameClock>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) {
    let Worked { caster, working, report, strength, marked } = worked.event().clone();
    if !matches!(working, Working::Lure | Working::Summon) {
        return;
    }
    let Marked::Spot(at) = marked else { return };
    let (root, played, state, unborn, level_bodies, navigation) = *level;
    let battle = battle.map(|battle| *battle);
    let (outcome, id) = (report.outcome, report.spell);
    let spell = &spells.spells[id];
    let now = clock.ms;
    news.write(MagicNews::Done(report));
    if outcome == Outcome::Fizzled {
        return;
    }
    let fighting_now = battle.is_some();
    let Some(battle) = battle.filter(|_| working == Working::Summon) else {
        // A Hypnotic Lure stands at `at` in place of any before it. Each
        // hostile monster able to see it, within the far clip, may resist (an
        // undead one always does); one that does not qualify is passed over;
        // outside a fight its whole group follows its choice (retail
        // `ApplyMonsterControlToNearbyMonsters`).
        for old in &lures {
            commands.entity(old).despawn();
        }
        let turns = crate::game::hits::definitions::duration(
            &mut rng,
            spell.duration_base,
            spell.duration_per_power,
            strength,
        );
        let lure = commands
            .spawn((
                Lure { expires: workings::lure_ends(now, turns, AGING_TICK_MS) },
                Transform::from_translation(at),
                LevelOwned(root),
            ))
            .id();
        let far = level_assets.get(&played.definition).unwrap().view_distance();
        let mut order: Vec<_> = monsters
            .iter()
            .filter(|(_, _, owner, ..)| owner.0 == root)
            .map(|(entity, monster, ..)| (monster.location, entity))
            .collect();
        order.sort_unstable();
        for (_, entity) in order {
            let (_, monster, _, pose, group, influence) = monsters.get(entity).unwrap();
            let Ok(body) = bodies.get(entity) else { continue };
            let group = group.and_then(|group| groups.get(group.0).ok());
            let here = pose.translation();
            let drawn = workings::Drawn {
                hostile: body.stance() == Stance::Hostile,
                resisted: influence.is_some_and(|held| held.response == Response::Resisted),
                can_fight: body.vitals.can_fight(),
                blind: body.vitals.conditions.has(Condition::Blind),
                off: (here - at).abs().max_element(),
                far,
                clear: obstacles.clear(monster.eye(here), at),
            };
            if !workings::lure_tests(&drawn) {
                continue;
            }
            let response = workings::lure_response(
                &mut rng,
                &workings::Lured {
                    kind: body.resistances.kind,
                    level: body.level.0,
                    resistance: body.resistances.realms[Realm::Mental],
                    against: strength.against,
                },
            );
            let affected = match group {
                Some((.., members)) if !fighting_now => members.iter().collect(),
                _ => vec![entity],
            };
            for entity in affected {
                commands.entity(entity).insert(LureInfluence { lure, response });
            }
        }
        return;
    };
    // An elemental answers the summoner: its element weighted by the
    // summoner's realm skills (none of fire under water), its tier by the
    // power, friendly unless the summoning backfired. It joins the fight and
    // lasts no longer; Power Cast may add its element's buff.
    let skills = bodies.get(caster).ok().and_then(|body| {
        body.character.map(|character| {
            [Skill::FireMagic, Skill::WaterMagic, Skill::AirMagic, Skill::EarthMagic]
                .map(|skill| character.skill(skill))
        })
    });
    let summoned = workings::summon(
        &mut rng,
        &workings::Summoning { skills, under_water: state.camera_sway, outcome, strength },
    );
    let locations = monsters
        .iter()
        .filter(|(_, _, owner, ..)| owner.0 == root)
        .map(|(_, monster, ..)| monster.location);
    let authored =
        groups.iter().filter(|(_, owner, _)| owner.0 == root).map(|(group, ..)| group.authored);
    let ids = population::next_ids(locations, authored, unborn);
    let kind = &spells.species[summoned.species];
    let body = level_bodies.get(summoned.species, &body_assets).unwrap();
    let mut members = population::roll_group(ids, (summoned.species, 1), (kind, body), &mut rng);
    members[0].feet = at;
    let stance = if summoned.backfired { Stance::Hostile } else { Stance::Friendly };
    // Power Cast may add the element's associated buff, by the same slot and
    // duration rules as the spell itself.
    let element = workings::element(summoned.element);
    let buff = summoned
        .buffed
        .then(|| {
            spells.spells.0.iter().find(|candidate| {
                definitions
                    .spell(candidate.id)
                    .is_some_and(|row| definitions.get(row).buffs_elemental == Some(element))
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
        (ids.1, Some(SetStance { stance, at: now })),
        &spells.species,
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
            &crate::game::characters::Vitals,
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
        let (follows, walks) = workings::follows_lure(&workings::Following {
            response: influence.response,
            can_fight: body.can_fight(),
            blind: body.conditions.has(Condition::Blind),
            distance: here.distance(target),
        });
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
