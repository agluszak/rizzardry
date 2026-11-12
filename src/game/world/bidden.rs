//! What a monster is told to do (`docs/rules/scripts.md`): a monster given
//! a script ([`Script`]) carries it out a step at a time while it is not
//! fighting ([`run`]), and then keeps to the orders it ended with
//! ([`Orders`]).

use avian3d::prelude::{Position, Rotation};
use bevy::prelude::*;
use bevy_landmass::prelude::{AgentState, AgentTarget3d};

use super::facing;
use super::hostility::{Stance, Stand};
use super::identify::GroupMember;
use super::level::{Level, LevelOwned};
use super::obstacles::WorldObstacles;
use super::population::Monster;
use super::triggers::{self, Bidding};
use crate::content::level::LevelDefinition;
use crate::content::scripts::{
    Duty, MonsterScript, ScriptCondition, ScriptPlace, ScriptStance, ScriptStep, StandingOrders,
};
use crate::game::actors::Places;
use crate::game::combat::Combatant;
use crate::game::npcs::NpcBody;
use crate::game::npcs::conversation::{Begin, Conversation, Overheard};
use crate::game::party::body::PartyBody;
use crate::game::rng::GameRng;

/// The most commands a script carries out in one step (retail
/// `ProcessScript`).
const COMMANDS_A_STEP: u32 = 50;

/// The script a monster is given, by its file's name: by its placement, or
/// by the story. Giving another replaces it.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Script(pub String);

/// Where a monster was placed: what its script calls home.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Home(pub Vec3);

/// How a monster keeps itself once its script has run to its orders.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Orders(pub StandingOrders);

/// A body plays an animation once, by its name among the body's cycles.
/// The screen plays it and says when it is over ([`Played`]).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Cycle(pub String);

/// An animation a body was playing has been played out.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Played {
    #[event_target]
    pub body: Entity,
}

/// An animation played out is over.
pub(super) fn played(played: On<Played>, mut commands: Commands) {
    commands.entity(played.body).try_remove::<Cycle>();
}

/// What a script waits for before it goes on.
#[derive(Debug)]
enum Wait {
    Arrival,
    /// The animation it plays to be over.
    Cycle,
    /// A conversation, or a line it said, to be over.
    Talk,
    Delay(Timer),
}

/// A script being carried out: the command it has come to, the conditions
/// it is inside, and what it waits for.
#[derive(Component, Debug)]
pub struct Running {
    script: Handle<MonsterScript>,
    at: usize,
    conditions: Vec<bool>,
    wait: Option<Wait>,
}

/// A script comes to one of the story's named happenings.
#[derive(EntityEvent, Clone, Debug)]
pub struct ScriptAction {
    #[event_target]
    pub body: Entity,
    pub action: String,
}

/// A monster's script is sent on to one of its labels (retail
/// `SetScriptLabel`): one whose script is over goes nowhere.
#[derive(EntityEvent, Clone, Debug)]
pub struct Jump {
    #[event_target]
    pub body: Entity,
    pub label: String,
}

pub(super) fn jump(
    jump: On<Jump>,
    mut running: Query<&mut Running>,
    scripts: Res<Assets<MonsterScript>>,
) {
    let Ok(mut running) = running.get_mut(jump.body) else { return };
    let label = jump.label.to_ascii_uppercase();
    let to = scripts.get(&running.script).and_then(|script| script.labels.get(&label)).copied();
    if let Some(to) = to {
        running.at = to;
        running.wait = None;
        running.conditions.clear();
    }
}

/// A monster given a script begins it from the top, with home where it
/// stands if it had none.
pub(super) fn begin(
    given: On<Insert<Script>>,
    bodies: Query<(&Script, Has<Home>)>,
    places: Places,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let body = given.entity;
    let Ok((Script(name), has_home)) = bodies.get(body) else { return };
    let path = format!("retail://data/monsters/scripts/{}", name.to_lowercase());
    let mut entity = commands.entity(body);
    entity.remove::<Orders>().insert(Running {
        script: assets.load(path),
        at: 0,
        conditions: Vec::new(),
        wait: None,
    });
    if !has_home {
        entity.insert(Home(places.of(body).feet));
    }
}

