//! Retail appearance and small compositions of Bevy's native controls.
use super::screen_art::Picture;
use bevy::input_focus::{InputFocus, InputFocusVisible, tab_navigation::TabIndex};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, Pressed, Selected};
use bevy::ui_widgets::Button;

#[derive(Component, Clone, Default)]
#[require(Outline = Outline::new(px(1), px(1), Color::NONE))]
pub struct FocusRing;

/// Explicit sheet indices: retail sheets do not share a frame ordering.
#[derive(Component, Clone, Copy, PartialEq)]
#[require(Hovered)]
pub struct ControlSkin {
    pub normal: usize,
    pub hovered: usize,
    pub pressed: usize,
    pub pressed_hovered: usize,
    pub disabled: usize,
}

impl ControlSkin {
    pub const fn new(
        normal: usize,
        hovered: usize,
        pressed: usize,
        pressed_hovered: usize,
        disabled: usize,
    ) -> Self {
        Self { normal, hovered, pressed, pressed_hovered, disabled }
    }
}

/// Keep ordinary buttons out of tab traversal while unavailable. Tabs retain
/// their native roving-tab policy and are deliberately not handled here.
pub fn sync_button_navigation(
    mut commands: Commands,
    mut buttons: Query<
        (Entity, Has<InteractionDisabled>, Has<Pressed>, &mut TabIndex),
        With<Button>,
    >,
) {
    for (entity, disabled, pressed, mut tab) in &mut buttons {
        tab.set_if_neq(TabIndex(if disabled { -1 } else { 0 }));
        if disabled && pressed {
            commands.entity(entity).remove::<Pressed>();
        }
    }
}

pub fn set_enabled(commands: &mut Commands, entity: Entity, enabled: bool, disabled: bool) {
    if enabled == disabled {
        if enabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).insert(InteractionDisabled).remove::<Pressed>();
        }
    }
}

