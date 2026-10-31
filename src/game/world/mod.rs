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

/// The level rotation that looks from `here` toward `toward`, if they are
/// apart.
pub fn facing(here: Vec3, toward: Vec3) -> Option<Quat> {
    let delta = (toward - here).with_y(0.0);
    (delta.length_squared() > 0.0001)
        .then(|| Quat::from_rotation_y(crate::game::party::body::yaw_toward(delta)))
}

/// Turn an actor at `here`, level, to face the point `toward`. Its physics
/// rotation and its transform turn together, so physics' transform
/// synchronization cannot restore the old yaw.
pub fn face(
    here: Vec3,
    toward: Vec3,
    transform: &mut Transform,
    rotation: Option<Mut<avian3d::prelude::Rotation>>,
) {
    let Some(facing) = facing(here, toward) else { return };
    if let Some(mut rotation) = rotation {
        rotation.0 = facing;
    }
    transform.rotation = facing;
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
            .add_message::<level::Travel>()
            .add_message::<level::TravelAnswer>()
            .add_message::<search::ToggleSearch>()
            .add_message::<identify::Identified>()
            .add_message::<search::SearchNews>()
            .init_resource::<search::Search>()
            .init_resource::<level::LevelMemory>()
            .add_message::<triggers::TriggerEffect>()
            .add_message::<interaction::LockNews>()
            .add_message::<ActivateDevice>()
            .add_message::<items::TakeItem>()
            .add_message::<items::DropHeld>()
            .add_message::<population::Birth>()
            .add_message::<particles::EmissionSpent>()
            .add_message::<crate::game::automap::CommitNote>()
            .add_plugins(interaction::plugin)
            .add_observer(interaction::end_attempts)
            // Resolve travel intent early; activation owns the later level swap.
            .add_systems(PreUpdate, (level::answer_travel_questions, level::travel).chain())
            .add_observer(level::replace_travel_question)
            .add_observer(level::suspend_physics_for_entry)
            .add_observer(level::resume_physics_after_entry)
            .add_observer(search::suspend_search)
            .add_observer(search::resume_search)
            .add_systems(
                Update,
                (
                    level::activate_level.in_set(GameplaySet::LevelActivation),
                    (
                        activate_devices,
                        items::take_items,
                        items::drop_held,
                        crate::game::automap::commit_notes,
                    )
                        .chain()
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
                (encounters::write_off_wiped_out, encounters::run_encounters)
                    .chain()
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

/// The player uses a device, its reach checked when the request executes.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct ActivateDevice {
    pub device: Entity,
}

/// Run the devices the player uses, in order.
pub fn activate_devices(
    mut requests: MessageReader<ActivateDevice>,
    mut devices: triggers::Devices,
    geometry: Res<Assets<crate::content::level::LevelDefinition>>,
) {
    for &ActivateDevice { device } in requests.read() {
        devices.activate(&geometry, device, true);
    }
}