/// Carry each script on (retail `ProcessScript`), for monsters not in a
/// fight and while the party is not talking: up to fifty commands a step,
/// until one waits or jumps. A script that runs out leaves its orders.
pub(super) fn run(
    mut commands: Commands,
    mut running: Query<
        (
            (Entity, &mut Running, &LevelOwned),
            (Option<&GroupMember>, Option<&NpcBody>, Option<&Home>, Has<Cycle>),
        ),
        (With<Monster>, Without<Combatant>),
    >,
    (scripts, definitions): (Res<Assets<MonsterScript>>, Res<Assets<LevelDefinition>>),
    levels: Query<&Level>,
    (places, obstacles): (Places, WorldObstacles),
    party: Single<Entity, With<PartyBody>>,
    conversations: Query<(), With<Conversation>>,
    mut walkers: Query<(&mut AgentTarget3d, &AgentState)>,
    time: Res<Time>,
    mut rng: ResMut<GameRng>,
    mut effects: MessageWriter<super::triggers::TriggerEffect>,
) {
    // The world waits while the party talks.
    if !conversations.is_empty() {
        return;
    }
    let party_at = places.of(*party);
    for ((body, mut running, owner), (member, npc, home, cycling)) in &mut running {
        let Some(script) = scripts.get(&running.script) else { continue };
        let here = places.of(body);
        match &mut running.wait {
            Some(Wait::Arrival) => match walkers.get_mut(body) {
                Ok((_, state)) if *state != AgentState::ReachedTarget => continue,
                Ok((mut target, _)) => *target = AgentTarget3d::None,
                Err(_) => {}
            },
            Some(Wait::Cycle) if cycling => continue,
            Some(Wait::Cycle) => {}
            Some(Wait::Delay(timer)) => {
                if !timer.tick(time.delta()).is_finished() {
                    continue;
                }
            }
            // No conversation is under way, or this would not be reached.
            Some(Wait::Talk) | None => {}
        }
        running.wait = None;
        let spot = |place: &ScriptPlace| match place {
            ScriptPlace::Party => Some(party_at.feet),
            ScriptPlace::Home => home.map(|home| home.0),
            ScriptPlace::Spot(name) => levels
                .get(owner.0)
                .ok()
                .and_then(|level| definitions.get(&level.definition))
                .and_then(|definition| definition.spot(name)),
        };
        for _ in 0..COMMANDS_A_STEP {
            let Some(step) = script.steps.get(running.at) else {
                let mut entity = commands.entity(body);
                entity.remove::<Running>();
                if let Some(orders) = &script.orders {
                    // East is 0 and the compass turns counter-clockwise.
                    if let Some(way) = orders.facing {
                        let ahead = here.feet + Vec3::new(way.cos(), 0.0, -way.sin());
                        entity.insert(Rotation(facing(here.feet, ahead).unwrap_or_default()));
                    }
                    entity.insert(Orders(orders.clone()));
                }
                break;
            };
            running.at += 1;
            // Inside a condition that does not hold, only its end is heard.
            if running.conditions.last() == Some(&false)
                && !matches!(step, ScriptStep::Else | ScriptStep::EndIf)
            {
                continue;
            }
            match step {
                ScriptStep::Goto(to) => {
                    running.conditions.clear();
                    running.at = *to;
                    break;
                }
                ScriptStep::WalkTo { place, waits } => {
                    if let Ok((mut target, _)) = walkers.get_mut(body) {
                        *target = match (place, spot(place)) {
                            (ScriptPlace::Party, _) => AgentTarget3d::Entity(*party),
                            (_, Some(point)) => AgentTarget3d::Point(point),
                            (_, None) => AgentTarget3d::None,
                        };
                    }
                    if *waits {
                        running.wait = Some(Wait::Arrival);
                    }
                    break;
                }
                ScriptStep::Face(place) => {
                    if let Some(turned) = spot(place).and_then(|at| facing(here.feet, at)) {
                        commands.entity(body).insert(Rotation(turned));
                    }
                }
                ScriptStep::Teleport(place) => {
                    if let Some(at) = spot(place) {
                        commands.entity(body).insert(Position(at));
                    }
                }
                ScriptStep::Say { quote, waits } => {
                    if let (Some(&NpcBody(actor)), Some(quote)) = (npc, *quote) {
                        commands.trigger(Overheard { actor, quote });
                    }
                    if *waits {
                        running.wait = Some(Wait::Talk);
                    }
                    break;
                }
                ScriptStep::Talk { quote } => {
                    if let Some(&NpcBody(actor)) = npc {
                        commands.trigger(Begin { actor, quote: *quote, with_held: false });
                    }
                    running.wait = Some(Wait::Talk);
                    break;
                }
                ScriptStep::If { condition, negated } => {
                    let holds = match condition {
                        Some(ScriptCondition::PartyNear(metres)) => {
                            here.feet.distance(party_at.eye) < *metres
                                && obstacles.clear(here.eye, party_at.eye)
                        }
                        Some(ScriptCondition::Random(chance)) => rng.below(100) < *chance,
                        None => false,
                    };
                    running.conditions.push(holds != *negated);
                }
                ScriptStep::Else => {
                    if let Some(holds) = running.conditions.last_mut() {
                        *holds = !*holds;
                    }
                }
                ScriptStep::EndIf => {
                    running.conditions.pop();
                }
                ScriptStep::Disposition(stance) => {
                    if let Some(&GroupMember(group)) = member {
                        let stance = match stance {
                            ScriptStance::Neutral => Stance::Neutral,
                            ScriptStance::Hostile => Stance::Hostile,
                            ScriptStance::Friendly => Stance::Friendly,
                        };
                        commands.trigger(Stand { group, stance: Some(stance) });
                    }
                }
                ScriptStep::Trigger(name) => {
                    let run = Bidding::Run(name.clone());
                    commands.queue(move |world: &mut World| triggers::bid(world, run));
                }
                ScriptStep::Delay(seconds) => {
                    running.wait =
                        Some(Wait::Delay(Timer::from_seconds(*seconds, TimerMode::Once)));
                    break;
                }
                ScriptStep::Cycle { name, waits } => {
                    commands.entity(body).insert(Cycle(name.clone()));
                    if *waits {
                        running.wait = Some(Wait::Cycle);
                    }
                    break;
                }
                ScriptStep::Sound { path, volume } => {
                    effects.write(super::triggers::TriggerEffect::Sound {
                        path: path.clone(),
                        volume: volume / 127.0,
                        at: Some(here.feet),
                    });
                }
                ScriptStep::LookHere => {
                    commands.trigger(crate::game::party::body::LookAt { at: here.feet });
                }
                ScriptStep::Vanish => {
                    commands.entity(body).despawn();
                    break;
                }
                ScriptStep::DoAction(action) => {
                    commands.trigger(ScriptAction { body, action: action.clone() });
                }
                ScriptStep::End => running.at = script.steps.len(),
            }
        }
    }
}

