//! NPC state outlives its current world binding; a binding is not its identity.
pub mod access;
pub mod conversation;
mod population;
pub mod recruit;
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
    /// The body they stood in was killed: they stand nowhere again. One who
    /// dies seated in the party is dead as any member is.
    pub killed: bool,
}
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
            killed: false,
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
}
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
                ),
            );
        app.add_observer(story::follow);
        conversation::install(app);
        recruit::install(app);
        trade::install(app);
    }
}
