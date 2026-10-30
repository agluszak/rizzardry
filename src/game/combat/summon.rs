//! Monsters a special attack calls up mid-fight (retail
//! `SpawnSummonedMonsterGroup`): a group of the summoning's species comes
//! for the summoner's side and joins the fight.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::{Battle, CombatNews, Combatant};
use crate::content::monster::MonsterBody;
use crate::game::party::body::{self, PartyBody};
use crate::game::rng::GameRng;
use crate::game::time::GameClock;
use crate::game::world::hostility::{self, SetStance, Stance};
use crate::game::world::identify::MonsterGroup;
use crate::game::world::level::{Level, PlayedLevel};
use crate::game::world::navigation::{LevelNavigation, Walker, WalkerMotionConfig, feet};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{self, Monster, MonsterBodies, Unborn};
use crate::game::world::{ActorPose, STEP_HEIGHT};

/// A summoning landing: a group of `species` comes for the summoner.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summon {
    pub summoner: Entity,
    pub species: u32,
}

/// How far a scattered spot's ground may lie below the spot (retail's 2500
/// units).
const GROUND_BELOW: f32 = 5.0;
/// Retail tries the near-party spot, then 1500 and 3000 units out.
const NEAR_PARTY: [f32; 3] = [0.0, 3.0, 6.0];