/// A monster ordered to face the party turns to it while it has a clear
/// line to it.
pub(super) fn face_party(
    mut commands: Commands,
    ordered: Query<(Entity, &Orders), (With<Monster>, Without<Combatant>)>,
    (places, obstacles): (Places, WorldObstacles),
    party: Single<Entity, With<PartyBody>>,
) {
    let party_at = places.of(*party);
    for (body, Orders(orders)) in &ordered {
        let here = places.of(body);
        if orders.faces_party
            && obstacles.clear(here.eye, party_at.eye)
            && let Some(turned) = facing(here.feet, party_at.feet)
        {
            commands.entity(body).insert(Rotation(turned));
        }
    }
}

/// Which of its patrol's places a monster heads for.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Beat(usize);

/// A monster within this of a place of its orders is at it (retail's 1500
/// units).
const AT_ITS_PLACE: f32 = 3.0;

/// A monster keeps its orders while it is not fighting (retail
/// `ChooseMonsterRTAIMode`): a guard walks back to its post when it is far
/// from it; a patrol over places heads for each in turn, or for another at
/// random; a patrol of an area wanders within its range of home and turns
/// back when beyond it. A monster not fit to act keeps none, and neither
/// does one whose group has a member not fit.
pub(super) fn keep_orders(
    mut ordered: Query<
        (
            (Entity, &Orders, &LevelOwned, Option<&Home>, Option<&mut Beat>, Option<&Glances>),
            (&mut AgentTarget3d, &AgentState, Option<&GroupMember>),
        ),
        (
            With<Monster>,
            Without<Combatant>,
            Without<super::pursuit::Charging>,
            Without<super::noise::Investigating>,
        ),
    >,
    bodies: Query<(Has<crate::game::actors::CanFight>, Option<&GroupMember>), With<Monster>>,
    groups: Query<&super::identify::MonsterGroup>,
    (levels, definitions): (Query<&Level>, Res<Assets<LevelDefinition>>),
    (places, party): (Places, Single<Entity, With<PartyBody>>),
    conversations: Query<(), With<Conversation>>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    if !conversations.is_empty() {
        return;
    }
    let unfit: Vec<Entity> = bodies
        .iter()
        .filter(|(can_fight, _)| !can_fight)
        .filter_map(|(_, member)| member.map(|member| member.0))
        .collect();
    for ((body, Orders(orders), owner, home, beat, glances), (mut target, state, member)) in
        &mut ordered
    {
        // A resting monster stands.
        if glances.is_some_and(|glances| glances.resting) {
            *target = AgentTarget3d::None;
            continue;
        }
        let Some(duty) = &orders.duty else { continue };
        let fit = bodies.get(body).is_ok_and(|(can_fight, _)| can_fight);
        if !fit || member.is_some_and(|member| unfit.contains(&member.0)) {
            continue;
        }
        let here = places.of(body).feet;
        let spot = |place: &ScriptPlace| match place {
            ScriptPlace::Party => Some(places.of(*party).feet),
            ScriptPlace::Home => home.map(|home| home.0),
            ScriptPlace::Spot(name) => levels
                .get(owner.0)
                .ok()
                .and_then(|level| definitions.get(&level.definition))
                .and_then(|definition| definition.spot(name)),
        };
        let heads = match duty {
            Duty::Guard(place) => spot(place).filter(|post| post.distance(here) >= AT_ITS_PLACE),
            Duty::PointPatrol { places: beat_places, random } => {
                let Some(mut beat) = beat else {
                    commands.entity(body).insert(Beat::default());
                    continue;
                };
                let count = beat_places.len();
                let at = beat_places.get(beat.0).and_then(spot);
                if at.is_none_or(|at| at.distance(here) < AT_ITS_PLACE) {
                    beat.0 = if !random {
                        (beat.0 + 1) % count
                    } else if count < 2 {
                        0
                    } else {
                        // Another of them, each as likely.
                        let other = rng.below(count as u32 - 1) as usize;
                        if other >= beat.0 { other + 1 } else { other }
                    };
                }
                beat_places.get(beat.0).and_then(spot)
            }
            // The original walks a path of its own choosing between the two
            // lengths; here it makes for a point as far.
            Duty::Patrol { distance, variation } => {
                let Some(home) = home.map(|home| home.0) else { continue };
                let aimed = !matches!(*target, AgentTarget3d::None);
                // One that can find no way where its patrol sends it: an
                // encounter leaves the level; another tries elsewhere
                // (retail `HandleMonsterPatrolFailure`).
                if aimed && matches!(*state, AgentState::NoPath | AgentState::TargetNotOnNavMesh) {
                    let group = member.map(|member| member.0);
                    let encounter = group.and_then(|group| groups.get(group).ok());
                    if let (Some(group), Some(true)) =
                        (group, encounter.map(|group| group.encounter.is_some()))
                    {
                        commands.trigger(super::encounters::Retire { group });
                        continue;
                    }
                } else if aimed && *state != AgentState::ReachedTarget {
                    continue;
                }
                if home.distance(here) > *variation {
                    Some(home)
                } else {
                    let turn = rng.below(360) as f32 * std::f32::consts::PI / 180.0;
                    let span = (*variation - *distance).max(0.0);
                    let far = *distance + span * rng.below(1000) as f32 / 1000.0;
                    Some(here + Quat::from_rotation_y(turn) * Vec3::NEG_Z * far)
                }
            }
        };
        *target = heads.map_or(AgentTarget3d::None, AgentTarget3d::Point);
    }
}

