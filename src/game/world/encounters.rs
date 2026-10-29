//! Random encounters (retail `MonGen.cpp`): a level's generators call up
//! groups from their tables now and then, out of the party's sight, within
//! a budget that grows with time away and shrinks as the party kills them;
//! those left alone long enough, and far enough off, are retired.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::identify::{GroupMembers, MonsterGroup};
use super::level::{KeptMonster, Level, PlayedLevel};
use super::navigation::{LevelNavigation, Walker, WalkerMotionConfig, feet};
use super::obstacles::WorldObstacles;
use super::population::{self, Monster, MonsterBodies, Unborn};
use super::{ActorPose, STEP_HEIGHT};
use crate::content::items::Dice;
use crate::content::level::{
    EncounterLimits, EncounterTable, EncounterTime, Encounters, Generator,
};
use crate::content::monster::MonsterBody;
use crate::content::species::{MonsterCatalog, Species};
use crate::game::difficulty::Difficulty;
use crate::game::party::body::PartyBody;
use crate::game::rng::GameRng;
use crate::game::time::{DAY_MS, GameClock};

/// Past ten hours away the encounters about are rolled afresh rather than
/// aged (retail `W8_ENCOUNTER_STALE_SECONDS`).
const STALE_SECONDS: u64 = 36_000;
/// A generator only calls up an encounter between these distances from the
/// party (retail 35000 and 200000 units).
const NEAREST: f32 = 70.0;
const FARTHEST: f32 = 400.0;
/// Nor while another monster stands within this of the spot, each way
/// (retail's 5000-unit box).
const OCCUPIED: f32 = 10.0;
/// An encounter is retired only this far from the party (retail 50000
/// units).
const RETIRED_BEYOND: f32 = 100.0;
/// Retail scales an encounter's lingering by its default sight factor.
const LINGER_SCALE: f32 = 12.0;
/// How far below a spot the ground may lie (retail's 2500 units).
const GROUND_BELOW: f32 = 5.0;

/// On a level root: its random encounters as they run (retail's encounter
/// globals and each `MonGen`'s own state).
#[derive(Component, Clone, Debug, Default)]
pub struct EncounterState {
    pub budget: i32,
    /// How many may be about at once now.
    pub limit: i32,
    pub generators: Vec<GeneratorState>,
    /// The game second the party left, kept while the level is away; or
    /// the arrival still to settle (`None` there means a first visit).
    pub arrival: Option<Arrival>,
}

/// A level's arrival, settled once its ground is in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    First,
    Returned { left: u64 },
}

/// A generator's own state: whether it runs, and its timer, in gameplay
/// seconds (retail `MonGen::m_pTimer`, which runs on the pausable game
/// timer).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeneratorState {
    pub enabled: bool,
    pub elapsed: f32,
    pub duration: f32,
}

impl EncounterState {
    /// A level's encounters on its first visit: as authored, each timer
    /// partway through its interval.
    pub fn authored(encounters: &Encounters, rng: &mut GameRng) -> Self {
        let generators = encounters
            .generators
            .iter()
            .map(|generator| {
                let duration = jittered(interval(encounters, generator), rng);
                GeneratorState {
                    enabled: generator.enabled,
                    elapsed: generator.progress * duration,
                    duration,
                }
            })
            .collect();
        let mut state = EncounterState {
            budget: encounters.budget,
            limit: 0,
            generators,
            arrival: Some(Arrival::First),
        };
        state.clamp(&encounters.limits);
        state
    }

    /// Keep the budget, and the limit it sets, within the level's bounds
    /// (retail `ClampInteger` twice).
    fn clamp(&mut self, limits: &EncounterLimits) {
        self.budget = clamp(self.budget, limits.smallest_budget, limits.largest_budget);
        self.limit = clamp(self.budget, limits.fewest, limits.most);
    }
}

/// Retail `ClampInteger`: the floor first, then the ceiling.
fn clamp(value: i32, low: i32, high: i32) -> i32 {
    let value = value.max(low);
    value.min(high)
}

/// A generator's interval in seconds.
fn interval(encounters: &Encounters, generator: &Generator) -> f32 {
    f32::from(if generator.uses_defaults {
        encounters.default_interval.max(0) as u16
    } else {
        generator.interval
    })
}

/// An interval jittered by up to a fifth either way (retail `MonGen::Reset`).
fn jittered(interval: f32, rng: &mut GameRng) -> f32 {
    let jitter = interval * 0.2;
    rng.below(jitter as u32 * 2 + 1) as f32 + interval - jitter
}

/// Whether it is night now: before 5:00 or after 22:00 (retail
/// `SelectEncounterCandidates`).
fn night(clock: &GameClock) -> bool {
    let ms = clock.ms % DAY_MS;
    !(ms > 18_000_000 && ms <= 79_200_000)
}

