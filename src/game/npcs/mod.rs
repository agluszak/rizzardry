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

/// A person: one of the game's people, for the whole game. Never an actor:
/// a body [`Embodies`] them while they stand in a level, and their
/// companion self ([`CompanionOf`]) is them while it has a seat in the
/// party.
#[derive(Component, Clone, Copy, Debug)]
pub struct ActorId(pub NpcId);
impl ActorId {
    pub fn npc(&self) -> NpcId {
        self.0
    }
}

/// On the body a person stands in a level as: a monster like any other.
/// A person has one at most, and none while they travel with the party.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Embodied)]
pub struct Embodies(pub Entity);

/// The body a person stands in, maintained by Bevy from [`Embodies`].
#[derive(Component, Debug)]
#[relationship_target(relationship = Embodies)]
pub struct Embodied(Entity);

/// On the character a person is when they travel with the party: theirs
/// for the whole game, set aside while they have no seat.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = Companion)]
pub struct CompanionOf(pub Entity);

/// A person's companion self, maintained by Bevy from [`CompanionOf`].
#[derive(Component, Debug)]
#[relationship_target(relationship = CompanionOf, linked_spawn)]
pub struct Companion(Entity);

/// How a person regards the party, and what has passed between them.
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

/// Where the story has a person when they are not with the party.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub enum Whereabouts {
    /// Where a level places them.
    #[default]
    AsPlaced,
    /// At a named spot of a level, once the party is next there.
    Posted { map: String, spot: String },
    /// Taken from the world: nowhere until the story stands them somewhere.
    Gone,
    /// The body they stood in was killed, or they died in the party and
    /// were left: they stand nowhere again.
    Killed,
}

/// The species a person's body is of.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape(pub u32);

/// A person whose body dies is dead for good; one summoned was never
/// theirs.
fn body_killed(
    died: On<Add<crate::game::characters::Dead>>,
    bodies: Query<
        &Embodies,
        (
            With<crate::game::world::population::Monster>,
            Without<crate::game::magic::pointcasts::SummonedBy>,
        ),
    >,
    mut commands: Commands,
) {
    if let Ok(&Embodies(person)) = bodies.get(died.entity) {
        commands.entity(person).insert(Whereabouts::Killed);
    }
}

/// Who a person is at the moment, and who an actor is.
#[derive(bevy::ecs::system::SystemParam)]
pub struct People<'w, 's> {
    embodied: Query<'w, 's, &'static Embodied>,
    companions: Query<'w, 's, &'static Companion>,
    embodies: Query<'w, 's, &'static Embodies>,
    selves: Query<'w, 's, &'static CompanionOf>,
    seated: Query<'w, 's, (), With<crate::game::party::roster::PartyMember>>,
    ids: Query<'w, 's, &'static ActorId>,
}

impl People<'_, '_> {
    /// The body a person stands in, in whatever level.
    pub fn body(&self, person: Entity) -> Option<Entity> {
        self.embodied.get(person).ok().map(|embodied| embodied.0)
    }

    /// The member a person is, while they have a seat in the party.
    pub fn member(&self, person: Entity) -> Option<Entity> {
        self.companion(person).filter(|&companion| self.seated.contains(companion))
    }

    /// A person's companion self, seated or set aside.
    pub fn companion(&self, person: Entity) -> Option<Entity> {
        self.companions.get(person).ok().map(|companion| companion.0)
    }

    /// The person an actor is, if it is somebody: a body, or a companion.
    pub fn person(&self, actor: Entity) -> Option<Entity> {
        let body = self.embodies.get(actor).ok().map(|embodies| embodies.0);
        body.or_else(|| self.selves.get(actor).ok().map(|of| of.0))
    }

    /// Which of the game's people an actor is.
    pub fn id(&self, actor: Entity) -> Option<NpcId> {
        self.ids.get(self.person(actor)?).ok().map(|id| id.0)
    }
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
        }
    }
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
            .add_observer(story::dispatch)
            .add_observer(story::follow)
            .add_observer(story::slain)
            .add_observer(story::taken_up)
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