/// A monster ordered to look about keeps its orders for some minutes, then
/// rests for some, by turns (retail `LOOKABOUT`, kept in game minutes): each
/// spell of either lasts from half to one and a half times what the order
/// gives, the first its full length.
#[derive(Component, Clone, Copy, Debug)]
pub struct Glances {
    resting: bool,
    /// Game milliseconds at which the spell ends.
    until: u64,
    step: u32,
}

const A_MINUTE: u64 = 60_000;

/// While resting a monster stands, and now and then turns to a heading at
/// random: one time in twenty that it thinks, which it does every fifth
/// step. (The original thinks every fifth frame; its frame rate is not
/// fixed, so how often it turns is this game's own.)
pub(super) fn look_about(
    mut ordered: Query<
        (Entity, &Orders, Option<&mut Glances>),
        (With<Monster>, Without<Combatant>),
    >,
    clock: Res<crate::game::time::GameClock>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    for (body, Orders(orders), glances) in &mut ordered {
        let Some((often, lasts)) = orders.looks_about else { continue };
        let Some(mut glances) = glances else {
            let until = clock.ms + u64::from(often) * A_MINUTE;
            commands.entity(body).insert(Glances { resting: false, until, step: 0 });
            continue;
        };
        if clock.ms >= glances.until {
            glances.resting = !glances.resting;
            let length = if glances.resting { lasts } else { often };
            let minutes = (rng.below(length.max(1)) + length / 2).max(1);
            glances.until = clock.ms + u64::from(minutes) * A_MINUTE;
        }
        if glances.resting {
            glances.step += 1;
            if glances.step % 5 == 0 && rng.below(20) == 0 {
                let heading = (rng.below(360) as f32).to_radians();
                commands.entity(body).insert(Rotation(Quat::from_rotation_y(heading)));
            }
        }
    }
}
