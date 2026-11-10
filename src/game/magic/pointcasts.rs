//! Point effects commit source payment and their actual scene actors together.
use super::{
    Outcome, Utility,
    casting::{Aim, MagicNews, Refusal, Refused, UseContext},
};
use crate::game::data::Catalogs;
use crate::{
    content::{characters::Skill, monster::MonsterBody, species::MonsterKind, spells::SpellId},
    game::{
        characters::Condition,
        combat::{Battle, Combatant},
        difficulty::Difficulty,
        freedom::Freedom,
        hits::effects::Body,
        items::Items,
        party::body::PartyBody,
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

/// How a monster answers a lure (retail `MonsterResistsSpellEffect`): an
/// undead one always resists; the rest save in the Mental realm, their level
/// against the lure's strength, unscaled by difficulty.
fn lure_response(
    kind: Option<MonsterKind>,
    (level, resistance, against): (u32, i32, u32),
    rng: &mut GameRng,
) -> LureResponse {
    let chance = crate::game::hits::defences::condition_chance(level, resistance, against, 0);
    if kind == Some(MonsterKind::Undead) || crate::game::hits::defences::save_roll(rng, chance) {
        LureResponse::Resisted
    } else {
        LureResponse::Controlled
    }
}

pub fn spell(id: SpellId) -> bool {
    matches!(id, SpellId::HYPNOTIC_LURE | SpellId::SUMMON_ELEMENTAL)
}
fn tier(power: u32, realm: usize) -> u32 {
    // Retail g_elemental_summon_group_ids: air, earth, water, fire per tier.
    21 + (power.clamp(1, 7) - 1) * 4 + [3, 2, 0, 1][realm]
}
/// The spell Power Cast may lay on a summoned elemental of the element at
/// `realm` (fire, water, air, earth): Haste, Superman, Missile Shield or
/// Armorplate (retail `ApplySummonElementalEffect`, `Magic Effects.cpp:840`
/// and the spells 0x38, 0x3d, 0x1a, 0x20 it names).
fn element_buff(realm: usize) -> SpellId {
    [SpellId::HASTE, SpellId::SUPERMAN, SpellId::MISSILE_SHIELD, SpellId::ARMORPLATE][realm]
}

/// The elementals Summon Elemental may call, four elements to a tier
/// (retail `g_elemental_summon_group_ids`). Their bodies load with every
/// level ([`MonsterBodies::for_level`]).
pub const ELEMENTALS: std::ops::RangeInclusive<u32> = 21..=48;

/// A point cast (retail's Hypnotic Lure and Summon Elemental): at a spot
/// the party sees within the spell's reach. Its target is settled before
/// the cast is paid for; whatever the roll short of a fizzle, something
/// comes of it.
#[allow(clippy::type_complexity)]
pub(crate) fn cast(
    accepted: On<Utility>,
    mut commands: Commands,
    mut bodies: Query<Body>,
    (party, held): (Single<&avian3d::prelude::Position, With<PartyBody>>, Items),
    level: Single<(Entity, &Level, &LevelState, &Unborn, &MonsterBodies, &LevelNavigation)>,
    battle: Option<Single<(Entity, &crate::game::combat::Round), With<Battle>>>,
    summoned: Query<&Elementals>,
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
    (freedom, obstacles): (Freedom, WorldObstacles),
    mut laying: crate::game::actors::Laying,
    (definitions, body_assets, mut configs): (
        Res<Assets<crate::content::level::LevelDefinition>>,
        Res<Assets<MonsterBody>>,
        ResMut<Assets<WalkerMotionConfig>>,
    ),
    (content, difficulty, clock): (Res<Catalogs>, Res<Difficulty>, Res<GameClock>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
) {
    let Utility { caster, spell: id, choice } = *accepted;
    if !spell(id) {
        return;
    }
    let (root, played, state, unborn, level_bodies, navigation) = *level;
    let moment = battle.as_ref().map_or(0, |battle| battle.1.moment());
    let battle = battle.map(|battle| battle.0);
    let now = clock.ms;
    let position = *party;
    let spell = &content.spells[id];
    let eye = PartyBody::eye(position);
    let mut pay = || -> Result<_, Refusal> {
        if !freedom.free() {
            return Err(Refusal::WrongTime);
        }
        let Aim::Point(at) = choice.aim else { return Err(Refusal::NoTarget) };
        if eye.distance(at) > spell.reach.metres() + crate::content::EYE_HEIGHT
            || !obstacles.clear(eye, at)
        {
            return Err(Refusal::NoTarget);
        }
        if id == SpellId::SUMMON_ELEMENTAL {
            if battle.is_none() {
                return Err(Refusal::WrongTime);
            }
            let standing = summoned.get(caster).is_ok_and(|summons| {
                summons.iter().any(|e| bodies.get(e).is_ok_and(|b| !b.vitals.is_dead()))
            });
            if standing {
                return Err(Refusal::NoTarget);
            }
        }
        let mut body = bodies.get_mut(caster).map_err(|_| Refusal::CannotAct)?;
        // The element is weighted by the summoner's realm skills as
        // they stand before the cast is practised.
        let weights = body.character.as_deref().map(|profile| {
            [Skill::FireMagic, Skill::WaterMagic, Skill::AirMagic, Skill::EarthMagic]
                .map(|skill| profile.skill(skill) + 20)
        });
        let context = if battle.is_some() { UseContext::Combat } else { UseContext::World };
        let (report, strength) = super::casting::pay(
            &mut body,
            &held,
            (choice.source, context),
            (&content, *difficulty, state.camera_sway),
            moment,
            &mut rng,
            &mut commands,
        )?;
        Ok((report, strength, at, weights.unwrap_or_default()))
    };
    let (report, strength, at, mut weights) = match pay() {
        Ok(paid) => paid,
        Err(refusal) => {
            commands.trigger(Refused { caster, refusal });
            return;
        }
    };
    let outcome = report.outcome;
    news.write(MagicNews::Done(report));
    if outcome == Outcome::Fizzled {
        return;
    }
    let fighting_now = battle.is_some();
    let Some(battle) = battle.filter(|_| id == SpellId::SUMMON_ELEMENTAL) else {
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
                Lure { expires: now + u64::from(turns) * AGING_TICK_MS },
                Transform::from_translation(at),
                LevelOwned(root),
            ))
            .id();
        let far = definitions.get(&played.definition).unwrap().view_distance();
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
            if body.stance() != Stance::Hostile
                || influence.is_some_and(|held| held.response == LureResponse::Resisted)
                || !body.vitals.can_fight()
                || body.vitals.conditions.has(Condition::Blind)
                || (here - at).abs().max_element() > far
                || !obstacles.clear(monster.eye(here), at)
            {
                continue;
            }
            let response = lure_response(
                body.resistances.kind,
                (
                    body.level.0,
                    body.resistances.realms[crate::content::characters::Realm::Mental],
                    strength.against,
                ),
                &mut rng,
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
    // summoner's realm skills (none of fire under water), its tier by
    // the power, friendly unless the summoning backfired. It joins the
    // fight and lasts no longer; Power Cast may add its element's buff.
    if state.camera_sway {
        weights[0] = 0;
    }
    let realm = crate::game::rng::weighted(rng.below(weights.iter().sum()), weights).unwrap_or(3);
    let species = tier(strength.power, realm);
    let locations = monsters
        .iter()
        .filter(|(_, _, owner, ..)| owner.0 == root)
        .map(|(_, monster, ..)| monster.location);
    let authored =
        groups.iter().filter(|(_, owner, _)| owner.0 == root).map(|(group, ..)| group.authored);
    let ids = population::next_ids(locations, authored, unborn);
    let kind = &content.species[species];
    let body = level_bodies.get(species, &body_assets).unwrap();
    let mut members = population::roll_group(ids, (species, 1), (kind, body), &mut rng);
    members[0].feet = at;
    let stance = if outcome == Outcome::Backfired { Stance::Hostile } else { Stance::Friendly };
    // Power Cast may add the element's associated buff, by the same
    // slot and duration rules as the spell itself.
    let mut buffed = None;
    if strength.boost != 0 && rng.below(100) < strength.boost {
        let buff = element_buff(realm);
        let definition = &content.spells[buff];
        let turns = crate::game::hits::definitions::duration(
            &mut rng,
            definition.duration_base,
            definition.duration_per_power,
            strength,
        );
        let laid =
            crate::game::actors::Enchantment { spell: buff, power: strength.power, boost: 0 };
        buffed = Some((laid, turns));
    }
    let (group, spawned) = population::spawn_group(
        &mut commands,
        root,
        members,
        (ids.1, Some(SetStance { stance, at: now })),
        &content.species,
        (navigation, &mut configs),
    );
    // The summoning lasts no longer than the fight.
    commands.entity(group).insert(ChildOf(battle));
    let elemental = spawned[0];
    commands.entity(elemental).insert((Combatant(battle), SummonedBy(caster), ChildOf(battle)));
    if let Some((buff, turns)) = buffed {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Retail names spells 0x38, 0x3d, 0x1a and 0x20 for fire, water, air and
    /// earth: 56 Haste, 61 Superman, 26 Missile Shield, 32 Armorplate.
    /// Level 1 against a strength of 100, in a realm of resistance -100:
    /// (1 - 100)*3 + 0 - 100 = -397, held to the least save, 5%.
    #[test]
    fn an_undead_monster_always_resists_a_lure_and_the_rest_save_by_the_rule() {
        let mut rng = GameRng::seeded(7);
        let weak = (1, -100, 100);
        let answers = |kind, rng: &mut GameRng| {
            (0..400).filter(|_| lure_response(kind, weak, rng) == LureResponse::Resisted).count()
        };
        assert_eq!(answers(Some(MonsterKind::Undead), &mut rng), 400);
        // Five in a hundred save: far from all, far from none.
        let saved = answers(Some(MonsterKind::Beast), &mut rng);
        assert!((5..=50).contains(&saved), "{saved}");
    }

    #[test]
    fn an_elemental_is_buffed_by_the_spell_of_its_element() {
        let buffs: Vec<u8> = (0..4).map(|realm| element_buff(realm).0).collect();
        assert_eq!(buffs, [0x38, 0x3d, 0x1a, 0x20]);
    }
}
