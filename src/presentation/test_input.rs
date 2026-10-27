//! Input plumbing shared by the presentation's headless tests: the plugins
//! keyboard handling needs, and real keyboard and mouse input to drive it.

use bevy::camera::NormalizedRenderTarget;
use bevy::input::{
    ButtonState,
    keyboard::{Key, KeyboardInput},
};
use bevy::picking::events::Pointer;
use bevy::picking::pointer::{Location, PointerId};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

/// An app with the plugins keyboard input and its bindings need. Callers add
/// their own extras, then call `finish` and `cleanup`.
pub fn input_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        bevy::input::InputPlugin,
        bevy::state::app::StatesPlugin,
        EnhancedInputPlugin,
        bevy::input_focus::InputFocusPlugin,
    ));
    app
}

/// Press or release a key in `window` as the window backend reports it, then
/// update once.
pub fn key(app: &mut App, window: Entity, key_code: KeyCode, pressed: bool) {
    let (logical_key, text) = match key_code {
        KeyCode::Tab => (Key::Tab, None),
        KeyCode::Enter => (Key::Enter, None),
        KeyCode::Escape => (Key::Escape, None),
        KeyCode::Backspace => (Key::Backspace, None),
        KeyCode::ShiftLeft | KeyCode::ShiftRight => (Key::Shift, None),
        KeyCode::ArrowLeft => (Key::ArrowLeft, None),
        KeyCode::ArrowRight => (Key::ArrowRight, None),
        KeyCode::ArrowUp => (Key::ArrowUp, None),
        KeyCode::ArrowDown => (Key::ArrowDown, None),
        KeyCode::F6 => (Key::F6, None),
        _ => {
            let name = format!("{key_code:?}");
            let character = name
                .strip_prefix("Key")
                .or_else(|| name.strip_prefix("Digit"))
                .unwrap_or_else(|| panic!("no logical key for {key_code:?}"))
                .to_lowercase();
            (Key::Character(character.clone().into()), Some(character.into()))
        }
    };
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key,
        state: if pressed { ButtonState::Pressed } else { ButtonState::Released },
        text: text.filter(|_| pressed),
        repeat: false,
        window,
    });
    app.update();
}

/// Press and release a key in `window`.
pub fn tap(app: &mut App, window: Entity, key_code: KeyCode) {
    key(app, window, key_code, true);
    key(app, window, key_code, false);
}

/// The mouse, over a 640×480 screen.
pub fn mouse() -> Pointer {
    Pointer::new(
        PointerId::Mouse,
        Location {
            target: NormalizedRenderTarget::None { width: 640, height: 480 },
            position: Vec2::ZERO,
        },
    )
}
