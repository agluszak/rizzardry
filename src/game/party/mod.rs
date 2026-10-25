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
use crate::game::world::level::build_level_geometry;
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
        .init_resource::<formation::Formation>()
        .add_message::<camp::CampNews>()
        .add_message::<camp::CampRequest>()
        .add_systems(Startup, (body::spawn_exploration_controls, roster::spawn_test_party))
        .add_systems(
            Update,
            (
                body::place_party_on_ready.after(build_level_geometry),
                (body::look_around, body::aim_camera).chain(),
                body::interact,
                crate::game::world::items::click_items_in_view,
                roster::give_starting_equipment,
                formation::seat_party,
                (camp::request_camp, camp::force_camp, camp::cancel_camp, camp::advance_camp)
                    .chain(),
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
            ),
        )
        .add_systems(
            FixedPostUpdate,
            (walk_in::run_walk_in_triggers, walk_in::move_party)
                .chain()
                .after(PhysicsSystems::Writeback),
        );
    }
}
