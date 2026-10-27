//! The active level and its local objects.

pub mod damage;
pub mod identify;
pub mod items;
pub mod level;
pub mod navigation;
pub mod obstacles;
pub mod population;
pub mod props;
pub mod scripts;
pub mod search;
pub mod triggers;

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

/// The active map of an actor. Ordinary actors are map-owned; a named NPC
/// instead has a presence relationship whose removal cannot despawn its identity.
#[derive(bevy::ecs::query::QueryData)]
pub struct ActorLevel {
    presence: Option<&'static crate::game::npcs::NpcPresence>,
    parent: Option<&'static ChildOf>,
}
impl ActorLevelItem<'_, '_> {
    pub fn level(&self) -> Option<Entity> {
        self.presence.map(|p| p.0).or_else(|| self.parent.map(ChildOf::parent))
    }
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
            .init_resource::<population::MonsterBodies>()
            .add_message::<triggers::TriggerEffect>()
            // Resolve travel intent early; activation owns the later level swap.
            .add_systems(PreUpdate, (level::answer_travel_questions, level::travel).chain())
            .add_systems(
                Update,
                (
                    level::activate_level.in_set(crate::game::GameplaySet::LevelActivation),
                    (search::toggle_search, search::search_pulse)
                        .chain()
                        .run_if(level::level_active)
                        .after(level::activate_level),
                ),
            )
            .add_systems(
                FixedPreUpdate,
                props::animate_props
                    .before(navigation::links::sync_door_links)
                    .run_if(level::level_active),
            );
    }
}
