//! The party's roster, formation and spatial body.

pub mod body;

use avian3d::prelude::Gravity;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use crate::game::world::level::build_level_geometry;

pub struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            TnuaControllerPlugin::<body::PartyMotion>::new(FixedUpdate),
            TnuaAvian3dPlugin::new(FixedUpdate),
        ))
        .insert_resource(Gravity(Vec3::NEG_Y * body::GRAVITY))
        .add_input_context::<body::Exploration>()
        .add_systems(Startup, body::spawn_exploration_controls)
        .add_systems(
            Update,
            (
                body::spawn_party_on_ready.after(build_level_geometry),
                (body::look_around, body::aim_camera).chain(),
            ),
        )
        // Tnua and Avian step on the fixed timestep; movement input feeds
        // them on the same step.
        .add_systems(FixedUpdate, body::drive_party.in_set(TnuaUserControlsSystems));
    }
}
