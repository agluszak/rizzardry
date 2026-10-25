//! Drawing retail screen art in its 640×480 coordinates: sprite frames at
//! their offsets, lines of text in a retail font, and buttons that show
//! their hover frame.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use crate::content::ui::SpriteSheet;

/// Retail's screen, in its own pixels.
pub const SCREEN: Vec2 = Vec2::new(640.0, 480.0);

/// A node at `at` of `size`, in retail screen pixels.
pub fn absolute(at: Vec2, size: Vec2) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(at.x),
        top: Val::Px(at.y),
        width: Val::Px(size.x),
        height: Val::Px(size.y),
        ..default()
    }
}

/// Fills what it is drawn into and centres its children: holds a retail
/// screen, scaled as the interface is.
pub fn centred() -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        }
    }
}

/// A retail screen: 640×480, its children placed in its pixels.
pub fn screen() -> impl Scene {
    bsn! { Node { width: px(SCREEN.x), height: px(SCREEN.y) } }
}

/// A sheet's frame drawn with its top-left at `at`, shifted by the frame's
/// own offset as retail's sprite drawing does. It does not catch the
/// pointer; a button adds `Pickable::default()`.
pub fn sprite(sheet: &SpriteSheet, frame: usize, at: Vec2) -> Option<impl Scene + use<>> {
    let (image, info) = (sheet.images.get(frame)?.clone(), sheet.frames.get(frame)?);
    let offset = Vec2::new(f32::from(info.offset[0]), f32::from(info.offset[1]));
    let node = absolute(at + offset, Vec2::new(info.width as f32, info.height as f32));
    Some(bsn! { ImageNode { image: {image} } node Pickable::IGNORE })
}

/// A line of text, centred in a box `width` wide (or left-aligned when 0).
pub fn text(
    font: &SpriteSheet,
    images: &mut Assets<Image>,
    line: &str,
    at: Vec2,
    width: f32,
) -> impl Scene + use<> {
    let image = font.text_image(line);
    let size = image.size_f32();
    let x = if width > 0.0 { at.x + ((width - size.x) / 2.0).floor() } else { at.x };
    let (image, node) = (images.add(image), absolute(Vec2::new(x, at.y), size));
    bsn! { ImageNode { image: {image} } node Pickable::IGNORE }
}

/// A rectangle of colour at `at` of `size`.
pub fn rect(at: Vec2, size: Vec2, colour: Color) -> impl Scene {
    let node = absolute(at, size);
    bsn! { node BackgroundColor({colour}) Pickable::IGNORE }
}

/// A button's look while hovered (retail's buttons have a frame for it).
#[derive(Component, Clone, FromTemplate)]
#[require(Hovered)]
pub struct HoverArt {
    pub normal: Handle<Image>,
    pub hovered: Handle<Image>,
}

/// Show frame `hovered` of `sheet` while the pointer is over the button,
/// `normal` otherwise.
pub fn hover_art(sheet: &SpriteSheet, normal: usize, hovered: usize) -> impl Scene + use<> {
    let image = |frame: usize| sheet.images.get(frame).cloned().unwrap_or_default();
    let (normal, hovered) = (image(normal), image(hovered));
    bsn! { HoverArt { normal: {normal}, hovered: {hovered} } }
}

/// Show each button's hover frame while the pointer is over it.
pub fn paint_hover_art(
    mut buttons: Query<(&Hovered, &HoverArt, &mut ImageNode), Changed<Hovered>>,
) {
    for (hovered, art, mut image) in &mut buttons {
        image.image = if hovered.get() { art.hovered.clone() } else { art.normal.clone() };
    }
}