/// Retail's rarity classes.
fn rarity_class(rarity: f32) -> u8 {
    match rarity {
        r if r <= 3.0 => 3,
        r if r <= 10.0 => 7,
        r if r <= 30.0 => 20,
        _ => 70,
    }
}

/// Which of a table's entries suit now, the chosen one first (retail
/// `SelectEncounterCandidates`): a rarity class is drawn; entries of that
/// class, within half the party's level of a challenge and scaled less
/// common the further off, at a fitting time of day; failing any, any
/// entry of the class at a fitting time.
fn candidates(
    table: &EncounterTable,
    party_level: f32,
    night: bool,
    rng: &mut GameRng,
) -> Vec<usize> {
    let drawn = match rng.below(100) {
        0..=3 => 3,
        4..=10 => 7,
        11..=30 => 20,
        _ => 70,
    };
    let party_level = (party_level + 0.49).floor();
    let timely = |time: EncounterTime| match time {
        EncounterTime::Always => true,
        EncounterTime::Night => night,
        EncounterTime::Day => !night,
    };
    let mut candidates: Vec<usize> = table
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            let off = (1.0 - f32::from(entry.challenge) / party_level).abs();
            off < 0.5
                && rarity_class(f32::from(entry.rarity) * (1.0 - off * 1.8)) == drawn
                && timely(entry.time)
        })
        .map(|(i, _)| i)
        .collect();
    if candidates.is_empty() {
        candidates = table
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.rarity == drawn && timely(entry.time))
            .map(|(i, _)| i)
            .collect();
    }
    if candidates.len() > 1 {
        let chosen = candidates.remove(rng.below(candidates.len() as u32) as usize);
        candidates.insert(0, chosen);
    }
    candidates
}

/// How many come, the group dice leaned on by how strong they are for the
/// party and the difficulty (retail `RollEncounterGroupSize`): strong ones
/// toward the fewer half on Normal (fewest on Novice, as rolled on
/// Expert); others, when the dice's middle is under one, toward the more
/// half on Normal (most on Expert, as rolled on Novice).
fn group_size(kind: &Species, party_level: f32, difficulty: Difficulty, rng: &mut GameRng) -> u32 {
    let dice: Dice = kind.group_size;
    let fewest = (i32::from(dice.base) + i32::from(dice.count)).max(0) as u32;
    let most = (i32::from(dice.base) + i32::from(dice.count) * i32::from(dice.sides)).max(0) as u32;
    let middle = (fewest + most) as f32 * 0.5;
    let mut rolled = rng.roll(dice);
    if f32::from(kind.display_level) / party_level >= 1.2 {
        match difficulty {
            Difficulty::Novice => return fewest,
            Difficulty::Expert => return rolled,
            Difficulty::Normal => {}
        }
        while rolled as f32 > middle {
            rolled = rng.roll(dice);
        }
        return rolled;
    }
    // Retail really compares the middle here, not the strength.
    if middle > 0.8 {
        return rolled;
    }
    match difficulty {
        Difficulty::Expert => return most,
        Difficulty::Novice => return rolled,
        Difficulty::Normal => {}
    }
    while (rolled as f32) < middle {
        rolled = rng.roll(dice);
    }
    rolled
}

/// The party as random encounters see it.
struct Onlooker {
    eye: Vec3,
    feet: Vec3,
    /// Retail's floating average level of the party.
    level: f32,
}

/// What generation needs of the world.
struct Spawning<'a, 'w, 's> {
    root: Entity,
    species: &'a MonsterCatalog,
    bodies: (&'a MonsterBodies, &'a Assets<MonsterBody>),
    navigation: &'a LevelNavigation,
    obstacles: &'a WorldObstacles<'w, 's>,
    /// The next free location and group ids.
    next: (i32, i32),
}

