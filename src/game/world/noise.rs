//! Noises, and the monsters that come to them (retail `Noise.cpp` and the
//! noise half of `ChooseMonsterRTAIMode`).
use avian3d::prelude::{LinearVelocity, Position, Rotation};
use bevy::prelude::*;
use bevy_landmass::prelude::AgentTarget3d;

use super::bidden::Orders;
use super::facing;
use super::hostility::Stance;
use super::identify::{GroupMembers, MonsterGroup};
use super::population::Monster;
use super::pursuit::Charging;
use super::search::Search;
use crate::game::combat::Combatant;
use crate::game::combat::monster::sight::Sight;
use crate::game::data::Catalogs;
use crate::game::hits::Deed;
use crate::game::hits::definitions::Rows;
use crate::game::party::body::{PartyBody, PartyMovement};
use crate::game::rng::GameRng;

/// Who a noise is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hearers {
    /// Groups that stand against the party.
    Hostile,
    /// Every group that is not fighting.
    All,
}

/// Something made a noise that carries so far.
#[derive(Event, Clone, Copy, Debug)]
pub struct Noise {
    pub at: Vec3,
    pub reach: f32,
    pub to: Hearers,
}

/// On a group: the noise it has heard and has not yet been to.
#[derive(Component, Clone, Copy, Debug)]
pub struct Heard {
    pub at: Vec3,
    pub reach: f32,
    /// How much of the noise's reach was left where it was heard.
    margin: f32,
    /// Whether the way there is short enough to walk, once looked at.
    walks: Option<bool>,
}

/// On a monster making for a noise.
#[derive(Component, Clone, Copy, Debug)]
pub struct Investigating;

/// A group whose leader is not deaf and stands within a noise's reach
/// hears it on a roll of 1 to 100 under its senses and half a point for
/// each metre of reach left; a fainter noise than one it already has in
/// mind is passed over (retail `AlertMonsterGroupsToNoise`).
pub(super) fn hear(
    noise: On<Noise>,
    groups: Query<(Entity, &GroupMembers, Option<&Heard>), With<MonsterGroup>>,
    leaders: Query<(&Monster, &Position, Option<&Orders>, Has<Combatant>)>,
    sight: Sight,
    content: If<Res<Catalogs>>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    for (group, members, heard) in &groups {
        let Some(leader) = members.iter().next() else { continue };
        let Ok((monster, at, orders, fighting)) = leaders.get(leader) else { continue };
        let deaf = orders.is_some_and(|Orders(orders)| orders.deaf);
        let passes = match noise.to {
            Hearers::Hostile => sight.side(leader) != Some(Stance::Hostile),
            Hearers::All => fighting,
        };
        if deaf || passes {
            continue;
        }
        let far = at.0.distance(noise.at);
        let left = noise.reach - far;
        // A fight is heard only by those who could walk to it in two of
        // their moves (retail compares the way there with the move's range
        // in thousands of units).
        let kind = &content.species[monster.species];
        if noise.to == Hearers::All && far > (kind.combat_move_range * 20.0).floor() * 2.0 {
            continue;
        }
        let senses = f32::from(kind.attributes.senses);
        if left <= 0.0
            || rng.below(100) as f32 >= left / 2.0 + senses
            || heard.is_some_and(|heard| left <= heard.margin)
        {
            continue;
        }
        let heard = Heard { at: noise.at, reach: noise.reach, margin: left, walks: None };
        commands.entity(group).insert(heard);
    }
}

/// What a closed door on the way to a noise adds to it (retail's 25000
/// units a hop, `NoiseHearingMargin`).
const DOOR_COST: f32 = 50.0;

