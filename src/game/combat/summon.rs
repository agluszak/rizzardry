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
        let (caller, caller_at, caller_walker) = monsters.get(summoner).unwrap();
        let kind = &species[id];
        let definition = bodies.get(id, &definitions).expect("bodies load with the level");
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
