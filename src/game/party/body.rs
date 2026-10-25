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

use crate::content::level::LevelGeometry;
use bevy::ecs::system::SystemParam;

use crate::retail::space::UNITS_PER_METRE as U;

// Retail motion values (Whizzardry `party_camera.cpp`, `collision.cpp`,
// `world.cpp`). Retail velocities are per 0.1 s tick. Retail collides a
// sphere of 500 units whose centre is 500 above the feet; the eye is 1000
// above the feet.
//
// Tnua floats the body above the ground instead of resting a collider on it.
// The body's origin is the retail sphere centre, held FLOAT_HEIGHT above the
// ground, and its collider is that sphere, a little smaller so it clears the
// ground. A sphere's curved front rides over step edges; a flat-bottomed
// cylinder of the same reach caught on stairs rising inside its radius (it
// never left the upper monastery's MO202 entrance).
pub const RADIUS: f32 = 500.0 / U;
pub const FLOAT_HEIGHT: f32 = 500.0 / U;
pub const EYE_ABOVE_CENTRE: f32 = 500.0 / U;
/// The eye above the feet.
pub const EYE_HEIGHT: f32 = 1000.0 / U;
pub const WALK_SPEED: f32 = 3750.0 / U;
const RUN_SPEED: f32 = 7500.0 / U;
/// Maximum yaw speed, 0.349 rad per tick.
const TURN_SPEED: f32 = 3.49;
/// 187.5 units per tick².
pub const GRAVITY: f32 = 18750.0 / U;
/// Retail caps a tick's motion at 3000 units (Whizzardry `collision.cpp`);
/// walking never comes near it, so in practice it limits falls.
const MAX_SPEED: f32 = 30000.0 / U;
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

impl PartyBody {
    /// The way the party faces, level.
    pub fn forward(&self) -> Vec3 {
        Quat::from_rotation_y(self.yaw) * Vec3::NEG_Z
    }

    /// The party's eye, over the centre of its body at `position`.
    pub fn eye(position: &Position) -> Vec3 {
        position.0 + Vec3::Y * EYE_ABOVE_CENTRE
    }

    /// Turn to face along `direction`, level.
    pub fn face(&mut self, direction: Vec3) {
        let facing = Transform::default().looking_to(direction.with_y(0.0), Vec3::Y);
        (self.yaw, _, _) = facing.rotation.to_euler(EulerRot::YXZ);
    }
}

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

/// Use the prop or object in front of the party.
#[derive(InputAction)]
#[action_output(bool)]
pub struct Interact;

pub fn spawn_exploration_controls(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Exploration
        Actions<Exploration> [
            // Retail's keys (data/strings/mgskeyboard.ini): the arrows walk
            // and turn, Num0 and Num. strafe; letters are commands.
            Action::<Walk>::new()
            DeadZone
            Bindings [
                Binding(KeyCode::NumpadDecimal)
                --
                Binding(KeyCode::Numpad0)
                ~Negate::all()
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
            --
            // Retail interacts by clicking in the view only.
            Action::<Interact>::new()
            Bindings [
                Binding(MouseButton::Left)
            ]
            --
            // Retail's keys are in data/strings/mgskeyboard.ini: C camps.
            Action::<crate::game::camp::Rest>::new()
            Bindings [
                Binding(KeyCode::KeyC)
            ]
            --
            Action::<crate::game::world::search::SearchKey>::new()
            Bindings [
                Binding(KeyCode::KeyS)
            ]
        ]
    });
}

/// What placing the party needs (part of a level's activation).
#[derive(SystemParam)]
pub struct PartyPlacement<'w, 's> {
    party: Query<
        'w,
        's,
        (
            Entity,
            &'static mut PartyBody,
            &'static mut Position,
            &'static mut Transform,
            &'static mut LinearVelocity,
        ),
    >,
    start: Option<Res<'w, PartyStart>>,
    motion_configs: ResMut<'w, Assets<PartyMotionConfig>>,
}