pub fn paint_controls(
    mut controls: Query<(
        &ControlSkin,
        &Hovered,
        Has<Pressed>,
        Has<Selected>,
        Has<InteractionDisabled>,
        &mut Picture,
    )>,
) {
    for (skin, hovered, pressed, selected, disabled, mut picture) in &mut controls {
        let frame = if disabled {
            skin.disabled
        } else if pressed || selected {
            if hovered.get() { skin.pressed_hovered } else { skin.pressed }
        } else if hovered.get() {
            skin.hovered
        } else {
            skin.normal
        };
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
}

pub fn paint_focus(
    focus: Option<Res<InputFocus>>,
    visible: Option<Res<InputFocusVisible>>,
    mut controls: Query<(Entity, &mut Outline), With<FocusRing>>,
) {
    let focused = visible.filter(|v| v.0).and_then(|_| focus.as_ref().and_then(|f| f.get()));
    for (entity, mut outline) in &mut controls {
        let color =
            if focused == Some(entity) { Color::srgb(0.83, 0.68, 0.4) } else { Color::NONE };
        if outline.color != color {
            outline.color = color;
        }
    }
}

use super::screen_art::absolute;
use crate::content::ui::SpriteSheet;
use bevy::picking::events::{PointerCancel, PointerScroll};
use bevy::ui_widgets::{
    Activate, SetSliderValue, Slider, SliderOrientation, SliderPrecision, SliderRange, SliderStep,
    SliderThumb, SliderValue, SliderValueChange, slider_self_update,
};

#[derive(Component)]
pub struct StepScrollbar {
    height: f32,
}
#[derive(Component)]
pub struct ScrollArrow {
    slider: Entity,
    step: f32,
}
#[derive(Component)]
pub struct ScrollThumb(Entity);
/// Attach to the content area that forwards wheel input to its own slider.
#[derive(Component)]
pub struct ScrollWheel(pub Entity);

pub fn step_scrollbar(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<SpriteSheet>,
    at: Vec2,
    height: f32,
    maximum: f32,
) -> Entity {
    let slider = commands
        .spawn((
            StepScrollbar { height },
            // Native vertical sliders increase upwards. Mirror the track's
            // coordinate system so scroll offsets increase downwards; counter-
            // mirror the thumb artwork. Value and native input stay authoritative.
            UiTransform::from_scale(Vec2::new(1.0, -1.0)),
            Slider { orientation: SliderOrientation::Vertical, ..default() },
            SliderRange::new(0.0, maximum),
            SliderStep(1.0),
            SliderPrecision(0),
            absolute(at + Vec2::Y * 16.0, Vec2::new(16.0, height)),
            BackgroundColor(Color::BLACK),
            Pickable::default(),
            ChildOf(parent),
        ))
        .observe(slider_self_update)
        .observe(cancel_slider_drag)
        .observe(scroll_wheel)
        .id();
    commands.entity(slider).insert(ScrollWheel(slider));
    commands.spawn((
        SliderThumb,
        ScrollThumb(slider),
        UiTransform::from_scale(Vec2::new(1.0, -1.0)),
        ControlSkin::new(4, 5, 4, 5, 4),
        Picture { sheet: sheet.clone(), frame: 4, at: Vec2::ZERO, bounds: Some(4) },
        Visibility::Hidden,
        Pickable::default(),
        ChildOf(slider),
    ));
    for (step, base, y) in [(-1.0, 0, 0.0), (1.0, 8, height + 16.0)] {
        commands
            .spawn((
                Button,
                TabIndex(0),
                FocusRing,
                ScrollArrow { slider, step },
                ControlSkin::new(base, base + 1, base + 2, base + 2, base + 3),
                Picture {
                    sheet: sheet.clone(),
                    frame: base,
                    at: at + Vec2::Y * y,
                    bounds: Some(base),
                },
                Pickable::default(),
                ChildOf(parent),
            ))
            .observe(step_arrow);
    }
    slider
}

fn step_arrow(
    event: On<Activate>,
    arrows: Query<&ScrollArrow, Without<InteractionDisabled>>,
    mut commands: Commands,
) {
    if let Ok(arrow) = arrows.get(event.entity) {
        commands.trigger(SetSliderValue {
            entity: arrow.slider,
            change: SliderValueChange::RelativeStep(arrow.step),
        });
    }
}

pub fn scroll_wheel(
    mut event: On<PointerScroll>,
    targets: Query<&ScrollWheel>,
    mut commands: Commands,
) {
    if let Ok(target) = targets.get(event.entity)
        && event.y != 0.0
    {
        commands.trigger(SetSliderValue {
            entity: target.0,
            change: SliderValueChange::RelativeStep(if event.y < 0.0 { 1.0 } else { -1.0 }),
        });
        event.propagate(false);
    }
}

pub fn sync_scrollbars(
    sliders: Query<(&StepScrollbar, &SliderValue, &SliderRange)>,
    arrows: Query<(Entity, &ScrollArrow, Has<InteractionDisabled>)>,
    mut thumbs: Query<(&ScrollThumb, &mut Picture, &mut Visibility)>,
    mut commands: Commands,
) {
    for (entity, arrow, disabled) in &arrows {
        let enabled =
            sliders.get(arrow.slider).is_ok_and(|(_, _, range)| range.end() > range.start());
        set_enabled(&mut commands, entity, enabled, disabled);
    }
    for (thumb, mut picture, mut visibility) in &mut thumbs {
        let Ok((scroll, value, range)) = sliders.get(thumb.0) else { continue };
        let extent = range.end() - range.start();
        visibility.set_if_neq(if extent > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let y = if extent > 0.0 {
            ((scroll.height - 10.0) * (1.0 - ((value.0 - range.start()) / extent).clamp(0.0, 1.0)))
                .floor()
        } else {
            0.0
        };
        if picture.at.y != y {
            picture.at.y = y;
        }
    }
}

// Bevy 0.20 leaves drag state set on cancellation; release it with capture.
pub fn cancel_slider_drag(
    event: On<PointerCancel>,
    mut sliders: Query<&mut bevy::ui_widgets::SliderDragState>,
    mut captures: ResMut<bevy::picking::hover::PointerCaptureMap>,
) {
    if let Ok(mut drag) = sliders.get_mut(event.entity)
        && drag.dragging
    {
        drag.dragging = false;
        captures.release(event.pointer.id);
    }
}

/// Compose the common retail appearance with a native button. The caller owns
/// its action; ordinary pictures and tab controls can use ControlSkin separately.
pub fn image_button(sheet: Handle<SpriteSheet>, at: Vec2, skin: ControlSkin) -> impl Scene {
    bsn! {
        template(move |_| Ok(Picture { sheet: sheet.clone(), at, frame: skin.normal, bounds: Some(skin.normal) }))
        template(move |_| Ok(skin))
        Button TabIndex(0) FocusRing Pickable::default()
    }
}

/// Bevy 0.20's native button presses, and so activates, with any pointer
/// button, and stops every click it sees; retail buttons answer the primary
/// button only, and other clicks reach their ancestors as if the button were
/// not there (a popup's right-click dismissal). Every native button gets
/// that policy here, once. Gestures that use other buttons (portraits, item
/// cells) read raw pointer events, which this leaves alone.
///
/// Both work only because global observers (the native ones) run before
/// entity-scoped ones in 0.20, and propagation is decided after all of a
/// target's observers. Delete this when the native button accepts a
/// pointer-button policy.
pub fn primary_buttons_only(add: On<Add<bevy::ui_widgets::Button>>, mut commands: Commands) {
    commands.entity(add.entity).observe(primary_press).observe(pass_other_clicks);
}

fn pass_other_clicks(mut click: On<bevy::picking::events::PointerClick>) {
    if click.button != bevy::picking::pointer::PointerButton::Primary {
        click.propagate(true);
    }
}

fn primary_press(event: On<bevy::picking::events::PointerPress>, mut commands: Commands) {
    if event.button != bevy::picking::pointer::PointerButton::Primary {
        commands.entity(event.entity).remove::<Pressed>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ui_widgets::{ButtonPlugin, SliderPlugin, Tab};

    /// Secondary and middle clicks cannot activate an ordinary button and
    /// reach its ancestors instead; the primary button and the keyboard
    /// activate it.
    #[test]
    fn only_the_primary_pointer_button_activates_buttons() {
        use bevy::input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
        };
        use bevy::input_focus::{FocusedInput, InputFocus};
        use bevy::picking::{
            backend::HitData,
            events::{PointerClick, PointerPress},
            pointer::PointerButton,
        };
        #[derive(Resource, Default)]
        struct Activations(usize);
        #[derive(Resource, Default)]
        struct Passed(Vec<PointerButton>);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ButtonPlugin))
            .init_resource::<InputFocus>()
            .init_resource::<Activations>()
            .init_resource::<Passed>()
            .add_observer(primary_buttons_only);
        // Pointer events bubble through a query that includes windows.
        app.world_mut().register_component::<bevy::window::Window>();
        let parent = app
            .world_mut()
            .spawn_empty()
            .observe(|click: On<PointerClick>, mut passed: ResMut<Passed>| {
                passed.0.push(click.button);
            })
            .id();
        let button = app
            .world_mut()
            .spawn((bevy::ui_widgets::Button, ChildOf(parent)))
            .observe(|_: On<Activate>, mut activations: ResMut<Activations>| activations.0 += 1)
            .id();
        app.world_mut().flush();
        let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
        for (pointer_button, activations) in
            [(PointerButton::Secondary, 0), (PointerButton::Middle, 0), (PointerButton::Primary, 1)]
        {
            let pointer = crate::presentation::test_input::mouse();
            app.world_mut().trigger(PointerPress {
                entity: button,
                pointer: pointer.clone(),
                button: pointer_button,
                hit: hit.clone(),
                count: 1,
            });
            app.update();
            app.world_mut().trigger(PointerClick {
                entity: button,
                pointer,
                button: pointer_button,
                hit: hit.clone(),
                count: 1,
                duration: std::time::Duration::ZERO,
            });
            app.update();
            assert_eq!(app.world().resource::<Activations>().0, activations, "{pointer_button:?}");
        }
        assert_eq!(
            app.world().resource::<Passed>().0,
            [PointerButton::Secondary, PointerButton::Middle]
        );
        app.world_mut().trigger(FocusedInput::new(
            button,
            KeyboardInput {
                key_code: KeyCode::Enter,
                logical_key: Key::Enter,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            },
            Entity::PLACEHOLDER,
        ));
        app.update();
        assert_eq!(app.world().resource::<Activations>().0, 2);
    }

    #[test]
    fn independent_scrollbars_target_their_own_slider_and_disable_native_buttons() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ButtonPlugin, SliderPlugin))
            .init_resource::<UiScale>()
            .init_resource::<bevy::picking::hover::PointerCaptureMap>()
            .add_systems(Update, (sync_scrollbars, sync_button_navigation, paint_controls).chain());
        let parent = app.world_mut().spawn_empty().id();
        let (first, second) = {
            let mut commands = app.world_mut().commands();
            let first = step_scrollbar(&mut commands, parent, default(), Vec2::ZERO, 192.0, 4.0);
            let second = step_scrollbar(&mut commands, parent, default(), Vec2::ZERO, 155.0, 2.0);
            (first, second)
        };
        app.world_mut().flush();
        let tab = app.world_mut().spawn((Tab, TabIndex(-1))).id();
        app.update();
        // Exercise actual native pointer input, not just SetSliderValue.
        use bevy::picking::{
            backend::HitData,
            events::{Pointer, PointerCancel, PointerDrag, PointerDragStart},
            pointer::{Location, PointerButton, PointerId},
        };
        let thumb = app
            .world_mut()
            .query::<(Entity, &ScrollThumb)>()
            .iter(app.world())
            .find_map(|(e, t)| (t.0 == first).then_some(e))
            .unwrap();
        let scale = app.world().get::<UiTransform>(first).unwrap().scale;
        app.world_mut().entity_mut(first).insert((
            ComputedNode { size: Vec2::new(16.0, 192.0), inverse_scale_factor: 1.0, ..default() },
            UiGlobalTransform::from_scale(scale),
        ));
        app.world_mut()
            .entity_mut(thumb)
            .insert(ComputedNode { size: Vec2::new(16.0, 10.0), ..default() });
        let pointer = Pointer::new(
            PointerId::Mouse,
            Location {
                target: bevy::camera::NormalizedRenderTarget::None { width: 640, height: 480 },
                position: Vec2::ZERO,
            },
        );
        let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
        app.world_mut().trigger(PointerDragStart {
            entity: first,
            pointer: pointer.clone(),
            button: PointerButton::Primary,
            hit: hit.clone(),
        });
        app.world_mut().trigger(PointerDrag {
            entity: first,
            pointer: pointer.clone(),
            button: PointerButton::Primary,
            distance: Vec2::new(0.0, 91.0),
            delta: Vec2::new(0.0, 91.0),
        });
        app.update();
        assert_eq!(
            app.world().get::<SliderValue>(first).unwrap().0,
            2.0,
            "dragging down advances rows"
        );
        assert_eq!(app.world().get::<Picture>(thumb).unwrap().at.y, 91.0);
        app.world_mut().trigger(PointerCancel { entity: first, pointer: pointer.clone(), hit });
        app.update();
        assert!(!app.world().get::<bevy::ui_widgets::SliderDragState>(first).unwrap().dragging);
        assert!(
            !app.world()
                .resource::<bevy::picking::hover::PointerCaptureMap>()
                .0
                .contains_key(&pointer.id)
        );
        let down = app
            .world_mut()
            .query::<(Entity, &ScrollArrow)>()
            .iter(app.world())
            .find_map(|(e, a)| (a.slider == second && a.step > 0.0).then_some(e))
            .unwrap();
        app.world_mut().trigger(Activate { entity: down });
        app.update();
        assert_eq!(app.world().get::<SliderValue>(first).unwrap().0, 2.0);
        assert_eq!(app.world().get::<SliderValue>(second).unwrap().0, 1.0);
        app.world_mut().entity_mut(second).insert(SliderRange::new(0.0, 0.0));
        app.world_mut().entity_mut(down).insert(Pressed);
        app.update();
        assert!(app.world().get::<InteractionDisabled>(down).is_some());
        assert!(app.world().get::<Pressed>(down).is_none());
        assert_eq!(app.world().get::<TabIndex>(down).unwrap().0, -1);
        assert_eq!(app.world().get::<Picture>(down).unwrap().frame, 11);
        assert_eq!(
            app.world().get::<TabIndex>(tab).unwrap().0,
            -1,
            "native tabs own their navigation policy"
        );
    }
}