/// Whether the straight step from one point to another passes through a box.
fn crosses(from: Vec3, to: Vec3, min: Vec3, max: Vec3) -> bool {
    let way = to - from;
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for axis in 0..3 {
        if way[axis] == 0.0 {
            if from[axis] < min[axis] || from[axis] > max[axis] {
                return false;
            }
            continue;
        }
        let (a, b) = ((min[axis] - from[axis]) / way[axis], (max[axis] - from[axis]) / way[axis]);
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
        if enter > leave {
            return false;
        }
    }
    true
}

/// A monster within this of a noise has been to it (retail's 1500 units).
const AT_THE_NOISE: f32 = 3.0;
/// The farthest a noise is walked to (retail's 125000 units).
const FARTHEST: f32 = 250.0;

/// The members of a group that has heard a noise walk to it if its leader
/// has a way there shorter than the noise's reach, looked for no farther
/// than one and a half times that reach and 250 m (retail
/// `TestNoiseLineOfSight`, `NoiseHearingMargin`); otherwise they turn to it.
/// Once the leader has been to it, it is forgotten. Those fighting or
/// charging do neither.
pub(super) fn investigate(
    mut groups: Query<(Entity, &GroupMembers, Option<&mut Heard>), With<MonsterGroup>>,
    mut walkers: Query<
        (&Position, &mut AgentTarget3d, Has<Investigating>),
        (With<Monster>, Without<Combatant>, Without<Charging>),
    >,
    (navigation, archipelagos): (
        Option<Single<&super::navigation::LevelNavigation>>,
        Query<&bevy_landmass::Archipelago3d>,
    ),
    bodies: Query<&Monster>,
    doors: Query<(&super::triggers::DoorState, &Children)>,
    parts: Query<&avian3d::prelude::ColliderAabb>,
    sight: Sight,
    mut commands: Commands,
) {
    // The bounds of each closed door's prop.
    let closed: Vec<(Vec3, Vec3)> = doors
        .iter()
        .filter(|(door, _)| !door.open)
        .filter_map(|(_, children)| {
            let boxes = children.iter().filter_map(|part| parts.get(part).ok());
            boxes.map(|b| (b.min, b.max)).reduce(|(lo, hi), (min, max)| (lo.min(min), hi.max(max)))
        })
        .collect();
    for (group, members, heard) in &mut groups {
        // Whether the way to it is walked, decided once, by the leader.
        let mut heard = heard;
        if let (Some(heard), Some(leader)) = (heard.as_deref_mut(), members.iter().next())
            && heard.walks.is_none()
        {
            let from = walkers.get(leader).map(|(at, ..)| at.0).unwrap_or(heard.at);
            let archipelago = bodies.get(leader).ok().zip(navigation.as_deref()).and_then(
                |(body, navigation)| {
                    archipelagos.get(navigation.archipelago_for(body.body.radius)).ok()
                },
            );
            let way = archipelago.and_then(|archipelago| {
                super::navigation::walking_path(archipelago, from, heard.at)
            });
            let looked = (heard.reach * 1.5).min(FARTHEST);
            heard.walks = Some(way.is_some_and(|way| {
                let walked: f32 = way.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
                // Each step of the way through a closed door counts 50 m more.
                let through = |pair: &[Vec3]| {
                    closed.iter().any(|&(min, max)| crosses(pair[0], pair[1], min, max))
                };
                let doors = way.windows(2).filter(|pair| through(pair)).count();
                walked <= looked && walked + DOOR_COST * (doors as f32) < heard.reach
            }));
        }
        let heard = heard.as_deref().copied();
        let leader_at = members.iter().next().and_then(|leader| walkers.get(leader).ok());
        let been = heard
            .zip(leader_at)
            .is_some_and(|(heard, (at, ..))| at.0.distance(heard.at) < AT_THE_NOISE);
        if been {
            commands.entity(group).remove::<Heard>();
        }
        let heard = heard.filter(|_| !been);
        let walks = heard.is_some_and(|heard| heard.walks == Some(true));
        for member in members.iter() {
            let Ok((at, mut target, investigating)) = walkers.get_mut(member) else { continue };
            let fit = sight.monster(member).is_some_and(|(body, _)| body.can_fight);
            match heard.filter(|_| fit) {
                Some(heard) if walks => {
                    *target = AgentTarget3d::Point(heard.at);
                    if !investigating {
                        commands.entity(member).insert(Investigating);
                    }
                }
                Some(heard) => {
                    if let Some(turned) = facing(at.0, heard.at) {
                        commands.entity(member).insert(Rotation(turned));
                    }
                }
                None if investigating => {
                    *target = AgentTarget3d::None;
                    commands.entity(member).remove::<Investigating>();
                }
                None => {}
            }
        }
    }
}

