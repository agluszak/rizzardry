//! The party's roster, formation and spatial body.

pub mod body;

use avian3d::prelude::Gravity;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use crate::game::world::level::spawn_active_level;

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
                body::spawn_party_on_ready.after(spawn_active_level),
                body::move_party.in_set(TnuaUserControlsSystems),
            ),
        );
    }
}
