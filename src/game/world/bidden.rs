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
    MonsterScript, ScriptCondition, ScriptPlace, ScriptStance, ScriptStep, StandingOrders,
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

/// What a script waits for before it goes on.
#[derive(Debug)]
enum Wait {
    Arrival,
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

/// A script shows something that is for the screen alone: an animation, a
/// sound, the camera turned to it, a death played out.
#[derive(Message, Clone, Debug)]
pub struct ScriptShow {
    pub body: Entity,
    /// The command as it was written.
    pub command: String,
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
        (Entity, &mut Running, &LevelOwned, Option<&GroupMember>, Option<&NpcBody>, Option<&Home>),
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
    mut shows: MessageWriter<ScriptShow>,
) {
    // The world waits while the party talks.
    if !conversations.is_empty() {
        return;
    }
    let party_at = places.of(*party);
    for (body, mut running, owner, member, npc, home) in &mut running {
        let Some(script) = scripts.get(&running.script) else { continue };
        let here = places.of(body);
        match &mut running.wait {
            Some(Wait::Arrival) => match walkers.get_mut(body) {
                Ok((_, state)) if *state != AgentState::ReachedTarget => continue,
                Ok((mut target, _)) => *target = AgentTarget3d::None,
                Err(_) => {}
            },
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
                        commands.trigger(Begin { actor, quote: *quote });
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
                ScriptStep::Show(command) => {
                    shows.write(ScriptShow { body, command: command.clone() });
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
