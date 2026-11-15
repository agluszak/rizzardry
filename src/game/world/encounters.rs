//! Random encounters (retail `MonGen.cpp`): a level's generators call up
//! groups from their tables now and then, out of the party's sight, within
//! a budget that grows with time away and shrinks as the party kills them;
//! those left alone long enough, and far enough off, are retired.

use avian3d::prelude::Position;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::identify::{EscortOf, Escorts, GroupMembers, MonsterGroup};
use super::level::{Level, LevelOwned};
use super::navigation::{LevelNavigation, Walker, WalkerMotionConfig, feet};
use super::obstacles::WorldObstacles;
use super::population::{self, Monster, MonsterBodies, Unborn};
use super::{ActorPose, STEP_HEIGHT};
use crate::content::items::Dice;
use crate::content::level::{EncounterLimits, EncounterTable, EncounterTime, Encounters};
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

/// The level's remaining encounter budget. Capacity is a rule of this value
/// and the level limits, never a second stored authority.
#[derive(Component, Debug)]
pub struct EncounterBudget {
    pub remaining: i32,
    limits: EncounterLimits,
    held: bool,
}

/// A level's arrival, settled once its ground is in place.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    First,
    Returned { away: u64 },
}

/// A generator's own state: whether it runs, and its timer, in gameplay
/// seconds (retail `MonGen::m_pTimer`, which runs on the pausable game
/// timer).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct EncounterGenerator {
    pub enabled: bool,
    pub timer: Timer,
    interval: f32,
    chance: i32,
    table: EncounterTable,
    harasses: bool,
}

impl EncounterBudget {
    /// Budget and capacity have separate bounds in the level definition.
    fn clamp(&mut self) {
        self.remaining =
            self.remaining.max(self.limits.smallest_budget).min(self.limits.largest_budget);
    }

    fn capacity(&self) -> i32 {
        self.remaining.max(self.limits.fewest).min(self.limits.most)
    }
}

/// Compile authored settings once. These entities, their timers and their
/// transforms remain with the level when it is set aside.
pub fn spawn_generators(
    commands: &mut Commands,
    level: Entity,
    encounters: &Encounters,
    rng: &mut GameRng,
) {
    let mut budget = EncounterBudget {
        remaining: encounters.budget,
        limits: encounters.limits,
        held: encounters.held,
    };
    budget.clamp();
    commands.entity(level).insert((budget, Arrival::First));
    for definition in &encounters.generators {
        let Some(table) = &definition.table else { continue };
        let (interval, chance) = if definition.uses_defaults {
            (f32::from(encounters.default_interval.max(0)), i32::from(encounters.default_chance))
        } else {
            (f32::from(definition.interval), i32::from(definition.chance))
        };
        let duration = jittered(interval, rng);
        let mut timer = Timer::from_seconds(duration, TimerMode::Once);
        timer.set_elapsed(std::time::Duration::from_secs_f32(
            definition.progress.max(0.0) * duration,
        ));
        commands.spawn((
            Name::new(definition.name.clone()),
            EncounterGenerator {
                enabled: definition.enabled && !definition.disabled,
                timer,
                interval,
                chance,
                table: table.clone(),
                harasses: definition.harasses,
            },
            Transform::from_translation(definition.position),
            LevelOwned(level),
        ));
    }
}

