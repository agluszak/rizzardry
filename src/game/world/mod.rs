//! The active level and its local objects.

pub mod containers;
pub mod damage;
pub mod encounters;
pub mod hostility;
pub mod identify;
pub mod interaction;
pub mod items;
pub mod level;
pub mod navigation;
pub mod obstacles;
pub mod particles;
pub mod perception;
pub mod population;
pub mod props;
pub mod scripts;
pub mod search;
pub mod triggers;

use crate::game::GameplaySet;
use bevy::prelude::*;

/// A borrowed gameplay pose, never a second position cache. Physics owns an
/// embodied actor's pose; before embodiment its level-local placement is current.
/// Gameplay level roots have identity transforms.
#[derive(bevy::ecs::query::QueryData)]
pub struct ActorPose {
    transform: &'static Transform,
    position: Option<&'static avian3d::prelude::Position>,
    rotation: Option<&'static avian3d::prelude::Rotation>,
}

impl ActorPoseItem<'_, '_> {
    pub fn translation(&self) -> Vec3 {
        self.position.map_or(self.transform.translation, |position| position.0)
    }

    pub fn rotation(&self) -> Quat {
        self.rotation.map_or(self.transform.rotation, |rotation| rotation.0)
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }
}

/// An actor's position, for operations that hold the whole world: as
/// [`ActorPose`] reads it.
pub fn actor_position(world: &World, actor: Entity) -> Vec3 {
    world.get::<avian3d::prelude::Position>(actor).map_or_else(
        || world.get::<Transform>(actor).expect("an actor in the world").translation,
        |position| position.0,
    )
}

/// Turn an actor, level, to face the point `toward`, for operations that
/// hold the whole world. Its physics rotation and its transform turn
/// together, so physics' transform synchronization cannot restore the old
/// yaw.
pub fn face(world: &mut World, actor: Entity, toward: Vec3) {
    let here = actor_position(world, actor);
    let delta = (toward - here).with_y(0.0);
    if delta.length_squared() <= 0.0001 {
        return;
    }
    let facing = Quat::from_rotation_y(crate::game::party::body::yaw_toward(delta));
    if let Some(mut rotation) = world.get_mut::<avian3d::prelude::Rotation>(actor) {
        rotation.0 = facing;
    }
    world.get_mut::<Transform>(actor).unwrap().rotation = facing;
}

/// The highest step the party and walking monsters climb: 300 retail units
/// (0.6 m).
///
/// Retail has no explicit step height: it slides a 500-unit sphere, which
/// rides over edges well above the first 210-unit guess. At 210 units the
/// upper monastery's navmesh split into 22 pieces, the largest 61% of the
/// walkable area; at 300 one piece covers 98%. Raising it to 600 merged
/// nothing further. Characterize against retail stairs.
pub const STEP_HEIGHT: f32 = 300.0 / crate::content::UNITS_PER_METRE;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(navigation::NavigationPlugin)
            .add_message::<level::LevelReady>()
            .add_message::<level::LevelEntryFailed>()
            .add_message::<level::Travel>()
            .add_message::<level::TravelAnswer>()
            .add_message::<search::ToggleSearch>()
            .add_message::<identify::Identified>()
            .add_message::<search::SearchNews>()
            .init_resource::<search::Search>()
            .init_resource::<level::LevelMemory>()
            .add_message::<triggers::TriggerEffect>()
            .add_message::<interaction::LockNews>()
            .add_message::<WorldRequest>()
            .add_message::<population::Birth>()
            .add_message::<particles::EmissionSpent>()
            .add_message::<crate::game::automap::CommitNote>()
            .add_plugins(interaction::plugin)
            .add_observer(interaction::end_attempts)
            // Resolve travel intent early; activation owns the later level swap.
            .add_systems(PreUpdate, (level::answer_travel_questions, level::travel).chain())
            .add_observer(level::suspend_physics_for_entry)
            .add_observer(level::resume_physics_after_entry)
            .add_observer(search::suspend_search)
            .add_observer(search::resume_search)
            .add_systems(
                Update,
                (
                    level::activate_level.in_set(GameplaySet::LevelActivation),
                    (handle_world_requests, crate::game::automap::commit_notes)
                        .after(GameplaySet::Burden)
                        .before(GameplaySet::Combat),
                    particles::switch_off_spent.before(GameplaySet::LevelActivation),
                    population::give_birth.run_if(level::level_active).before(GameplaySet::Combat),
                    search::toggle_search.run_if(level::level_active).after(level::activate_level),
                ),
            )
            // Encounters come over ground physics has placed.
            .add_systems(
                FixedPostUpdate,
                encounters::run_encounters
                    .after(avian3d::prelude::PhysicsSystems::Writeback)
                    .run_if(level::level_active),
            )
            .add_systems(
                FixedPreUpdate,
                props::animate_props
                    .before(navigation::links::sync_door_links)
                    .run_if(level::level_active),
            )
            // Sweeps follow gameplay time, however many steps a frame runs.
            .add_systems(FixedUpdate, search::search_pulse.run_if(level::level_active));
    }
}

/// What the player asks of the world under the pointer.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum WorldRequest {
    /// Use a device, validating reach when the request executes.
    Activate {
        device: Entity,
    },
    TakeItem {
        item: Entity,
    },
    /// Throw the held item from the party's eye along `aim`.
    DropHeld {
        level: Entity,
        aim: Dir3,
    },
}

/// Carry out the player's world requests in order; a refused one changes
/// nothing.
pub fn handle_world_requests(world: &mut World) {
    let requests: Vec<_> = world.resource_mut::<Messages<WorldRequest>>().drain().collect();
    for request in requests {
        let refused = match request {
            WorldRequest::Activate { device } => {
                triggers::activate(world, device, true);
                None
            }
            WorldRequest::TakeItem { item } => items::take_item(world, item).err(),
            WorldRequest::DropHeld { level, aim } => items::drop_held(world, level, aim).err(),
        };
        if let Some(why) = refused {
            debug!("{request:?} refused: {why:?}");
        }
    }
}

/// Whether the party is entering a level or fighting, when device, loot and
/// trade operations wait.
pub fn party_occupied(world: &mut World) -> bool {
    level::entering(world) || crate::game::combat::in_battle(world)
}
