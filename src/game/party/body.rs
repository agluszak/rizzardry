//! The party's single spatial body and first-person view.
//!
//! The party moves as one body driven by a Tnua walk controller. Individual
//! members have no world collider; formation-derived positions are a rules
//! concern added later.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_tnua::builtins::{TnuaBuiltinWalk, TnuaBuiltinWalkConfig};
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use crate::game::world::STEP_HEIGHT;
use crate::game::world::level::LevelReady;
use crate::retail::space::UNITS_PER_METRE as U;

// Retail motion values (Whizzardry `party_camera.cpp`, `collision.cpp`,
// `world.cpp`). Retail velocities are per 0.1 s tick. Retail collides a
// sphere of 500 units whose centre is 500 above the feet; the eye is 1000
// above the feet.
//
// Tnua floats the body above the ground instead of resting a collider on it.
// The body's origin is the retail sphere centre, held FLOAT_HEIGHT above the
// ground. Its collider keeps the retail 500-unit horizontal reach but is a
// cylinder raised STEP_HEIGHT off the ground so that steps reach the
// floating spring rather than the collider.
const RADIUS: f32 = 500.0 / U;
pub const FLOAT_HEIGHT: f32 = 500.0 / U;
const EYE_ABOVE_CENTRE: f32 = 500.0 / U;
const WALK_SPEED: f32 = 3750.0 / U;
const RUN_SPEED: f32 = 7500.0 / U;
/// Maximum yaw speed, 0.349 rad per tick.
const TURN_SPEED: f32 = 3.49;
/// 187.5 units per tick². Retail also caps falls at 3000 units per tick; that
/// cap is not applied yet.
pub const GRAVITY: f32 = 18750.0 / U;
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_4;
const LOOK_SENSITIVITY: f32 = 0.003;

/// Movement the party can perform: walking only. Wizardry has no jumping.
#[derive(TnuaScheme)]
#[scheme(basis = TnuaBuiltinWalk)]
pub enum PartyMotion {}

/// The party's spatial body. Exactly one exists while a level is active.
/// Its own rotation is locked; facing lives here and on the camera.
#[derive(Component, Debug, Default)]
pub struct PartyBody {
    pub yaw: f32,
    pub pitch: f32,
}

/// The first-person camera attached to the party body.
#[derive(Component, Debug)]
pub struct PartyCamera;

/// Input context while exploring the world.
#[derive(Component, Default, Clone)]
pub struct Exploration;

/// Forward/back (y) and strafe (x).
#[derive(InputAction)]
#[action_output(Vec2)]
pub struct Walk;

/// Turn right (positive) or left.
#[derive(InputAction)]
#[action_output(f32)]
pub struct Turn;

/// Mouse motion; applied while `FreeLook` is held.
#[derive(InputAction)]
#[action_output(Vec2)]
pub struct Look;

#[derive(InputAction)]
#[action_output(bool)]
pub struct FreeLook;

#[derive(InputAction)]
#[action_output(bool)]
pub struct Run;

pub fn spawn_exploration_controls(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Exploration
        Actions<Exploration> [
            Action::<Walk>::new()
            DeadZone
            Bindings [
                {preset::cardinal::wasd_keys()}
                --
                Binding(KeyCode::ArrowUp)
                ~SwizzleAxis::YXZ
                --
                Binding(KeyCode::ArrowDown)
                ~Negate::all()
                ~SwizzleAxis::YXZ
                --
                {preset::axial::left_stick()}
            ]
            --
            Action::<Turn>::new()
            Bindings [
                {preset::bidirectional::new(KeyCode::ArrowRight, KeyCode::ArrowLeft)}
                --
                {preset::bidirectional::new(KeyCode::KeyE, KeyCode::KeyQ)}
                --
                Binding(GamepadAxis::RightStickX)
            ]
            --
            Action::<Look>::new()
            Bindings [
                Binding::mouse_motion()
            ]
            --
            Action::<FreeLook>::new()
            Bindings [
                Binding(MouseButton::Right)
            ]
            --
            Action::<Run>::new()
            Bindings [
                Binding(KeyCode::ShiftLeft)
                --
                Binding(KeyCode::ShiftRight)
            ]
        ]
    });
}

