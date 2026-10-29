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
    preferences: Option<Res<super::settings::Preferences>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_produce_gameplay_requests_and_clear_movement_when_disabled() {
        use crate::presentation::screen::Screen;
        let mut app = crate::presentation::test_input::input_app();
        app.init_state::<Screen>()
            .add_input_context::<Exploration>()
            .add_message::<PartyLook>()
            .add_message::<CampRequest>()
            .add_message::<ToggleSearch>()
            .add_systems(Startup, spawn_exploration_controls)
            .add_systems(PreUpdate, read_controls.after(EnhancedInputSystems::Apply));
        let party = app.world_mut().spawn(crate::game::party::roster::Party).id();
        app.world_mut().spawn(crate::game::world::level::Level::test(default()));
        app.finish();
        app.cleanup();
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            for key in [KeyCode::ArrowUp, KeyCode::ShiftLeft, KeyCode::KeyC, KeyCode::KeyS] {
                keys.press(key);
            }
        }
        app.update();
        let movement = app.world().get::<PartyMovement>(party).unwrap();
        assert_eq!(movement.walk, Vec2::Y);
        assert!(movement.running);
        assert_eq!(app.world().resource::<Messages<CampRequest>>().len(), 1);
        assert_eq!(app.world().resource::<Messages<ToggleSearch>>().len(), 1);
        let controls = app
            .world_mut()
            .query_filtered::<Entity, With<Exploration>>()
            .single(app.world())
            .unwrap();
        // A screen opened by this frame's input, with the walk key still
        // held and the context not yet switched off, stops the walk at once.
        app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Inventory);
        app.update();
        let movement = app.world().get::<PartyMovement>(party).unwrap();
        assert_eq!((movement.walk, movement.running), (Vec2::ZERO, false));
        assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Inventory);
        app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::World);
        app.update();
        app.update();
        assert_eq!(app.world().get::<PartyMovement>(party).unwrap().walk, Vec2::Y);
        app.world_mut().entity_mut(controls).insert(ContextActivity::<Exploration>::new(false));
        app.update();
        let movement = app.world().get::<PartyMovement>(party).unwrap();
        assert_eq!(movement.walk, Vec2::ZERO);
        assert!(!movement.running);
    }
}