impl Spawning<'_, '_, '_> {
    /// A group of `count` of a species at a spot (retail `CreateGroup` at
    /// a generator: placed over the ground about it, facing the party,
    /// without a clearance check; members the ground will not hold share
    /// the last spot found, and with none they stand on the spot itself).
    fn group(
        &mut self,
        commands: &mut Commands,
        (id, count): (u32, u32),
        (at, toward): (Vec3, Vec3),
        (configs, rng): (&mut Assets<WalkerMotionConfig>, &mut GameRng),
        group: MonsterGroup,
        unborn: &mut Unborn,
    ) -> Option<Vec<Vec3>> {
        let kind = self.species.kind(id)?;
        let Some(definition) = self.bodies.0.0.get(&id).and_then(|body| self.bodies.1.get(body))
        else {
            warn!("encounter species {id} has no body ready");
            return None;
        };
        let count = count.min(9) as usize;
        if count == 0 {
            return None;
        }
        let (location, authored) = self.next;
        self.next = (location + count as i32, authored + 1);
        let mut members: Vec<KeptMonster> = (0..count)
            .map(|i| {
                let identity = (location + i as i32, id, Some(authored));
                KeptMonster::rolled(identity, (at, 0.0), (kind, definition), rng)
            })
            .collect();
        let toward = toward - at;
        let yaw = (-toward.x).atan2(-toward.z);
        let obstacles = self.obstacles;
        let spots = population::scatter(
            (at, yaw, 2.0 * members[0].monster.body.radius),
            count,
            rng,
            |spot| {
                let start = spot + Vec3::Y * STEP_HEIGHT;
                let hit = obstacles.first_hit(start, Dir3::NEG_Y, STEP_HEIGHT + GROUND_BELOW)?;
                Some(start - Vec3::Y * hit.distance)
            },
            |_| true,
            |_, _| true,
        );
        for (i, member) in members.iter_mut().enumerate() {
            member.feet = spots.get(i).or(spots.last()).copied().unwrap_or(at);
            member.yaw = yaw;
        }
        let group = MonsterGroup { authored, ..group };
        if kind.unborn {
            unborn.0.extend(members);
            return None;
        }
        let feet = members.iter().map(|member| member.feet).collect();
        let (group_entity, _) = population::spawn_group(
            commands,
            self.root,
            members,
            (authored, None),
            self.species,
            (self.navigation, configs),
        );
        commands.entity(group_entity).insert(group);
        Some(feet)
    }
}

/// One encounter from a generator (retail `MonGen::GenerateEncounter`): an
/// entry suiting now from its table, its companions at their chances, as
/// many as the encounter's size, registered as the level's; its companions
/// come along under it. A harassing generator never generates here: the
/// possession facts it waits on are not kept yet.
fn generate(
    commands: &mut Commands,
    generator: &Generator,
    (party, now_s, night, difficulty): (&Onlooker, u64, bool, Difficulty),
    spawning: &mut Spawning,
    (configs, rng): (&mut Assets<WalkerMotionConfig>, &mut GameRng),
    unborn: &mut Unborn,
) -> Option<(i32, Vec<Vec3>)> {
    let table = generator.table.as_ref().filter(|_| !generator.disabled)?;
    let candidates = candidates(table, party.level, night, rng);
    let &chosen = candidates.first()?;
    let id = table.entries[chosen].species;
    let kind = spawning.species.kind(id)?;
    let companions: Vec<u32> = kind
        .companions
        .iter()
        .flatten()
        .filter(|&&(_, chance)| rng.below(100) < u32::from(chance))
        .map(|&(species, _)| species)
        .collect();
    if generator.harasses {
        return None;
    }
    let count = group_size(kind, party.level, difficulty, rng);
    let main = MonsterGroup { encounter: Some(now_s), ..default() };
    let mut feet = spawning.group(
        commands,
        (id, count),
        (generator.position, party.feet),
        (configs, rng),
        main,
        unborn,
    )?;
    let leader = spawning.next.1 - 1;
    for companion in companions {
        let Some(companion_kind) = spawning.species.kind(companion) else { break };
        let count = rng.roll(companion_kind.group_size);
        let escort = MonsterGroup { escort_of: Some(leader), ..default() };
        feet.extend(
            spawning
                .group(
                    commands,
                    (companion, count),
                    (generator.position, party.feet),
                    (configs, rng),
                    escort,
                    unborn,
                )
                .unwrap_or_default(),
        );
    }
    Some((leader, feet))
}

/// The encounters about and who stands where, as this run has left them:
/// a spawn or retirement it accepts counts for the next generator at once,
/// before its commands apply (retail registers each synchronously).
struct Working {
    /// Each encounter about: its group id, when it came and where its
    /// leader stands.
    active: Vec<(i32, u64, Option<Vec3>)>,
    /// Living monsters' feet, with the encounter each belongs to.
    standing: Vec<(Option<i32>, Vec3)>,
}

impl Working {
    /// Take an encounter off the level: despawn it and its companions, and
    /// forget them here.
    fn retire(
        &mut self,
        commands: &mut Commands,
        groups: &Query<(Entity, &mut MonsterGroup, Option<&GroupMembers>)>,
        leader: i32,
    ) {
        for (entity, group, members) in groups.iter() {
            if group.authored == leader || group.escort_of == Some(leader) {
                for member in members.into_iter().flat_map(|m| m.iter()) {
                    commands.entity(member).despawn();
                }
                commands.entity(entity).despawn();
            }
        }
        self.active.retain(|&(l, ..)| l != leader);
        self.standing.retain(|&(l, _)| l != Some(leader));
    }