impl PartyPlacement<'_, '_> {
    /// Put the party in a level being activated: spawn it in the first,
    /// move it into the next and resume its body, suspended for the trip.
    ///
    /// Retail enters a level at an entrance trigger (e.g. `MO100`) and falls
    /// back to an eye position of (0, 1000, 0) when there is none. Entrance
    /// triggers the party arrives inside do not fire until it leaves them.
    pub fn place(&mut self, commands: &mut Commands, level: &LevelGeometry) {
        let start = self.start.as_ref().map(|s| (**s).clone()).unwrap_or_default();
        let entrance = start.entrance.as_deref().and_then(|name| {
            let found = level.entrances.iter().find(|e| e.name.eq_ignore_ascii_case(name));
            if found.is_none() {
                warn!("entrance {name} is missing; using the retail fallback position");
            }
            found
        });
        let eye = start.eye.or(entrance.map(|e| e.eye)).unwrap_or(Vec3::new(0.0, 1000.0 / U, 0.0));
        let yaw = start.yaw.or(entrance.map(|e| e.yaw)).unwrap_or_default();
        let position = eye - Vec3::Y * EYE_ABOVE_CENTRE;
        if let Ok((entity, mut body, mut at, mut transform, mut velocity)) = self.party.single_mut()
        {
            body.yaw = yaw;
            at.0 = position;
            transform.translation = position;
            velocity.0 = Vec3::ZERO;
            commands.entity(entity).remove::<RigidBodyDisabled>();
            return;
        }
        spawn_party(commands, &start, yaw, position, &mut self.motion_configs);
    }

    /// Let the party's body move again where it stands (travel that did
    /// not happen).
    pub fn resume(&self, commands: &mut Commands) {
        for (entity, ..) in &self.party {
            commands.entity(entity).remove::<RigidBodyDisabled>();
        }
    }
}

fn spawn_party(
    commands: &mut Commands,
    start: &PartyStart,
    yaw: f32,
    position: Vec3,
    motion_configs: &mut Assets<PartyMotionConfig>,
) {
    commands.spawn((
        Name::new("Party"),
        PartyBody { yaw, pitch: start.pitch.clamp(-MAX_PITCH, MAX_PITCH) },
        Transform::from_translation(position),
        Visibility::default(),
        RigidBody::Dynamic,
        Collider::sphere(RADIUS - 0.08),
        LockedAxes::ROTATION_LOCKED,
        MaxLinearSpeed(MAX_SPEED),
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
    ));
}

/// Where the party enters the next level that becomes ready: an entrance
/// trigger, unless an explicit eye position (metres) or facing overrides it.
#[derive(Resource, Debug, Clone, Default)]
pub struct PartyStart {
    pub entrance: Option<String>,
    pub eye: Option<Vec3>,
    pub yaw: Option<f32>,
    pub pitch: f32,
}

/// Mouse look: a per-frame input delta, applied every frame.
pub fn look_around(
    look: Single<&Action<Look>>,
    free_look: Single<&Action<FreeLook>>,
    mut party: Query<&mut PartyBody>,
) {
    let Ok(mut body) = party.single_mut() else { return };
    if ***free_look {
        body.yaw -= look.x * LOOK_SENSITIVITY;
        body.pitch = (body.pitch - look.y * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }
}

/// Turn and walk on the fixed simulation step, feeding Tnua.
pub fn drive_party(
    time: Res<Time>,
    walk: Single<&Action<Walk>>,
    turn: Single<&Action<Turn>>,
    run: Single<&Action<Run>>,
    mut party: Query<(&mut PartyBody, &mut TnuaController<PartyMotion>)>,
    freedom: crate::game::screen::Freedom,
) {
    let Ok((mut body, mut controller)) = party.single_mut() else { return };
    // A party held by a screen, a camp or a question stands still.
    let free = freedom.free();
    if free {
        body.yaw -= ***turn * TURN_SPEED * time.delta_secs();
    }
    // The walk config's speed is the run speed; walking scales the input.
    let input = if free { walk.clamp_length_max(1.0) } else { Vec2::ZERO };
    let fraction = if ***run { 1.0 } else { WALK_SPEED / RUN_SPEED };
    controller.initiate_action_feeding();
    controller.basis = TnuaBuiltinWalk {
        desired_motion: Quat::from_rotation_y(body.yaw)
            * Vec3::new(input.x, 0.0, -input.y)
            * fraction,
        desired_forward: None,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_a_direction_turns_the_view_that_way() {
        let mut body = PartyBody::default();
        for direction in [Vec3::NEG_X, Vec3::Z, Vec3::new(1.0, 5.0, -1.0)] {
            body.face(direction);
            let view = Quat::from_rotation_y(body.yaw) * Vec3::NEG_Z;
            assert!(view.abs_diff_eq(direction.with_y(0.0).normalize(), 1e-5), "{direction}");
        }
    }
}