/// How far the party walks between the noises its steps make (retail's
/// 2000 units), and how far they carry: 50 m, or 75 m running or searching.
const A_STEP: f32 = 4.0;
const STEPS_CARRY: f32 = 50.0;
const LOUD_STEPS_CARRY: f32 = 75.0;

/// On the party: how far it has walked since its steps were last heard.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Footfalls(f32);

/// The party's steps are a noise every four metres it walks.
pub(super) fn footsteps(
    mut party: Single<
        (Entity, &Position, &LinearVelocity, &PartyMovement, Option<&mut Footfalls>),
        With<PartyBody>,
    >,
    search: Res<Search>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let (body, at, velocity, movement, walked) = &mut *party;
    let Some(walked) = walked else {
        commands.entity(*body).insert(Footfalls::default());
        return;
    };
    walked.0 += velocity.0.with_y(0.0).length() * time.delta_secs();
    if walked.0 > A_STEP {
        walked.0 = 0.0;
        let loud = movement.running || search.on(false);
        let reach = if loud { LOUD_STEPS_CARRY } else { STEPS_CARRY };
        commands.trigger(Noise { at: at.0, reach, to: Hearers::Hostile });
    }
}

/// How far a round of fighting is heard (retail's 50000 units).
const A_FIGHT_CARRIES: f32 = 100.0;

/// A round of fighting is a noise where the party stands, for every group
/// not in the fight (retail `AlertWorldNoise`).
pub(super) fn fights_are_heard(
    _: On<crate::game::combat::RoundBegan>,
    party: Single<&Position, With<PartyBody>>,
    mut commands: Commands,
) {
    commands.trigger(Noise { at: party.0, reach: A_FIGHT_CARRIES, to: Hearers::All });
}

/// How far a spell's noise carries.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
struct Loudness {
    carries: f32,
    a_power: f32,
}

/// The spells that are heard, by their numbers.
static LOUD: std::sync::LazyLock<std::collections::HashMap<u8, Loudness>> =
    std::sync::LazyLock::new(|| {
        ron::from_str(include_str!("../../../assets/tables/noises.ron"))
            .expect("assets/tables/noises.ron is a table of noises")
    });

/// A spell that is heard makes its noise where it is aimed, as it is
/// cast.
pub(super) fn spells_are_heard(
    made: On<Add<Deed>>,
    deeds: Query<&Deed>,
    positions: Query<&Position>,
    mut commands: Commands,
) {
    let Ok(deed) = deeds.get(made.entity) else { return };
    let Rows::Spell(spell) = deed.strike.rows else { return };
    let Some(loud) = LOUD.get(&spell.0) else { return };
    let aimed = deed.aimed_at().find_map(|victim| positions.get(victim).ok());
    if let Some(at) = aimed {
        let reach = loud.carries + loud.a_power * deed.strike.strength.power as f32;
        commands.trigger(Noise { at: at.0, reach, to: Hearers::Hostile });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sonic Boom carries 50 m and 30 m more a point of power (retail's
    /// 25000 units and 15000 a point).
    #[test]
    fn sonic_boom_is_heard_farther_at_higher_power() {
        let boom = LOUD[&30];
        assert_eq!(boom.carries + boom.a_power * 7.0, 260.0);
    }
}