    /// A new encounter and its companions, standing where they were put.
    fn add(&mut self, (leader, feet): (i32, Vec<Vec3>), now_s: u64) {
        self.active.push((leader, now_s, feet.first().copied()));
        self.standing.extend(feet.into_iter().map(|at| (Some(leader), at)));
    }
}

/// Whether a generator may call up an encounter now (retail
/// `MonGen::CanGenerateEncounter`), in retail's order: nothing holds
/// generation off (a fight, a conversation) and it is enabled; unless
/// forced, its spot is 70 to 400 m from the party and out of its sight;
/// fewer encounters are about than the limit, retiring overdue ones far
/// from the party first when it is reached; and unless forced, no monster
/// stands near the spot and its chance comes up.
fn admits(
    (generator, state): (&Generator, &GeneratorState),
    encounters: &Encounters,
    party: &Onlooker,
    (working, limit, now_s): (&mut Working, i32, u64),
    (commands, groups): (&mut Commands, &Query<(Entity, &mut MonsterGroup, Option<&GroupMembers>)>),
    obstacles: &WorldObstacles,
    (force, blocked, rng): (bool, bool, &mut GameRng),
) -> bool {
    if blocked || encounters.held || !state.enabled {
        return false;
    }
    let spot = generator.position;
    let distance = party.eye.distance(spot);
    if !force {
        if !(NEAREST..=FARTHEST).contains(&distance) {
            return false;
        }
        // Looking a step above the spot, so the ground it lies on does not
        // hide it.
        let above = spot + Vec3::Y * STEP_HEIGHT;
        let seen = Dir3::new(above - party.eye).is_ok_and(|direction| {
            obstacles.first_hit(party.eye, direction, party.eye.distance(above)).is_none()
        });
        if seen {
            return false;
        }
    }
    if working.active.len() as i32 >= limit {
        // Retire the overdue far from the party (retail
        // `CullExpiredEncounters`; the age compares unsigned).
        let lingers = encounters.limits.lingers as f32 * LINGER_SCALE;
        let overdue: Vec<i32> = working
            .active
            .iter()
            .filter(|&&(_, spawned, at)| {
                (now_s.wrapping_sub(spawned) as u32) as f32 > lingers
                    && at.is_none_or(|at| at.distance(party.feet) > RETIRED_BEYOND)
            })
            .map(|&(leader, ..)| leader)
            .collect();
        for leader in overdue {
            working.retire(commands, groups, leader);
        }
        if working.active.len() as i32 >= limit {
            return false;
        }
    }
    if force {
        return true;
    }
    if working.standing.iter().any(|(_, at)| (*at - spot).abs().max_element() <= OCCUPIED) {
        return false;
    }
    let chance = if generator.uses_defaults {
        i32::from(encounters.default_chance)
    } else {
        i32::from(generator.chance)
    };
    (rng.below(100) as i32) < chance
}

