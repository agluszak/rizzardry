//! The party's roster, formation and spatial body.

pub mod body;
pub mod formation;
pub mod recovery;
pub mod roster;
pub mod walk_in;

use avian3d::prelude::{Gravity, PhysicsSystems};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use crate::game::camp;
use crate::game::screen::party_free;
use crate::game::world::level::level_active;
use crate::game::world::triggers::PendingPartyMove;

/// A member's standing choice for the next round. Recruitment starts on
/// Attack (retail `ResetPartySlotRow`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Attack,
    /// No attack; +2 armour, and stamina back at the round's end.
    Defend,
}

pub struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            TnuaControllerPlugin::<body::PartyMotion>::new(FixedUpdate),
            TnuaAvian3dPlugin::new(FixedUpdate),
        ))
        .insert_resource(Gravity(Vec3::NEG_Y * body::GRAVITY))
        .add_input_context::<body::Exploration>()
        .init_resource::<roster::PartyRoster>()
        .init_resource::<formation::Formation>()
        .add_message::<camp::CampNews>()
        .add_message::<camp::CampRequest>()
        .add_message::<camp::LeaveCamp>()
        .add_systems(Startup, body::spawn_exploration_controls)
        .add_systems(
            Update,
            (
                roster::give_starting_equipment.in_set(crate::game::GameplaySet::Equipment),
                (
                    body::look_around
                        .run_if(party_free)
                        .after(crate::game::GameplaySet::LevelActivation)
                        .before(crate::game::GameplaySet::Perception),
                    (
                        camp::request_camp.run_if(party_free),
                        camp::force_camp,
                        camp::leave_camp,
                        camp::advance_camp,
                    )
                        .chain()
                        .after(crate::game::GameplaySet::Combat),
                )
                    .run_if(level_active),
            ),
        )
        // Tnua and Avian step on the fixed timestep; movement input feeds
        // them on the same step.
        .init_resource::<PendingPartyMove>()
        .add_systems(
            FixedUpdate,
            (
                (body::drive_party.in_set(TnuaUserControlsSystems), walk_in::fly_party).chain(),
                recovery::pass_time,
            )
                .run_if(level_active),
        )
        .add_systems(
            FixedPostUpdate,
            (
                walk_in::run_walk_in_triggers.pipe(crate::game::world::triggers::walk),
                walk_in::move_party,
            )
                .chain()
                .after(PhysicsSystems::Writeback)
                .run_if(level_active),
        );
    }
}

/// Apply party damage and settle the target's death invariants before another
/// operation can use its formation seat. Retain resource change tracking until
/// death actually changes a seat.
pub fn damage_member(
    member: Entity,
    vitals: &mut crate::game::characters::Vitals,
    formation: &mut ResMut<formation::Formation>,
    amount: u32,
) -> u32 {
    let lost = vitals.damage(amount);
    if vitals.is_dead() && formation.row_of(member).is_some() {
        formation.unseat(member);
    }
    lost
}