/// Call up the groups summonings ask for: as many as the species' group
/// dice roll (nine at most), with the summoner's side (friendly when it is
/// friendly, else hostile). When the party sees the summoner they come at
/// the party's back, near or a little further off; otherwise where the
/// summoner stands. Those the ground will not hold stay away, and if none
/// will fit the summoning fails. They join the fight.
pub fn summon(
    mut commands: Commands,
    mut summons: MessageReader<Summon>,
    battle: Option<Single<Entity, With<Battle>>>,
    party: Query<(&Position, &PartyBody)>,
    (monsters, groups, stances): (
        Query<(&Monster, ActorPose, Option<&Walker>)>,
        Query<&MonsterGroup>,
        hostility::Stances,
    ),
    mut levels: Query<(Entity, &LevelNavigation, &mut Unborn, &MonsterBodies), With<Level>>,
    (level, obstacles): (PlayedLevel, WorldObstacles),
    (definitions, mut configs): (Res<Assets<MonsterBody>>, ResMut<Assets<WalkerMotionConfig>>),
    (clock, mut rng): (Res<GameClock>, ResMut<GameRng>),
    content: If<Res<crate::game::data::Catalogs>>,
    mut news: MessageWriter<CombatNews>,
) {
    let (Some(battle), Some(far)) = (battle, level.view_distance()) else {
        summons.clear();
        return;
    };
    let species = &content.species;
    let Ok((party_at, party_body)) = party.single() else { return };
    let Ok((root, navigation, mut unborn, bodies)) = levels.single_mut() else { return };
    let eye = PartyBody::eye(party_at);
    // The next free ids, the unborn's counted too. Groups spawned here are
    // not in the queries until this run ends, so each takes its ids from
    // these and moves them on.
    let (mut next_location, mut next_group) = population::next_ids(
        monsters.iter().map(|(m, ..)| m.location),
        groups.iter().map(|g| g.authored),
        &unborn,
    );
    // Where this run's earlier groups landed, and their radii: they too are
    // not in the queries, yet the next group must stand clear of them.
    let mut landed: Vec<(Vec3, f32)> = vec![];
    for &Summon { summoner, species: id } in summons.read() {
        let Ok((caller, caller_at, caller_walker)) = monsters.get(summoner) else { continue };
        let Some(kind) = species.kind(id) else { continue };
        let Some(definition) = bodies.get(id, &definitions) else {
            warn!("summoned species {id} has no body ready");
            continue;
        };
        let authored = next_group;
        let count = rng.roll(kind.group_size);
        let mut members = population::roll_group(
            (next_location, authored),
            (id, count),
            (kind, definition),
            &mut rng,
        );
        let count = members.len();
        if count == 0 {
            continue;
        }
        let stance = match stances.own(summoner, species, clock.ms) {
            Some(Stance::Friendly) => Stance::Friendly,
            _ => Stance::Hostile,
        };
        let caller_feet = feet(caller_at.translation(), caller_walker);
        let caller_eye = caller.eye(caller_feet);
        let seen = eye.distance(caller_eye) <= far && obstacles.clear(eye, caller_eye);
        let radius = members[0].monster.body.radius;
        let spacing = 2.0 * radius;
        let ground = |spot: Vec3| obstacles.ground_below(spot, GROUND_BELOW);
        let clear = |spot: Vec3| {
            let apart = |at: Vec3, radius: f32| {
                at.with_y(0.0).distance(spot.with_y(0.0)) >= radius + spacing / 2.0
            };
            apart(party_at.0, body::RADIUS)
                && landed.iter().all(|&(at, radius)| apart(at, radius))
                && monsters.iter().all(|(other, at, walker)| {
                    other.is_dead() || apart(feet(at.translation(), walker), other.body.radius)
                })
        };
        let reaches = |spot: Vec3, from: Vec3| {
            obstacles.clear(from + Vec3::Y * STEP_HEIGHT, spot + Vec3::Y * STEP_HEIGHT)
        };
        // They face the party.
        let facing = |at: Vec3| {
            let toward = party_at.0 - at;
            body::yaw_toward(toward)
        };
        let spots = if seen {
            NEAR_PARTY
                .iter()
                .map(|&distance| {
                    let heading = party_body.yaw + rng.below(1000) as f32 * 0.0016 - 0.8;
                    let distance =
                        if distance > 0.0 { distance } else { body::RADIUS + radius + 0.002 };
                    // At the party's feet's height, behind it.
                    let at = PartyBody::feet(party_at)
                        + Quat::from_rotation_y(heading) * Vec3::Z * distance;
                    population::scatter(
                        (at, facing(at), spacing),
                        count,
                        &mut rng,
                        ground,
                        clear,
                        reaches,
                    )
                })
                .find(|spots| !spots.is_empty())
                .unwrap_or_default()
        } else {
            population::scatter(
                (caller_feet, facing(caller_feet), spacing),
                count,
                &mut rng,
                ground,
                clear,
                reaches,
            )
        };
        if spots.is_empty() {
            news.write(CombatNews::SummonFails);
            continue;
        }
        members.truncate(spots.len());
        landed.extend(spots.iter().map(|&spot| (spot, radius)));
        next_location += members.len() as i32;
        next_group += 1;
        for (member, &spot) in members.iter_mut().zip(&spots) {
            member.feet = spot;
            member.yaw = facing(spots[0]);
        }
        let set = Some(SetStance { stance, at: clock.ms });
        if kind.unborn {
            unborn.0.extend(members);
            continue;
        }
        let (_, joined) = population::spawn_group(
            &mut commands,
            root,
            members,
            (authored, set),
            species,
            (navigation, &mut configs),
        );
        for member in joined {
            commands.entity(member).insert(Combatant(*battle));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::Dice;
    use crate::content::level::LevelDefinition;
    use crate::content::species::{MonsterCatalog, Species};
    use crate::game::combat::Combatant;
    use crate::game::world::identify::GroupMember;
    use crate::game::world::obstacles::scenery;
    use crate::game::world::population::ScaledBody;
    use avian3d::prelude::{Collider, PhysicsPlugins, RigidBody};
    use bevy::ecs::system::RunSystemOnce;

    /// A summoner 10 m before a party facing it, on a floor, and maybe a
    /// wall between them; returns the app, the summoner and the battle.
    fn scene(wall: bool) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::default(),
        ))
        .init_resource::<GameClock>()
        .insert_resource(GameRng::seeded(3))
        .init_resource::<crate::game::factions::Factions>()
        .init_asset::<MonsterBody>()
        .init_asset::<LevelDefinition>()
        .init_resource::<Assets<WalkerMotionConfig>>()
        .add_message::<Summon>()
        .add_message::<CombatNews>();
        app.finish();
        app.cleanup();
        let world = app.world_mut();
        let mut catalog = vec![Species::default(); 13];
        catalog[12] = Species { group_size: Dice { base: 3, count: 0, sides: 0 }, ..default() };
        let species = MonsterCatalog(catalog);
        world.insert_resource(crate::game::data::Catalogs { species, ..default() });
        let body = world.resource_mut::<Assets<MonsterBody>>().add(MonsterBody {
            scale: 1.0,
            fight_radius: 0.5,
            target_height: 1.5,
            ..default()
        });
        let level_bodies = MonsterBodies([(12, body)].into());
        let definition =
            world.resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition::default());
        let root = world
            .spawn((
                Level::test(definition),
                level_bodies,
                LevelNavigation { archipelagos: vec![] },
                Unborn::default(),
            ))
            .id();
        world.spawn((Position(Vec3::Y * body::FLOAT_HEIGHT), PartyBody::default()));
        world.spawn((
            RigidBody::Static,
            Collider::cuboid(100.0, 0.2, 100.0),
            scenery(),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        if wall {
            world.spawn((
                RigidBody::Static,
                Collider::cuboid(8.0, 8.0, 0.2),
                scenery(),
                Transform::from_xyz(0.0, 0.0, -5.0),
            ));
        }
        let battle = world.spawn(Battle::default()).id();
        let summoner = world
            .spawn((
                Monster {
                    enchantments: default(),
                    location: 4,
                    species: 0,
                    hp: 10,
                    max_hp: 10,
                    stamina: 10,
                    max_stamina: 10,
                    conditions: Default::default(),
                    spell_points: 0,
                    body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.5, speed: 0.0 },
                },
                Transform::from_xyz(0.0, 0.0, -10.0),
                crate::game::world::level::LevelOwned(root),
                Combatant(battle),
            ))
            .id();
        app.update();
        app.update();
        (app, summoner, battle)
    }

    /// Those summoned: where they stand, and whether they joined the fight.
    fn called(app: &mut App) -> Vec<(Monster, Vec3, Option<Entity>)> {
        let world = app.world_mut();
        world
            .query::<(&Monster, &Transform, Option<&Combatant>)>()
            .iter(world)
            .filter(|(m, ..)| m.species == 12)
            .map(|(m, at, fighting)| (m.clone(), at.translation, fighting.map(|c| c.0)))
            .collect()
    }

    /// Seen by the party, a summoner's group comes at the party's back,
    /// apart, on the summoner's side and into the fight, under the next
    /// free ids (retail `SpawnSummonedMonsterGroup`).
    #[test]
    fn a_seen_summoner_calls_its_group_up_behind_the_party() {
        let (mut app, summoner, battle) = scene(false);
        app.world_mut().write_message(Summon { summoner, species: 12 });
        app.world_mut().run_system_once(summon).unwrap();
        let called = called(&mut app);
        assert_eq!(called.len(), 3);
        let mut locations: Vec<_> = called.iter().map(|(m, ..)| m.location).collect();
        locations.sort();
        assert_eq!(locations, [5, 6, 7]);
        for (_, at, fighting) in &called {
            assert!(at.z > 0.5, "behind the party: {at}");
            assert_eq!(*fighting, Some(battle));
        }
        // A step (two radii) apart, give or take retail's fifth-of-a-step
        // jitter each way.
        for (i, (_, a, _)) in called.iter().enumerate() {
            for (_, b, _) in &called[i + 1..] {
                assert!(a.with_y(0.0).distance(b.with_y(0.0)) >= 0.5, "apart: {a} {b}");
            }
        }
        let world = app.world_mut();
        let (_, membership) = world
            .query::<(&Monster, &GroupMember)>()
            .iter(world)
            .find(|(m, _)| m.species == 12)
            .unwrap();
        let group = world.get::<MonsterGroup>(membership.0).unwrap();
        assert_eq!(group.set.map(|set| set.stance), Some(Stance::Hostile));
    }

    /// Two summonings landing in one run each get their own group and their
    /// own monsters' ids, though neither group is in the queries until the
    /// run ends.
    #[test]
    fn summonings_in_one_run_take_distinct_ids() {
        let (mut app, summoner, _) = scene(false);
        app.world_mut().write_message(Summon { summoner, species: 12 });
        app.world_mut().write_message(Summon { summoner, species: 12 });
        app.world_mut().run_system_once(summon).unwrap();
        let mut locations: Vec<_> = called(&mut app).iter().map(|(m, ..)| m.location).collect();
        locations.sort();
        assert_eq!(locations, [5, 6, 7, 8, 9, 10]);
        let world = app.world_mut();
        let mut groups: Vec<_> = world
            .query::<(&Monster, &GroupMember)>()
            .iter(world)
            .filter(|(m, _)| m.species == 12)
            .map(|(_, membership)| membership.0)
            .collect();
        groups.sort();
        groups.dedup();
        let mut authored: Vec<_> =
            groups.iter().map(|&g| world.get::<MonsterGroup>(g).unwrap().authored).collect();
        authored.sort();
        authored.dedup();
        assert_eq!(authored.len(), 2, "two groups under two ids: {authored:?}");
    }

    /// Two summonings by one unseen summoner in one run scatter from the
    /// same spot, yet the second group stands clear of the first.
    #[test]
    fn summonings_in_one_run_stand_clear_of_each_other() {
        let (mut app, summoner, _) = scene(true);
        app.world_mut().write_message(Summon { summoner, species: 12 });
        app.world_mut().write_message(Summon { summoner, species: 12 });
        app.world_mut().run_system_once(summon).unwrap();
        let called = called(&mut app);
        assert_eq!(called.len(), 6);
        // The first group holds locations 5–7, the second 8–10; each of the
        // second stands a radius plus half a step from each of the first.
        let (first, second): (Vec<_>, Vec<_>) = called.iter().partition(|(m, ..)| m.location < 8);
        assert_eq!((first.len(), second.len()), (3, 3));
        for (_, a, _) in &first {
            for (_, b, _) in &second {
                assert!(a.with_y(0.0).distance(b.with_y(0.0)) >= 1.0, "apart: {a} {b}");
            }
        }
    }

    /// Unseen, the group comes where the summoner stands.
    #[test]
    fn an_unseen_summoner_calls_its_group_up_beside_itself() {
        let (mut app, summoner, _) = scene(true);
        app.world_mut().write_message(Summon { summoner, species: 12 });
        app.world_mut().run_system_once(summon).unwrap();
        let called = called(&mut app);
        assert_eq!(called.len(), 3);
        for (_, at, _) in &called {
            assert!(at.z < -6.0, "by the summoner: {at}");
        }
    }
}
