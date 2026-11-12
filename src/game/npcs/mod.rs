//! NPC state outlives its current world binding; a binding is not its identity.
pub mod access;
pub mod conversation;
pub mod devices;
mod population;
pub mod recruit;
pub mod scenes;
pub mod story;
pub mod trade;
use crate::content::npcs::{DialogueScript, NpcDefinition, NpcId};
use bevy::prelude::*;
pub use population::{NpcPopulation, Placing};
use std::collections::HashMap;

#[derive(Component, Clone, Copy, Debug)]
pub struct ActorId(pub NpcId);
impl ActorId {
    pub fn npc(&self) -> NpcId {
        self.0
    }
}

#[derive(Component, Debug)]
#[relationship(relationship_target = NpcBodies)]
pub struct NpcBody(pub Entity);
#[derive(Component, Debug, Deref)]
#[relationship_target(relationship = NpcBody)]
pub struct NpcBodies(Vec<Entity>);

#[derive(Component, Debug)]
pub struct Npc {
    pub disposition: u8,
    pub greeting_pending: bool,
    pub paid_topics: Vec<u16>,
    pub mindread_refusals: u8,
    pub trade_pool: u16,
    pub charmed_at: Option<u64>,
    pub small_talk_at: Option<u64>,
}

/// The body a person stood in was killed: they stand nowhere again. One who
/// dies seated in the party is dead as any member is.
#[derive(Component, Clone, Copy, Debug)]
pub struct Killed;

/// An NPC whose body dies is dead for good; one summoned was never theirs.
fn body_killed(
    died: On<Add<crate::game::characters::Dead>>,
    bodies: Query<&NpcBody, Without<crate::game::magic::pointcasts::SummonedBy>>,
    mut commands: Commands,
) {
    if let Ok(&NpcBody(actor)) = bodies.get(died.entity) {
        commands.entity(actor).insert(Killed);
    }
}

/// The story has taken a person from the world: they stand nowhere until it
/// stands them somewhere.
#[derive(Component, Clone, Copy, Debug)]
pub struct Gone;
impl Npc {
    fn from_definition(d: &NpcDefinition) -> Self {
        Self {
            disposition: d.disposition,
            greeting_pending: true,
            paid_topics: vec![],
            mindread_refusals: 0,
            trade_pool: d.trade_pool,
            charmed_at: None,
            small_talk_at: None,
        }
    }
}
/// Where an NPC stands, and what their body is: a level's path and a species.
#[derive(Component, Clone, Debug)]
pub struct NpcLocation {
    pub map: String,
    pub species: u32,
}
/// A trigger can provide an interaction binding without a monster.
#[derive(Component, Debug)]
pub struct NpcTrigger(pub Entity);
#[derive(Component)]
pub struct NpcSpeaker(pub Entity);
#[derive(Resource, Default)]
pub struct NpcAssets {
    pub scripts: HashMap<NpcId, Handle<DialogueScript>>,
    /// The lines of those the story may rename, under their other name.
    pub renamed: HashMap<NpcId, Handle<DialogueScript>>,
}

/// The name the story has given someone in place of their own; their lines
/// and voice are that name's.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Alias(pub String);
/// On an NPC that is one of a level's placed monsters rather than someone
/// with a name of their own: which placement of which level.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    map: String,
    location: i32,
}

/// The NPCs with names of their own have been made, as the first level is
/// entered.
#[derive(Resource)]
pub(super) struct Peopled;

pub struct NpcPlugin;
impl Plugin for NpcPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<DialogueScript>()
            .init_resource::<NpcAssets>()
            .init_resource::<story::StoryFacts>()
            .add_systems(
                Update,
                (
                    population::initialize.before(crate::game::GameplaySet::LevelActivation),
                    population::bind_triggers.after(crate::game::GameplaySet::LevelActivation),
                    devices::gate_devices.after(crate::game::GameplaySet::LevelActivation),
                    devices::enter.after(crate::game::GameplaySet::LevelActivation),
                    population::station
                        .after(crate::game::GameplaySet::LevelActivation)
                        .run_if(crate::game::world::level::level_active),
                    story::marked_slip_away
                        .after(crate::game::GameplaySet::LevelActivation)
                        .run_if(crate::game::world::level::level_active)
                        .run_if(any_with_component::<story::Marked>),
                ),
            );
        app.add_message::<story::StoryTold>()
            .add_observer(body_killed)
            .add_observer(story::follow)
            .add_observer(story::slain)
            .add_observer(story::taken_up)
            .add_observer(story::hostiles_die)
            .add_observer(scenes::befalls_the_fit)
            .add_observer(story::happens)
            .add_observer(story::follows_cycle)
            .add_observer(story::faded)
            .add_observer(story::befalls_if)
            .add_observer(devices::ask)
            .add_observer(devices::watch)
            .add_observer(devices::shoots)
            .add_observer(devices::crush_begins)
            .add_observer(devices::befalls_at_a_device)
            .add_observer(scenes::picked)
            .add_observer(scenes::befalls_the_party)
            .add_observer(scenes::ends)
            .add_observer(scenes::falls)
            .add_systems(
                FixedUpdate,
                (
                    scenes::pine,
                    scenes::awaited,
                    scenes::recur,
                    scenes::parted,
                    devices::still,
                    devices::crush,
                )
                    .run_if(crate::game::world::level::level_active),
            )
            .add_systems(
                FixedUpdate,
                story::come_due.run_if(crate::game::world::level::level_active),
            )
            .add_observer(story::marked_leave)
            .add_observer(story::befalls_the_story)
            .add_observer(story::befalls_the_level)
            .add_observer(story::befalls_someone);
        conversation::install(app);
        recruit::install(app);
        trade::install(app);
    }
}
