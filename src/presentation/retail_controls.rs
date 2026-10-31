//! Retail appearance and small compositions of Bevy's native controls.
use super::screen_art::Picture;
use crate::presentation::screen_art::visible_if;
use bevy::input_focus::{InputFocus, InputFocusVisible, tab_navigation::TabIndex};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, Pressed, Selected};
use bevy::ui_widgets::Button;

#[derive(Component, Clone, Default)]
#[require(Outline = Outline::new(px(1), px(1), Color::NONE))]
pub struct FocusRing;

/// Use the hover sprite for keyboard focus on controls whose retail art has
/// no separate focus outline.
#[derive(Component, Clone, Copy)]
pub struct FocusHighlight;

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

/// Change availability, keyboard eligibility and press state together.
/// This helper is for ordinary buttons; native tabs own their roving tab index.
pub fn set_enabled(commands: &mut Commands, entity: Entity, enabled: bool, disabled: bool) {
    if enabled == disabled {
        if enabled {
            commands.entity(entity).remove::<InteractionDisabled>().insert(TabIndex(0));
        } else {
            commands.entity(entity).insert((InteractionDisabled, TabIndex(-1))).remove::<Pressed>();
        }
    }
}

/// Project a screen's choice into the native widget state only when it changes.
pub fn set_selected(commands: &mut Commands, entity: Entity, selected: bool, was_selected: bool) {
    if selected != was_selected {
        if selected {
            commands.entity(entity).insert(bevy::ui::Selected);
        } else {
            commands.entity(entity).remove::<bevy::ui::Selected>();
        }
    }
}

pub fn paint_controls(
    focus: Option<Res<InputFocus>>,
    mut controls: Query<(
        Entity,
        &ControlSkin,
        &Hovered,
        Has<Pressed>,
        Has<Selected>,
        Has<InteractionDisabled>,
        Has<FocusHighlight>,
        &mut Picture,
    )>,
) {
    for (entity, skin, hovered, pressed, selected, disabled, focus_highlight, mut picture) in
        &mut controls
    {
        let frame = if disabled {
            skin.disabled
        } else if pressed || selected {
            if hovered.get() { skin.pressed_hovered } else { skin.pressed }
        } else if hovered.get()
            || (focus_highlight && focus.as_ref().and_then(|focus| focus.get()) == Some(entity))
        {
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

/// Keep a scrolling view's offset within its current content extent.
pub fn set_scroll_maximum(
    commands: &mut Commands,
    slider: Entity,
    value: &SliderValue,
    range: &SliderRange,
    maximum: f32,
) {
    if range.end() != maximum {
        commands.entity(slider).insert(SliderRange::new(0.0, maximum));
    }
    if value.0 > maximum {
        commands.entity(slider).insert(SliderValue(maximum));
    }
}

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
        visibility.set_if_neq(visible_if(extent > 0.0));
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

/// On a view that takes focus: what had it, given back when the view goes,
/// despawned or (for one that only hides) this removed. Focus goes back if
/// that is still there, and is cleared otherwise.
#[derive(Component, Debug)]
pub struct RestoreFocus(pub Option<Entity>);

pub fn give_focus_back(
    remove: On<Remove<RestoreFocus>>,
    views: Query<&RestoreFocus>,
    exists: Query<()>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    let (Ok(RestoreFocus(previous)), Some(focus)) = (views.get(remove.entity), focus.as_mut())
    else {
        return;
    };
    match previous.filter(|&entity| exists.contains(entity)) {
        Some(entity) => focus.set(entity, bevy::input_focus::FocusCause::Navigated),
        None => focus.clear(),
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