/// Returning wakes the same generators. Keep an arrival not yet settled if
/// travel happened before its first fixed step.
pub fn age_encounters(
    woke: On<super::level::Woke>,
    arrivals: Query<&Arrival>,
    mut commands: Commands,
) {
    let away = woke.away_ms / 1000;
    let arrival = match arrivals.get(woke.level) {
        Ok(Arrival::First) => Arrival::First,
        Ok(Arrival::Returned { away: pending }) => Arrival::Returned { away: pending + away },
        Err(_) => Arrival::Returned { away },
    };
    commands.entity(woke.level).insert(arrival);
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

/// Which of a table's entries comes now, chosen among those that suit (retail
/// `SelectEncounterCandidates`): a rarity class is drawn; entries of that
/// class, within half the party's level of a challenge and scaled less
/// common the further off, at a fitting time of day; failing any, any
/// entry of the class at a fitting time.
fn choose_entry(
    table: &EncounterTable,
    party_level: f32,
    night: bool,
    rng: &mut GameRng,
) -> Option<usize> {
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
    match candidates[..] {
        [] => None,
        [only] => Some(only),
        _ => Some(candidates[rng.below(candidates.len() as u32) as usize]),
    }
}

/// How many come, the group dice leaned on by how strong they are for the
/// party and the difficulty (retail `RollEncounterGroupSize`): strong ones
/// toward the fewer half on Normal (fewest on Novice, as rolled on
/// Expert); others, when the dice's middle is under one, toward the more
/// half on Normal (most on Expert, as rolled on Novice).
fn group_size(kind: &Species, party_level: f32, difficulty: Difficulty, rng: &mut GameRng) -> u32 {
    let dice: Dice = kind.group_size;
    let fewest = (dice.base + dice.count).max(0) as u32;
    let most = (dice.base + dice.count * dice.sides).max(0) as u32;
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

/// Place one group facing the party. If there is no ground under a scatter
/// candidate, use the last supported spot (or the generator itself).
fn spawn_group(
    commands: &mut Commands,
    root: Entity,
    (id, count): (u32, u32),
    (at, toward): (Vec3, Vec3),
    species: &MonsterCatalog,
    bodies: (&MonsterBodies, &Assets<MonsterBody>),
    navigation: &LevelNavigation,
    configs: &mut Assets<WalkerMotionConfig>,
    obstacles: &WorldObstacles,
    unborn: &mut Unborn,
    next: &mut (i32, i32),
    rng: &mut GameRng,
) -> Option<Entity> {
    let kind = &species[id];
    let definition = bodies.0.get(id, bodies.1).expect("bodies load with the level");
    let (location, authored) = *next;
    let mut members = population::roll_group(*next, (id, count), (kind, definition), rng);
    if members.is_empty() {
        return None;
    }
    *next = (location + members.len() as i32, authored + 1);
    let yaw = crate::game::party::body::yaw_toward(toward - at);
    let spots = population::scatter(
        (at, yaw, 2.0 * members[0].monster.body.radius),
        members.len(),
        rng,
        |spot| obstacles.ground_below(spot, GROUND_BELOW),
        |_| true,
        |_, _| true,
    );
    for (i, member) in members.iter_mut().enumerate() {
        member.feet = spots.get(i).or(spots.last()).copied().unwrap_or(at);
        member.yaw = yaw;
    }
    if kind.unborn {
        unborn.0.extend(members);
        return None;
    }
    Some(
        population::spawn_group(
            commands,
            root,
            members,
            (authored, None),
            species,
            (navigation, configs),
        )
        .0,
    )
}

/// A live random encounter, on its main group. Removing this component frees
/// capacity without removing the group or its surviving escorts.
#[derive(Component, Clone, Copy, Debug)]
pub struct Encounter {
    pub spawned: u64,
}

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(admit_and_retire).add_observer(spawn_encounter).add_systems(
        FixedPostUpdate,
        (write_off_wiped_out, poll_generators)
            .chain()
            .after(avian3d::prelude::PhysicsSystems::Writeback)
            .after(crate::game::time::pass_time)
            .run_if(super::level::level_active)
            .run_if(resource_exists::<crate::game::data::Catalogs>),
    );
}

// These events are private to the encounter plugin, each with one consumer.
// Bevy applies a trigger's commands before the next queued trigger: retirement
// precedes SpawnEncounter, and spawning precedes the next EncounterDue. This
// is the transaction boundary; there is no manual World runner or shadow roster.
#[derive(Event)]
struct EncounterDue {
    generator: Entity,
    forced: bool,
}

#[derive(Event)]
struct SpawnEncounter {
    generator: Entity,
    forced: bool,
}

/// Retiring a random encounter includes its escorts. Relationships alone do
/// not cascade: a defeated main group must leave surviving companions alive.
fn retire(
    commands: &mut Commands,
    groups: &Query<(Option<&GroupMembers>, Option<&Escorts>), With<MonsterGroup>>,
    owners: &Query<&LevelOwned>,
    root: Entity,
    leader: Entity,
) {
    let Ok((_, escorts)) = groups.get(leader) else { return };
    for group in std::iter::once(leader).chain(escorts.into_iter().flat_map(|e| e.iter())) {
        if !owners.get(group).is_ok_and(|owner| owner.0 == root) {
            continue;
        }
        if let Ok((members, _)) = groups.get(group) {
            for member in members.into_iter().flat_map(|m| m.iter()) {
                if owners.get(member).is_ok_and(|owner| owner.0 == root) {
                    commands.entity(member).despawn();
                }
            }
        }
        commands.entity(group).despawn();
        debug!(?group, "retired encounter group");
    }
}

/// Settle arrival once, then advance the active level's generator timers.
/// Forced rolls preserve timers. Normal attempts rearm when they become due,
/// including attempts that admission subsequently refuses.
fn poll_generators(
    mut commands: Commands,
    mut budgets: Query<(Entity, &mut EncounterBudget, Option<&Arrival>), With<Level>>,
    mut generators: Query<(Entity, &mut EncounterGenerator, &LevelOwned)>,
    mut encounters: Query<(Entity, &mut Encounter, &LevelOwned)>,
    groups: Query<(Option<&GroupMembers>, Option<&Escorts>), With<MonsterGroup>>,
    owners: Query<&LevelOwned>,
    party: Query<(), With<PartyBody>>,
    time: Res<Time<Fixed>>,
    mut rng: ResMut<GameRng>,
) {
    let Ok((root, mut budget, arrival)) = budgets.single_mut() else { return };
    if party.single().is_err() {
        return;
    }
    let mut order: Vec<_> = generators
        .iter()
        .filter(|(_, _, owner)| owner.0 == root)
        .map(|(entity, _, _)| entity)
        .collect();
    let mut forced = false;
    if let Some(arrival) = arrival {
        let elapsed = match *arrival {
            Arrival::First => {
                budget.remaining = budget.limits.largest_budget;
                STALE_SECONDS + 1
            }
            Arrival::Returned { away } => {
                if budget.limits.period > 0 {
                    budget.remaining += (away / budget.limits.period as u64) as i32;
                }
                away
            }
        };
        budget.clamp();
        commands.entity(root).remove::<Arrival>();
        forced = elapsed > STALE_SECONDS;
        debug!(level = ?root, budget = budget.remaining, forced, "settled encounter arrival");
        for (entity, mut encounter, owner) in &mut encounters {
            if owner.0 != root {
                continue;
            }
            if forced {
                retire(&mut commands, &groups, &owners, root, entity);
            } else {
                encounter.spawned += elapsed;
            }
        }
        if forced && order.len() > 1 {
            let count = order.len() as u32;
            for _ in 0..count {
                let (first, second) = (rng.below(count) as usize, rng.below(count) as usize);
                order.swap(first, second);
            }
        }
    }
    for entity in order {
        let (_, mut generator, _) = generators.get_mut(entity).unwrap();
        if !forced {
            if !generator.timer.tick(time.delta()).is_finished() {
                continue;
            }
            generator.timer =
                Timer::from_seconds(jittered(generator.interval, &mut rng), TimerMode::Once);
        }
        commands.trigger(EncounterDue { generator: entity, forced });
    }
}

/// Read the live level population for capacity, occupancy and id allocation.
/// Nothing here caches or patches positions; ActorPose reads physics directly.
#[derive(SystemParam)]
struct Population<'w, 's> {
    groups: Query<'w, 's, (&'static MonsterGroup, &'static LevelOwned)>,
    encounters: Query<
        'w,
        's,
        (Entity, &'static Encounter, &'static LevelOwned, Option<&'static GroupMembers>),
    >,
    monsters: Query<
        'w,
        's,
        (
            &'static Monster,
            &'static crate::game::characters::Vitals,
            ActorPose,
            Option<&'static Walker>,
            &'static LevelOwned,
        ),
    >,
}

impl Population<'_, '_> {
    fn count(&self, root: Entity) -> i32 {
        self.encounters.iter().filter(|(_, _, owner, _)| owner.0 == root).count() as i32
    }

    fn occupied(&self, root: Entity, spot: Vec3) -> bool {
        self.monsters.iter().any(|(_, vitals, pose, walker, owner)| {
            owner.0 == root
                && !vitals.is_dead()
                && (feet(pose.translation(), walker) - spot).abs().max_element() <= OCCUPIED
        })
    }
}

/// Admission's spatial gates precede culling. Apply retirements before the
/// second half reads capacity and occupancy, including on this same attempt.
fn admit_and_retire(
    attempt: On<EncounterDue>,
    mut commands: Commands,
    generators: Query<(&EncounterGenerator, &Transform, &LevelOwned)>,
    budgets: Query<&EncounterBudget>,
    party: Query<&Position, With<PartyBody>>,
    battle: Query<(), With<crate::game::combat::Battle>>,
    talking: Query<(), With<crate::game::npcs::conversation::Conversation>>,
    population: Population,
    groups: Query<(Option<&GroupMembers>, Option<&Escorts>), With<MonsterGroup>>,
    owners: Query<&LevelOwned>,
    obstacles: WorldObstacles,
    clock: Res<GameClock>,
) {
    let Ok((generator, at, owner)) = generators.get(attempt.generator) else { return };
    let (Ok(budget), Ok(party)) = (budgets.get(owner.0), party.single()) else { return };
    if !battle.is_empty() || !talking.is_empty() || budget.held || !generator.enabled {
        return;
    }
    let eye = PartyBody::eye(party);
    if !attempt.forced
        && (!(NEAREST..=FARTHEST).contains(&eye.distance(at.translation))
            || obstacles.clear(eye, at.translation + Vec3::Y * STEP_HEIGHT))
    {
        return;
    }
    if population.count(owner.0) >= budget.capacity() {
        let now_s = clock.ms / 1000;
        let lingers = budget.limits.lingers as f32 * LINGER_SCALE;
        for (entity, encounter, group_owner, members) in &population.encounters {
            if group_owner.0 != owner.0 || (now_s - encounter.spawned) as f32 <= lingers {
                continue;
            }
            let at = members.into_iter().flat_map(|m| m.iter()).find_map(|m| {
                let (_, _, pose, walker, _) = population.monsters.get(m).ok()?;
                Some(feet(pose.translation(), walker))
            });
            if at.is_none_or(|at| at.distance(PartyBody::feet(party)) > RETIRED_BEYOND) {
                retire(&mut commands, &groups, &owners, owner.0, entity);
            }
        }
    }
    commands.trigger(SpawnEncounter { generator: attempt.generator, forced: attempt.forced });
}

/// Commit one generator against the population after culling. Commands from
/// this call apply before the next attempt; no parallel active/standing lists
/// are needed to account for deferred entities.
fn spawn_encounter(
    attempt: On<SpawnEncounter>,
    mut commands: Commands,
    generators: Query<(&EncounterGenerator, &Transform, &LevelOwned)>,
    mut levels: Query<(&EncounterBudget, &LevelNavigation, &mut Unborn, &MonsterBodies)>,
    population: Population,
    party: Query<(&Position, &crate::game::party::roster::PartyRoster), With<PartyBody>>,
    characters: Query<(&crate::game::characters::Character, Has<crate::game::npcs::Npc>)>,
    obstacles: WorldObstacles,
    definitions: Res<Assets<MonsterBody>>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    clock: Res<GameClock>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    content: Res<crate::game::data::Catalogs>,
) {
    let Ok((generator, at, owner)) = generators.get(attempt.generator) else { return };
    let Ok((budget, navigation, mut unborn, bodies)) = levels.get_mut(owner.0) else { return };
    let Ok((party_at, roster)) = party.single() else { return };
    if population.count(owner.0) >= budget.capacity()
        || (!attempt.forced
            && (population.occupied(owner.0, at.translation)
                || (rng.below(100) as i32) >= generator.chance))
    {
        return;
    }
    // Harassment's possession gate is not implemented yet.
    if generator.harasses {
        return;
    }
    let party_level = crate::game::party::mean_level(
        crate::game::party::averaged_places(roster.iter().map(|m| characters.get(m).unwrap()))
            .into_iter()
            .map(|c| c.level()),
    );
    let Some(chosen) = choose_entry(&generator.table, party_level, night(&clock), &mut rng) else {
        return;
    };
    let species = &content.species;
    let id = generator.table.entries[chosen].species;
    let kind = &species[id];
    let count = group_size(kind, party_level, *difficulty, &mut rng);
    let mut next = population::next_ids(
        population
            .monsters
            .iter()
            .filter(|(.., level)| level.0 == owner.0)
            .map(|(monster, ..)| monster.location),
        population
            .groups
            .iter()
            .filter(|(_, level)| level.0 == owner.0)
            .map(|(group, _)| group.authored),
        &unborn,
    );
    let mut spawn = |id, count, rng: &mut GameRng| {
        spawn_group(
            &mut commands,
            owner.0,
            (id, count),
            (at.translation, PartyBody::feet(party_at)),
            species,
            (bodies, &definitions),
            navigation,
            &mut configs,
            &obstacles,
            &mut unborn,
            &mut next,
            rng,
        )
    };
    let Some(leader) = spawn(id, count, &mut rng) else { return };
    let mut escorts = Vec::new();
    for &(companion, chance) in kind.companions.iter().flatten() {
        if rng.below(100) < u32::from(chance) {
            let count = rng.roll(species[companion].group_size);
            if let Some(escort) = spawn(companion, count, &mut rng) {
                escorts.push(escort);
            }
        }
    }
    commands.entity(leader).insert(Encounter { spawned: clock.ms / 1000 });
    for escort in escorts {
        commands.entity(escort).insert(EscortOf(leader));
    }
    debug!(generator = ?attempt.generator, group = ?leader, "generated encounter");
}

/// Remove defeated encounters from capacity exactly once. Only combat defeats
/// spend budget; surviving escorts remain ordinary groups on the level.
pub fn write_off_wiped_out(
    mut commands: Commands,
    mut levels: Query<(Entity, &mut EncounterBudget), With<Level>>,
    groups: Query<(Entity, &LevelOwned, Option<&GroupMembers>), With<Encounter>>,
    monsters: Query<&crate::game::characters::Vitals, With<Monster>>,
    battle: Option<Single<(), With<crate::game::combat::Battle>>>,
) {
    let Ok((root, mut budget)) = levels.single_mut() else { return };
    for (entity, owner, members) in &groups {
        if owner.0 != root {
            continue;
        }
        let gone = members.is_none_or(|members| {
            members.iter().all(|member| monsters.get(member).is_ok_and(|vitals| vitals.is_dead()))
        });
        if gone {
            commands.entity(entity).remove::<Encounter>();
            if battle.is_some() {
                budget.remaining -= 1;
                budget.clamp();
            }
        }
    }
}