/// Run the played level's random encounters on the fixed step, once
/// physics has placed its ground: settle an arrival's budget (rolling
/// every generator afresh on a first visit or after long away, otherwise
/// ageing the encounters about by the time away); charge the budget for
/// encounters the party wipes out in a fight; retire overdue ones far from
/// the party; and let each generator whose timer runs out try to call one
/// up.
#[allow(clippy::too_many_arguments)]
pub fn run_encounters(
    mut commands: Commands,
    mut levels: Query<
        (Entity, &mut EncounterState, &LevelNavigation, &mut Unborn, &MonsterBodies),
        With<Level>,
    >,
    mut groups: Query<(Entity, &mut MonsterGroup, Option<&GroupMembers>)>,
    monsters: Query<(&Monster, ActorPose, Option<&Walker>, Option<&super::identify::GroupMember>)>,
    party: Query<(&Position, Option<&crate::game::party::roster::PartyRoster>), With<PartyBody>>,
    characters: Query<&crate::game::characters::Character>,
    blocked: (
        Option<Single<(), With<crate::game::combat::Battle>>>,
        Query<(), With<crate::game::npcs::conversation::Conversation>>,
    ),
    (level, obstacles): (PlayedLevel, WorldObstacles),
    (definitions, mut configs): (Res<Assets<MonsterBody>>, ResMut<Assets<WalkerMotionConfig>>),
    (time, clock, difficulty, mut rng): (
        Res<Time<Fixed>>,
        Res<GameClock>,
        Option<Res<Difficulty>>,
        ResMut<GameRng>,
    ),
    content: crate::game::data::Content,
) {
    let (Some(definition), Some(species)) = (level.definition(), content.species()) else { return };
    let encounters = &definition.encounters;
    if encounters.generators.is_empty() {
        return;
    }
    let Ok((root, mut state, navigation, mut unborn, bodies)) = levels.single_mut() else { return };
    let Ok((party_at, roster)) = party.single() else { return };
    let (in_battle, talking) = blocked;
    let in_battle = in_battle.is_some();
    let blocked = in_battle || !talking.is_empty();
    let difficulty = difficulty.map_or_else(Difficulty::default, |d| *d);
    let now_s = clock.ms / 1000;
    let levels: Vec<f32> = roster
        .into_iter()
        .flat_map(|roster| roster.iter())
        .filter_map(|m| characters.get(m).ok())
        .map(|c| f32::from(c.level))
        .take(6)
        .collect();
    let onlooker = Onlooker {
        eye: PartyBody::eye(party_at),
        feet: party_at.0 - Vec3::Y * crate::game::party::body::FLOAT_HEIGHT,
        level: if levels.is_empty() {
            1.0
        } else {
            levels.iter().sum::<f32>() / levels.len() as f32
        },
    };
    let night = night(&clock);
    // The next free ids, the unborn's counted too.
    let next_location = monsters
        .iter()
        .map(|(m, ..)| m.location)
        .chain(unborn.0.iter().map(|k| k.monster.location))
        .max()
        .unwrap_or(0)
        + 1;
    let next_group = groups
        .iter()
        .map(|(_, g, _)| g.authored)
        .chain(unborn.0.iter().filter_map(|k| k.group))
        .max()
        .unwrap_or(0)
        + 1;
    let mut spawning = Spawning {
        root,
        species,
        bodies: (bodies, &definitions),
        navigation,
        obstacles: &obstacles,
        next: (next_location, next_group),
    };
    let alive = |members: Option<&GroupMembers>| {
        members
            .into_iter()
            .flat_map(|m| m.iter())
            .filter(|&m| monsters.get(m).is_ok_and(|(b, ..)| !b.is_dead()))
            .count()
    };
    // Wiped out: off the books, and in a fight it costs the budget.
    for (_, mut group, members) in &mut groups {
        if group.encounter.is_some() && alive(members) == 0 {
            group.encounter = None;
            if in_battle {
                state.budget -= 1;
                state.clamp(&encounters.limits);
            }
        }
    }
    let mut working = Working {
        active: groups
            .iter()
            .filter_map(|(_, group, members)| {
                let spawned = group.encounter?;
                let at = members.into_iter().flat_map(|m| m.iter()).find_map(|m| {
                    let (_, pose, walker, _) = monsters.get(m).ok()?;
                    Some(feet(pose.translation(), walker))
                });
                Some((group.authored, spawned, at))
            })
            .collect(),
        standing: monsters
            .iter()
            .filter(|(m, ..)| !m.is_dead())
            .map(|(_, pose, walker, member)| {
                let group = member.and_then(|member| groups.get(member.0).ok());
                let leader =
                    group.and_then(|(_, g, _)| g.encounter.map(|_| g.authored).or(g.escort_of));
                (leader, feet(pose.translation(), walker))
            })
            .collect(),
    };
    if let Some(arrival) = state.arrival.take() {
        let elapsed = match arrival {
            Arrival::First => {
                state.budget = encounters.limits.largest_budget;
                STALE_SECONDS + 1
            }
            Arrival::Returned { left } => {
                let elapsed = now_s.saturating_sub(left);
                if encounters.limits.period > 0 {
                    state.budget += (elapsed / encounters.limits.period as u64) as i32;
                }
                elapsed
            }
        };
        state.clamp(&encounters.limits);
        if elapsed > STALE_SECONDS {
            // Every encounter about goes, and every generator, in a
            // shuffled order, gets a forced try (retail
            // `RollRandomEncounters`).
            let leaders: Vec<i32> = working.active.iter().map(|&(leader, ..)| leader).collect();
            for leader in leaders {
                working.retire(&mut commands, &groups, leader);
            }
            let mut order: Vec<usize> = (0..encounters.generators.len()).collect();
            let count = order.len() as u32;
            if count > 1 {
                for _ in 0..count {
                    let (first, second) = (rng.below(count) as usize, rng.below(count) as usize);
                    order.swap(first, second);
                }
            }
            for index in order {
                let generator = &encounters.generators[index];
                if admits(
                    (generator, &state.generators[index]),
                    encounters,
                    &onlooker,
                    (&mut working, state.limit, now_s),
                    (&mut commands, &groups),
                    &obstacles,
                    (true, blocked, &mut rng),
                ) && let Some(spawned) = generate(
                    &mut commands,
                    generator,
                    (&onlooker, now_s, night, difficulty),
                    &mut spawning,
                    (&mut configs, &mut rng),
                    &mut unborn,
                ) {
                    working.add(spawned, now_s);
                }
            }
            return;
        }
        for (_, mut group, _) in &mut groups {
            if let Some(spawned) = group.encounter.as_mut() {
                *spawned += elapsed;
            }
        }
        for (_, spawned, _) in &mut working.active {
            *spawned += elapsed;
        }
    }
    let delta = time.delta_secs();
    for index in 0..encounters.generators.len() {
        let generator = &encounters.generators[index];
        state.generators[index].elapsed += delta;
        if state.generators[index].elapsed < state.generators[index].duration {
            continue;
        }
        if admits(
            (generator, &state.generators[index]),
            encounters,
            &onlooker,
            (&mut working, state.limit, now_s),
            (&mut commands, &groups),
            &obstacles,
            (false, blocked, &mut rng),
        ) && let Some(spawned) = generate(
            &mut commands,
            generator,
            (&onlooker, now_s, night, difficulty),
            &mut spawning,
            (&mut configs, &mut rng),
            &mut unborn,
        ) {
            working.add(spawned, now_s);
        }
        let duration = jittered(interval(encounters, generator), &mut rng);
        state.generators[index] =
            GeneratorState { elapsed: 0.0, duration, ..state.generators[index] };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::level::{EncounterEntry, LevelDefinition};
    use crate::game::world::identify::GroupMember;
    use crate::game::world::obstacles::scenery;
    use avian3d::prelude::{Collider, PhysicsPlugins, RigidBody};
    use bevy::ecs::system::RunSystemOnce;

    const SPECIES: u32 = 1;
    /// The game second a new clock reads.
    const NOON: u64 = 43_200;

    /// A level on a floor with one generator 150 m off behind a wall, its
    /// table offering one species at every rarity, the party at the
    /// origin; returns the app and the level.
    fn level(budget: (i32, i32)) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::default(),
        ))
        .init_resource::<GameClock>()
        .insert_resource(GameRng::seeded(9))
        .init_asset::<MonsterBody>()
        .init_asset::<MonsterCatalog>()
        .init_asset::<LevelDefinition>()
        .init_resource::<Assets<WalkerMotionConfig>>();
        app.finish();
        app.cleanup();
        let world = app.world_mut();
        let crab = Species { group_size: Dice { base: 2, count: 0, sides: 0 }, ..default() };
        let species = world
            .resource_mut::<Assets<MonsterCatalog>>()
            .add(MonsterCatalog(vec![default(), crab]));
        world.insert_resource(crate::game::data::GameData {
            items: default(),
            loot: default(),
            species,
            spells: default(),
        });
        let body = world.resource_mut::<Assets<MonsterBody>>().add(MonsterBody {
            scale: 1.0,
            fight_radius: 0.5,
            target_height: 1.5,
            ..default()
        });
        let level_bodies = MonsterBodies([(SPECIES, body)].into());
        let table = EncounterTable {
            name: "Woods".into(),
            entries: [3, 7, 20, 70]
                .map(|rarity| EncounterEntry {
                    species: SPECIES,
                    rarity,
                    time: EncounterTime::Always,
                    challenge: 1,
                })
                .to_vec(),
        };
        let encounters = Encounters {
            limits: EncounterLimits {
                most: 5,
                fewest: 1,
                largest_budget: budget.0,
                smallest_budget: budget.1,
                period: 600,
                lingers: 10,
            },
            budget: 3,
            held: false,
            default_chance: 100,
            default_interval: 10,
            generators: vec![Generator {
                name: "MonGen001".into(),
                enabled: true,
                chance: 100,
                interval: 1,
                position: Vec3::new(0.0, 0.0, -150.0),
                table: Some(table),
                ..default()
            }],
        };
        let definition = world
            .resource_mut::<Assets<LevelDefinition>>()
            .add(LevelDefinition { encounters, ..default() });
        let root = world
            .spawn((
                Level::test(definition),
                level_bodies,
                LevelNavigation { archipelagos: vec![] },
                Unborn::default(),
            ))
            .id();
        let party = crate::game::party::roster::test_party(world, &[]);
        world.entity_mut(party).insert((
            Position(Vec3::Y * crate::game::party::body::FLOAT_HEIGHT),
            PartyBody::default(),
        ));
        world.spawn((
            RigidBody::Static,
            Collider::cuboid(400.0, 0.2, 400.0),
            scenery(),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        world.spawn((
            RigidBody::Static,
            Collider::cuboid(40.0, 20.0, 0.2),
            scenery(),
            Transform::from_xyz(0.0, 0.0, -100.0),
        ));
        app.update();
        app.update();
        (app, root)
    }

    fn arrive(app: &mut App, root: Entity, arrival: Arrival) {
        let definition = PlayedLevelDefinition::of(app, root);
        let mut state = EncounterState::authored(&definition.encounters, &mut GameRng::seeded(1));
        state.arrival = Some(arrival);
        app.world_mut().entity_mut(root).insert(state);
    }

    struct PlayedLevelDefinition;
    impl PlayedLevelDefinition {
        fn of(app: &App, root: Entity) -> LevelDefinition {
            let handle = &app.world().get::<Level>(root).unwrap().definition;
            app.world().resource::<Assets<LevelDefinition>>().get(handle).unwrap().clone()
        }
    }

    /// The encounter groups about: each group's id, its registration and
    /// its members' feet.
    fn encounters(app: &mut App) -> Vec<(MonsterGroup, Vec<Vec3>)> {
        let world = app.world_mut();
        let members: Vec<(Entity, Vec3)> = world
            .query::<(&GroupMember, &Transform)>()
            .iter(world)
            .map(|(member, at)| (member.0, at.translation))
            .collect();
        world
            .query::<(Entity, &MonsterGroup)>()
            .iter(world)
            .filter(|(_, group)| group.encounter.is_some())
            .map(|(entity, group)| {
                (*group, members.iter().filter(|(g, _)| *g == entity).map(|&(_, at)| at).collect())
            })
            .collect()
    }

    /// A first visit fills the budget and rolls every generator, out of
    /// sight or not, so the level starts with its encounters about.
    #[test]
    fn a_first_visit_rolls_every_generator() {
        let (mut app, root) = level((8, 2));
        arrive(&mut app, root, Arrival::First);
        app.world_mut().run_system_once(run_encounters).unwrap();
        let state = app.world().get::<EncounterState>(root).unwrap();
        assert_eq!((state.budget, state.limit), (8, 5), "the budget fills, the limit follows");
        assert!(state.arrival.is_none());
        let about = encounters(&mut app);
        assert_eq!(about.len(), 1);
        let (group, feet) = &about[0];
        assert_eq!(group.encounter, Some(NOON), "registered as it came");
        assert_eq!(feet.len(), 2, "as many as the group dice say");
        assert!(feet.iter().all(|at| at.distance(Vec3::new(0.0, 0.0, -150.0)) < 5.0), "{feet:?}");
    }

    /// Back after a while, the budget grows by the time away and the
    /// encounters about age by none of it; no new roll happens.
    #[test]
    fn a_return_earns_budget_and_keeps_the_encounters() {
        let (mut app, root) = level((8, 2));
        arrive(&mut app, root, Arrival::First);
        app.world_mut().run_system_once(run_encounters).unwrap();
        app.world_mut().resource_mut::<GameClock>().ms = (NOON + 1_200) * 1000;
        {
            let mut state = app.world_mut().get_mut::<EncounterState>(root).unwrap();
            state.budget = 2;
            state.arrival = Some(Arrival::Returned { left: NOON });
            state.generators[0].elapsed = 0.0;
        }
        app.world_mut().run_system_once(run_encounters).unwrap();
        let state = app.world().get::<EncounterState>(root).unwrap();
        assert_eq!(state.budget, 4, "1200 s away earns two at 600 s each");
        let about = encounters(&mut app);
        assert_eq!(about.len(), 1, "no new roll");
        assert_eq!(about[0].0.encounter, Some(NOON + 1_200), "aged by none of the time away");
    }

    /// A generator due, its spot out of the party's sight and far enough,
    /// calls an encounter up; one in the party's sight does not.
    #[test]
    fn a_due_generator_calls_up_only_out_of_sight() {
        for (wall, called) in [(true, 1), (false, 0)] {
            let (mut app, root) = level((8, 2));
            if !wall {
                let world = app.world_mut();
                let walls: Vec<Entity> = world
                    .query::<(Entity, &Transform, &Collider)>()
                    .iter(world)
                    .filter(|(_, at, _)| at.translation.z < -50.0)
                    .map(|(e, ..)| e)
                    .collect();
                for wall in walls {
                    world.despawn(wall);
                }
                app.update();
                app.update();
            }
            arrive(&mut app, root, Arrival::Returned { left: NOON });
            {
                let mut state = app.world_mut().get_mut::<EncounterState>(root).unwrap();
                state.generators[0].elapsed = state.generators[0].duration;
            }
            app.world_mut().run_system_once(run_encounters).unwrap();
            assert_eq!(encounters(&mut app).len(), called, "wall {wall}");
            let state = app.world().get::<EncounterState>(root).unwrap();
            assert_eq!(state.generators[0].elapsed, 0.0, "the timer starts again");
        }
    }

    /// An encounter the party wipes out in a fight is off the books and
    /// costs the budget one (retail `UnregisterActiveEncounterGroup`).
    #[test]
    fn an_encounter_wiped_out_in_a_fight_costs_the_budget() {
        let (mut app, root) = level((8, 2));
        arrive(&mut app, root, Arrival::First);
        app.world_mut().run_system_once(run_encounters).unwrap();
        let world = app.world_mut();
        for mut monster in world.query::<&mut Monster>().iter_mut(world) {
            monster.hp = 0;
        }
        world.spawn(crate::game::combat::Battle::default());
        app.world_mut().run_system_once(run_encounters).unwrap();
        let state = app.world().get::<EncounterState>(root).unwrap();
        assert_eq!((state.budget, state.limit), (7, 5));
        assert!(encounters(&mut app).is_empty());
    }

    /// Change the level's authored encounters.
    fn author(app: &mut App, root: Entity, change: impl FnOnce(&mut Encounters)) {
        let handle = app.world().get::<Level>(root).unwrap().definition.clone();
        let mut definitions = app.world_mut().resource_mut::<Assets<LevelDefinition>>();
        change(&mut definitions.get_mut(&handle).unwrap().encounters);
    }

    /// Make every generator due.
    fn due(app: &mut App, root: Entity) {
        let mut state = app.world_mut().get_mut::<EncounterState>(root).unwrap();
        for generator in &mut state.generators {
            generator.elapsed = generator.duration;
        }
    }

    /// Two generators due at once with room for one encounter: the first
    /// one's spawn counts against the second at once.
    #[test]
    fn generators_due_together_respect_the_limit() {
        let (mut app, root) = level((8, 2));
        author(&mut app, root, |encounters| {
            encounters.limits.most = 1;
            let mut second = encounters.generators[0].clone();
            second.position = Vec3::new(15.0, 0.0, -150.0);
            encounters.generators.push(second);
        });
        arrive(&mut app, root, Arrival::Returned { left: NOON });
        due(&mut app, root);
        app.world_mut().run_system_once(run_encounters).unwrap();
        assert_eq!(encounters(&mut app).len(), 1);
    }

    /// At the limit, an overdue encounter far off is retired only when a
    /// generator is otherwise free to generate: not during a fight, and
    /// then the generator replaces it in the same step.
    #[test]
    fn retirement_waits_on_the_generation_gates() {
        let (mut app, root) = level((8, 2));
        author(&mut app, root, |encounters| encounters.limits.most = 1);
        arrive(&mut app, root, Arrival::First);
        app.world_mut().run_system_once(run_encounters).unwrap();
        let first = encounters(&mut app)[0].0.authored;
        // Long past its lingering (10 × 12 s).
        app.world_mut().resource_mut::<GameClock>().ms = (NOON + 600) * 1000;
        let battle = app.world_mut().spawn(crate::game::combat::Battle::default()).id();
        due(&mut app, root);
        app.world_mut().run_system_once(run_encounters).unwrap();
        let about = encounters(&mut app);
        assert_eq!(about.len(), 1);
        assert_eq!(about[0].0.authored, first, "a fight retires nothing");
        app.world_mut().despawn(battle);
        due(&mut app, root);
        app.world_mut().run_system_once(run_encounters).unwrap();
        let about = encounters(&mut app);
        assert_eq!(about.len(), 1);
        assert_ne!(about[0].0.authored, first, "retired and replaced");
        assert_eq!(about[0].0.encounter, Some(NOON + 600));
    }

    /// Strong encounters lean small and weak tiny ones large, by difficulty
    /// (retail `RollEncounterGroupSize`).
    #[test]
    fn group_sizes_lean_by_strength_and_difficulty() {
        let mut rng = GameRng::seeded(4);
        let strong = Species {
            display_level: 12,
            group_size: Dice { base: 0, count: 2, sides: 4 },
            ..default()
        };
        assert_eq!(group_size(&strong, 5.0, Difficulty::Novice, &mut rng), 2);
        for _ in 0..20 {
            assert!(group_size(&strong, 5.0, Difficulty::Normal, &mut rng) <= 5);
        }
        let tiny = Species {
            display_level: 1,
            group_size: Dice { base: 0, count: 0, sides: 1 },
            ..default()
        };
        assert_eq!(group_size(&tiny, 5.0, Difficulty::Expert, &mut rng), 0);
        let weak = Species {
            display_level: 1,
            group_size: Dice { base: 0, count: 1, sides: 1 },
            ..default()
        };
        assert_eq!(group_size(&weak, 5.0, Difficulty::Expert, &mut rng), 1);
    }
}
