//! Device bindings translate input into gameplay requests and movement intent.
use crate::game::camp::CampRequest;
use crate::game::party::body::{PartyLook, PartyMovement};
use crate::game::world::search::ToggleSearch;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

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
            Action::<Rest>::new()
            Bindings [
                Binding(KeyCode::KeyC)
            ]
            --
            Action::<SearchKey>::new()
            Bindings [
                Binding(KeyCode::KeyS)
            ]
        ]
    });
}

#[derive(InputAction)]
#[action_output(bool)]
pub struct Rest;

#[derive(InputAction)]
#[action_output(bool)]
pub struct SearchKey;

/// Publish the exploration intents, rechecked after this frame's input has
/// been handled: a key that opens a screen this frame leaves the context
/// active until the next, but the party must not walk behind the screen.
pub fn read_controls(
    input: super::screen::WorldInput,
    walk: Single<&Action<Walk>>,
    turn: Single<&Action<Turn>>,
    run: Single<&Action<Run>>,
    look: Single<&Action<Look>>,
    free_look: Single<&Action<FreeLook>>,
    rest: Single<&ActionEvents, With<Action<Rest>>>,
    search: Single<&ActionEvents, With<Action<SearchKey>>>,
    preferences: Option<Res<crate::settings::Preferences>>,
    mut movement: Single<&mut PartyMovement>,
    mut looks: MessageWriter<PartyLook>,
    mut camps: MessageWriter<CampRequest>,
    mut searches: MessageWriter<ToggleSearch>,
) {
    if !input.free() {
        movement.set_if_neq(PartyMovement::default());
        return;
    }
    movement.set_if_neq(PartyMovement { walk: ***walk, turn: ***turn, running: ***run });
    if ***free_look {
        let mut delta = -***look * 0.003;
        if preferences.is_some_and(|p| p.invert_mouse_y) {
            delta.y = -delta.y;
        }
        looks.write(PartyLook(delta));
    }
    if rest.contains(ActionEvents::START) {
        camps.write(CampRequest);
    }
    if search.contains(ActionEvents::START) {
        searches.write(ToggleSearch);
    }
}
