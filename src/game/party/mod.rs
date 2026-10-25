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
        .init_resource::<crate::game::screen::Screen>()
        .init_resource::<formation::Formation>()
        .add_message::<camp::CampNews>()
        .add_message::<camp::CampRequest>()
        .add_systems(Startup, (body::spawn_exploration_controls, roster::spawn_test_party))
        .add_systems(
            Update,
            (
                roster::give_starting_equipment,
                formation::seat_party,
                (
                    (
                        (body::look_around, body::aim_camera).chain(),
                        body::interact,
                        crate::game::world::items::click_items_in_view,
                    )
                        .run_if(party_free),
                    (
                        camp::request_camp.run_if(party_free),
                        camp::force_camp,
                        camp::cancel_camp,
                        camp::advance_camp,
                    )
                        .chain(),
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
            (walk_in::run_walk_in_triggers, walk_in::move_party)
                .chain()
                .after(PhysicsSystems::Writeback)
                .run_if(level_active),
        );
    }
}