/// Place the party once the level exists.
///
/// Retail enters a level at its entrance trigger (e.g. `MO100`) and falls
/// back to an eye position of (0, 1000, 0) when there is none. Triggers are
/// not decoded yet, so the fallback is the default.
pub fn spawn_party_on_ready(
    mut commands: Commands,
    mut ready: MessageReader<LevelReady>,
    start: Option<Res<PartyStart>>,
    mut motion_configs: ResMut<Assets<PartyMotionConfig>>,
) {
    if ready.read().last().is_none() {
        return;
    }
    let start = start.map(|s| *s).unwrap_or_default();
    let eye = start.eye.unwrap_or(Vec3::new(0.0, 1000.0 / U, 0.0));
    let position = eye - Vec3::Y * EYE_ABOVE_CENTRE;
    commands
        .spawn((
            Name::new("Party"),
            PartyBody { yaw: start.yaw, ..default() },
            Transform::from_translation(position),
            Visibility::default(),
            RigidBody::Dynamic,
            Collider::cylinder(RADIUS, 2.0 * (FLOAT_HEIGHT - STEP_HEIGHT)),
            LockedAxes::ROTATION_LOCKED,
            TnuaController::<PartyMotion>::default(),
            TnuaConfig::<PartyMotion>(motion_configs.add(PartyMotionConfig {
                basis: TnuaBuiltinWalkConfig {
                    speed: RUN_SPEED,
                    float_height: FLOAT_HEIGHT,
                    max_slope: 50f32.to_radians(),
                    free_fall_extra_gravity: 0.0,
                    ..default()
                },
            })),
            TnuaAvian3dSensorShape(Collider::cylinder(RADIUS * 0.9, 0.0)),
        ))
        .with_children(|party| {
            party.spawn((
                Name::new("Party camera"),
                PartyCamera,
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: 70f32.to_radians(),
                    // Retail clip range 250..75000 units.
                    near: 250.0 / U,
                    far: 75000.0 / U,
                    ..default()
                }),
                Transform::from_xyz(0.0, EYE_ABOVE_CENTRE, 0.0),
            ));
        });
}

/// Where the party enters: an explicit eye position in metres, if any, and
/// the initial yaw.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PartyStart {
    pub eye: Option<Vec3>,
    pub yaw: f32,
}

pub fn move_party(
    time: Res<Time>,
    walk: Single<&Action<Walk>>,
    turn: Single<&Action<Turn>>,
    look: Single<&Action<Look>>,
    free_look: Single<&Action<FreeLook>>,
    run: Single<&Action<Run>>,
    mut party: Query<(&mut PartyBody, &mut TnuaController<PartyMotion>)>,
    mut camera: Query<&mut Transform, With<PartyCamera>>,
) {
    let Ok((mut body, mut controller)) = party.single_mut() else { return };
    let dt = time.delta_secs();

    body.yaw -= ***turn * TURN_SPEED * dt;
    if ***free_look {
        body.yaw -= look.x * LOOK_SENSITIVITY;
        body.pitch = (body.pitch - look.y * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }
    let facing = Quat::from_rotation_y(body.yaw);
    if let Ok(mut camera) = camera.single_mut() {
        camera.rotation = facing * Quat::from_rotation_x(body.pitch);
    }

    // The walk config's speed is the run speed; walking scales the input.
    let input = walk.clamp_length_max(1.0);
    let fraction = if ***run { 1.0 } else { WALK_SPEED / RUN_SPEED };
    controller.initiate_action_feeding();
    controller.basis = TnuaBuiltinWalk {
        desired_motion: facing * Vec3::new(input.x, 0.0, -input.y) * fraction,
        desired_forward: None,
    };
}
