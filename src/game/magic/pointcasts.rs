//! Point effects commit source payment and their actual scene actors together.
use super::{
    Outcome,
    casting::{Aim, MagicChoice, MagicNews, Refusal, UseContext},
};
use crate::{
    content::{characters::Skill, monster::MonsterBody, species::MonsterKind, spells::SpellId},
    game::{
        characters::{Character, Condition},
        combat::{Battle, Combatant},
        data::GameData,
        rng::GameRng,
        time::{AGING_TICK_MS, GameClock},
        world::{
            hostility::{self, SetStance, Stance},
            identify::{GroupMember, GroupMembers, MonsterGroup},
            level::{Level, LevelOwned},
            navigation::{self, LevelNavigation, WalkerMotionConfig},
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
#[derive(Component, Clone, Copy)]
#[relationship(relationship_target = LureTargets)]
pub struct LureInfluence {
    #[relationship]
    pub lure: Entity,
    pub response: LureResponse,
}
#[derive(Component)]
#[relationship_target(relationship = LureInfluence)]
pub struct LureTargets(Vec<Entity>);

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

/// Every elemental's body definition, which a summoning needs before it
/// is paid for.
fn bodies(world: &World, root: Entity) -> Result<Vec<(u32, MonsterBody)>, Refusal> {
    let handles = world.get::<MonsterBodies>(root).ok_or(Refusal::MissingContent)?;
    let assets = world.get_resource::<Assets<MonsterBody>>().ok_or(Refusal::MissingContent)?;
    ELEMENTALS
        .map(|id| Ok((id, *handles.get(id, assets).ok_or(Refusal::MissingContent)?)))
        .collect()
}

pub(crate) fn cast(world: &mut World, caster: Entity, choice: MagicChoice) -> Result<(), Refusal> {
    if !crate::game::freedom::free_in(world) {
        return Err(Refusal::WrongTime);
    }
    let id = super::utilities::source_spell(world, caster, choice.source)?;
    if !spell(id) {
        return Err(Refusal::Unsupported);
    }
    let Aim::Point(at) = choice.aim else { return Err(Refusal::NoTarget) };
    if !at.is_finite() {
        return Err(Refusal::NoTarget);
    }
    let root = world
        .query_filtered::<Entity, With<Level>>()
        .single(world)
        .map_err(|_| Refusal::WrongTime)?;
    let battle = world.query_filtered::<Entity, With<Battle>>().single(world).ok();
    let (spell, far) = {
        let spell = GameData::spells(world)
            .and_then(|catalog| catalog.get(id))
            .cloned()
            .ok_or(Refusal::MissingContent)?;
        let level = world.get::<Level>(root).unwrap();
        let far = world
            .get_resource::<Assets<crate::content::level::LevelDefinition>>()
            .and_then(|assets| assets.get(&level.definition))
            .ok_or(Refusal::MissingContent)?
            .view_distance();
        (spell, far)
    };
    let eye = crate::game::party::body::PartyBody::eye_in(world).ok_or(Refusal::WrongTime)?;
    if eye.distance(at) > spell.reach.metres() + crate::content::EYE_HEIGHT {
        return Err(Refusal::NoTarget);
    }
    let mut obstacles = SystemState::<WorldObstacles>::new(world);
    let view = obstacles.get(world).map_err(|_| Refusal::MissingContent)?;
    if !view.clear(eye, at) {
        return Err(Refusal::NoTarget);
    }
    let definitions = if id == SpellId::SUMMON_ELEMENTAL {
        if battle.is_none() {
            return Err(Refusal::WrongTime);
        }
        if world.get::<Elementals>(caster).is_some_and(|summons| {
            summons
                .iter()
                .any(|entity| world.get::<Monster>(entity).is_some_and(|body| !body.is_dead()))
        }) {
            return Err(Refusal::NoTarget);
        }
        if world
            .get::<Character>(caster)
            .is_some_and(|profile| super::Strength::of(profile, &spell, 1).boost != 0)
        {
            let spells = GameData::spells(world).ok_or(Refusal::MissingContent)?;
            if [
                SpellId::HASTE,
                SpellId::BODY_OF_STONE,
                SpellId::MISSILE_SHIELD,
                SpellId::ARMORPLATE,
            ]
            .iter()
            .any(|&id| spells.get(id).is_none())
            {
                return Err(Refusal::MissingContent);
            }
        }
        let bodies = bodies(world, root)?;
        // Spawning needs walker configs; a walking elemental needs every
        // size class's navigation too.
        if !world.contains_resource::<Assets<WalkerMotionConfig>>()
            || bodies.iter().any(|(_, body)| body.movement_rate > 0.0)
                && world
                    .get::<LevelNavigation>(root)
                    .is_none_or(|nav| nav.archipelagos.len() < navigation::bake::SIZE_CLASSES.len())
        {
            return Err(Refusal::MissingContent);
        }
        bodies
    } else {
        vec![]
    };
    let underwater = world
        .get::<crate::game::world::triggers::LevelState>(root)
        .is_some_and(|state| state.camera_sway);
    let weights = if id == SpellId::SUMMON_ELEMENTAL {
        let profile = world.get::<Character>(caster).ok_or(Refusal::CannotAct)?;
        [Skill::FireMagic, Skill::WaterMagic, Skill::AirMagic, Skill::EarthMagic]
            .map(|skill| profile.skill(skill) + 20)
    } else {
        [0; 4]
    };
    let (report, strength) = super::sources::cast_source(
        world,
        caster,
        choice.source,
        if battle.is_some() { UseContext::Combat } else { UseContext::World },
    )?;
    if report.outcome != Outcome::Fizzled {
        let now = world.resource::<GameClock>().ms;
        if id == SpellId::HYPNOTIC_LURE {
            let old: Vec<_> = world.query_filtered::<Entity, With<Lure>>().iter(world).collect();
            for old in old {
                remove_lure(world, old);
            }
            let turns =
                super::effects::duration(&mut world.resource_mut::<GameRng>(), &spell, strength);
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
                let Some(here) = crate::game::world::actor_position(world, entity) else {
                    continue;
                };
                if (here - at).abs().max_element() > far {
                    continue;
                }
                let kind = GameData::species(world).unwrap().kind(body.species).unwrap();
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
                let clear = sight.get(world).is_ok_and(|obstacles| obstacles.clear(eye, at));
                if !clear {
                    continue;
                }
                let response =
                    if super::effects::save_roll(&mut world.resource_mut::<GameRng>(), chance) {
                        LureResponse::Resisted
                    } else {
                        LureResponse::Controlled
                    };
                let group = world
                    .get::<GroupMember>(entity)
                    .and_then(|group| world.get::<GroupMembers>(group.0));
                let affected = if battle.is_none() {
                    group.map_or_else(|| vec![entity], |members| members.iter().collect())
                } else {
                    vec![entity]
                };
                for entity in affected {
                    world.entity_mut(entity).insert(LureInfluence { lure, response });
                }
            }
        } else {
            let battle = battle.unwrap();
            let mut weights = weights;
            if underwater {
                weights[0] = 0;
            }
            let roll = world.resource_mut::<GameRng>().below(weights.iter().sum());
            let realm = crate::game::rng::weighted(roll, weights).unwrap_or(3);
            let species_id = tier(strength.power, realm);
            let kind = GameData::species(world).unwrap().kind(species_id).unwrap().clone();
            let body = &definitions.iter().find(|(id, _)| *id == species_id).unwrap().1;
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
            let ids = population::next_ids(
                locations,
                groups,
                world.get::<Unborn>(root).unwrap_or(&Unborn::default()),
            );
            let mut members = population::roll_group(
                ids,
                (species_id, 1),
                (&kind, body),
                &mut world.resource_mut::<GameRng>(),
            );
            members[0].feet = at;
            let stance = if report.outcome == Outcome::Backfired {
                Stance::Hostile
            } else {
                Stance::Friendly
            };
            // Checked before payment for elementals that walk; one that
            // stands needs none.
            let navigation = LevelNavigation {
                archipelagos: world
                    .get::<LevelNavigation>(root)
                    .map_or_else(Vec::new, |nav| nav.archipelagos.clone()),
            };
            let catalog = world.resource::<GameData>().species.clone();
            let (group, spawned) = world.resource_scope(
                |world, catalogs: Mut<Assets<crate::content::species::MonsterCatalog>>| {
                    world.resource_scope(|world, mut configs: Mut<Assets<WalkerMotionConfig>>| {
                        population::spawn_group(
                            &mut world.commands(),
                            root,
                            members,
                            (ids.1, Some(SetStance { stance, at: now })),
                            catalogs.get(&catalog).unwrap(),
                            (&navigation, &mut configs),
                        )
                    })
                },
            );
            world.flush();
            // The summoning lasts no longer than the fight.
            world.entity_mut(group).insert(ChildOf(battle));
            let elemental = spawned[0];
            world.entity_mut(elemental).insert((
                Combatant(battle),
                SummonedBy(caster),
                ChildOf(battle),
            ));
            // Power Cast may add the element's associated buff; use the same slot and duration rules.
            if strength.boost != 0 && world.resource_mut::<GameRng>().below(100) < strength.boost {
                let buff = [
                    SpellId::HASTE,
                    SpellId::BODY_OF_STONE,
                    SpellId::MISSILE_SHIELD,
                    SpellId::ARMORPLATE,
                ][realm];
                let definition = GameData::spells(world).and_then(|c| c.get(buff)).cloned();
                if let Some(definition) = definition {
                    let turns = super::effects::duration(
                        &mut world.resource_mut::<GameRng>(),
                        &definition,
                        strength,
                    );
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
        }
    }
    world.write_message(MagicNews::Done(report));
    Ok(())
}

fn remove_lure(world: &mut World, entity: Entity) {
    let affected: Vec<_> =
        world.get::<LureTargets>(entity).map_or_else(Vec::new, |targets| targets.iter().collect());
    for actor in affected {
        if world.get::<Combatant>(actor).is_none()
            && let Some(mut target) = world.get_mut::<AgentTarget3d>(actor)
        {
            *target = AgentTarget3d::None;
        }
    }
    world.despawn(entity);
}

pub(crate) fn update_lure(world: &mut World) {
    let Some(now) = world.get_resource::<GameClock>().map(|clock| clock.ms) else { return };
    let expired: Vec<_> = world
        .query::<(Entity, &Lure)>()
        .iter(world)
        .filter(|(_, lure)| lure.expires <= now)
        .map(|(entity, _)| entity)
        .collect();
    for entity in expired {
        remove_lure(world, entity);
    }
    let followers: Vec<_> = world
        .query::<(Entity, &LureInfluence)>()
        .iter(world)
        .map(|(entity, influence)| (entity, *influence))
        .collect();
    for (entity, influence) in followers {
        if world.get::<Combatant>(entity).is_some() {
            continue;
        }
        let target = crate::game::world::actor_position(world, influence.lure);
        let follows = world
            .get::<Monster>(entity)
            .is_some_and(|body| body.can_fight() && !body.conditions.has(Condition::Blind))
            && influence.response == LureResponse::Controlled;
        let here = crate::game::world::actor_position(world, entity);
        let goal = match (follows, here, target) {
            (true, Some(here), Some(target)) if here.distance(target) > 5.0 => {
                AgentTarget3d::Entity(influence.lure)
            }
            _ => AgentTarget3d::None,
        };
        if let Some(mut walking) = world.get_mut::<AgentTarget3d>(entity) {
            *walking = goal;
        }
        if follows && let Some(target) = target {
            crate::game::world::face(world, entity, target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::{
            characters::Realm,
            species::{MonsterCatalog, Species},
            spells::{SpellCatalog, SpellDefinition, Spellbook, Targeting, Usable},
        },
        game::{
            characters::{Gender, Profession, Race, Vitals},
            inventory::{Carried, PartyInventory},
            magic::{LearnedSpells, SpellPoints, casting::Source},
            world::{population::ScaledBody, scripts::LevelScript, triggers::tests::Scenario},
        },
    };
    use bevy::ecs::system::RunSystemOnce;
    fn scene() -> Scenario {
        let mut scene = Scenario::new(vec![], &[], LevelScript::None);
        let world = &mut scene.world;
        world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        world.init_resource::<crate::game::factions::Factions>();
        world.init_resource::<Assets<MonsterCatalog>>();
        world.init_resource::<Assets<SpellCatalog>>();
        world.init_resource::<Assets<MonsterBody>>();
        world.init_resource::<Assets<WalkerMotionConfig>>();
        world.init_resource::<Messages<MagicNews>>();
        world.init_resource::<Messages<crate::game::factions::FactionNews>>();
        let mut species = vec![Species::default(); 49];
        for kind in &mut species {
            kind.experience = 9000;
        }
        let species = world.resource_mut::<Assets<MonsterCatalog>>().add(MonsterCatalog(species));
        let mut spells = vec![SpellDefinition::default(); 62];
        for id in [SpellId::HYPNOTIC_LURE, SpellId::SUMMON_ELEMENTAL] {
            spells[id.0 as usize] = SpellDefinition {
                id,
                realm: Some(Realm::Fire),
                cost: 1,
                books: Spellbook::Wizardry.into(),
                targeting: Targeting::Point,
                reach: crate::content::species::Reach::Long,
                usable: if id == SpellId::SUMMON_ELEMENTAL {
                    Usable::InCombat
                } else {
                    Usable::Always
                },
                duration_base: 5,
                ..default()
            };
        }
        let spells = world.resource_mut::<Assets<SpellCatalog>>().add(SpellCatalog(spells));
        {
            let mut data = world.resource_mut::<GameData>();
            data.spells = spells;
            data.species = species;
        }
        let body = world
            .resource_mut::<Assets<MonsterBody>>()
            .add(MonsterBody { movement_rate: 0.0, ..default() });
        for id in 21..=48 {
            world.get_mut::<MonsterBodies>(scene.root).unwrap().0.insert(id, body.clone());
        }
        let mut profile = Character::new("Caller", Race::Human, Gender::Male, Profession::Mage);

        profile.profession_levels[Profession::Mage] = 30;
        profile.skill_points[Skill::Wizardry] = 100;
        profile.skill_points[Skill::FireMagic] = 100;
        world.entity_mut(scene.members[0]).insert((
            Vitals::new(&profile),
            profile,
            Carried::default(),
            LearnedSpells([SpellId::HYPNOTIC_LURE, SpellId::SUMMON_ELEMENTAL].into()),
            SpellPoints::default(),
        ));
        crate::game::magic::test_set_points(world, scene.members[0], Realm::Fire, 100);
        scene
    }
    fn choice(spell: SpellId) -> MagicChoice {
        MagicChoice {
            source: Source::Spell { spell, power: 1 },
            aim: Aim::Point(Vec3::new(1.0, 0.0, -3.0)),
        }
    }
    #[test]
    fn summoning_validates_definitions_before_payment_and_uses_native_lifetimes_without_rewards() {
        let mut scene = scene();
        let caster = scene.members[0];
        let root = scene.root;
        let world = &mut scene.world;
        let battle = world.spawn((Battle::default(), LevelOwned(root))).id();
        let definitions = world.entity_mut(root).take::<MonsterBodies>().unwrap();
        let rng = world.resource::<GameRng>().clone();
        assert_eq!(
            cast(world, caster, choice(SpellId::SUMMON_ELEMENTAL)),
            Err(Refusal::MissingContent)
        );
        assert_eq!(&rng, world.resource::<GameRng>());
        assert_eq!(crate::game::magic::test_points(world, caster, Realm::Fire), 100);
        world.entity_mut(root).insert(definitions);
        cast(world, caster, choice(SpellId::SUMMON_ELEMENTAL)).unwrap();
        let summon = world.get::<Elementals>(caster).unwrap().iter().next().unwrap();
        assert!((21..=24).contains(&world.get::<Monster>(summon).unwrap().species));
        assert_eq!(crate::game::magic::test_points(world, caster, Realm::Fire), 99);
        assert_eq!(cast(world, caster, choice(SpellId::SUMMON_ELEMENTAL)), Err(Refusal::NoTarget));
        let party = crate::game::party::roster::the_party(world).unwrap();
        let before = world.get::<PartyInventory>(party).unwrap().gold;
        let result = world
            .run_system_once(
                move |mut damage: crate::game::world::damage::MonsterDamage,
                      mut inventory: Single<&mut PartyInventory>,
                      mut rng: ResMut<GameRng>| {
                    damage.apply(summon, 999, (1, 0), &mut inventory, &mut rng)
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.experience, 0);
        assert_eq!(world.get::<PartyInventory>(party).unwrap().gold, before);
        cast(world, caster, choice(SpellId::SUMMON_ELEMENTAL)).unwrap();
        let new = world
            .get::<Elementals>(caster)
            .unwrap()
            .iter()
            .find(|&entity| entity != summon)
            .unwrap();
        world.despawn(battle);
        assert!(world.get_entity(new).is_err());
        assert!(world.get::<Character>(caster).is_some());
    }
    #[test]
    fn a_lure_owns_control_and_expiry_and_damage_breaks_it_without_despawning_the_monster() {
        let mut scene = scene();
        let caster = scene.members[0];
        let root = scene.root;
        let world = &mut scene.world;
        let target = world
            .spawn((
                Monster {
                    enchantments: default(),
                    location: 1,
                    species: 0,
                    hp: 10,
                    max_hp: 10,
                    stamina: 10,
                    max_stamina: 10,
                    conditions: Default::default(),
                    spell_points: 0,
                    body: ScaledBody { scale: 1.0, radius: 1.0, eye_height: 1.0, speed: 1.0 },
                },
                Transform::from_xyz(1.0, 0.0, -15.0),
                LevelOwned(root),
                AgentTarget3d::None,
            ))
            .id();
        for seed in 0..30 {
            world.insert_resource(GameRng::seeded(seed));
            cast(world, caster, choice(SpellId::HYPNOTIC_LURE)).unwrap();
            if world
                .get::<LureInfluence>(target)
                .is_some_and(|influence| influence.response == LureResponse::Controlled)
            {
                break;
            }
        }
        let influence = *world.get::<LureInfluence>(target).unwrap();
        assert_eq!(influence.response, LureResponse::Controlled);
        update_lure(world);
        assert_eq!(
            world.get::<AgentTarget3d>(target),
            Some(&AgentTarget3d::Entity(influence.lure))
        );
        world
            .run_system_once(
                move |mut damage: crate::game::world::damage::MonsterDamage,
                      mut inventory: Single<&mut PartyInventory>,
                      mut rng: ResMut<GameRng>| {
                    damage.apply(target, 1, (1, 0), &mut inventory, &mut rng)
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(world.get::<LureInfluence>(target).unwrap().response, LureResponse::Resisted);
        assert_eq!(world.get::<AgentTarget3d>(target), Some(&AgentTarget3d::None));
        world.resource_mut::<GameClock>().advance(AGING_TICK_MS * 10);
        update_lure(world);
        assert!(world.get::<LureInfluence>(target).is_none());
        assert!(world.get::<Monster>(target).is_some());
    }
}
