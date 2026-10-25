//! Drawing retail screen art in its 640×480 coordinates: sprite frames at
//! their offsets, and lines of text in a retail font.

use bevy::prelude::*;

use crate::content::ui::SpriteSheet;

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

/// A sheet's frame drawn with its top-left at `at`, shifted by the frame's
/// own offset as retail's sprite drawing does.
pub fn sprite(sheet: &SpriteSheet, frame: usize, at: Vec2) -> Option<(ImageNode, Node, Pickable)> {
    let (image, info) = (sheet.images.get(frame)?, sheet.frames.get(frame)?);
    let offset = Vec2::new(f32::from(info.offset[0]), f32::from(info.offset[1]));
    let size = Vec2::new(info.width as f32, info.height as f32);
    Some((ImageNode::new(image.clone()), absolute(at + offset, size), Pickable::IGNORE))
}

/// A line of text, centred in a box `width` wide (or left-aligned when 0).
pub fn text(
    font: &SpriteSheet,
    images: &mut Assets<Image>,
    line: &str,
    at: Vec2,
    width: f32,
) -> (ImageNode, Node, Pickable) {
    let image = font.text_image(line);
    let size = image.size_f32();
    let x = if width > 0.0 { at.x + ((width - size.x) / 2.0).floor() } else { at.x };
    (ImageNode::new(images.add(image)), absolute(Vec2::new(x, at.y), size), Pickable::IGNORE)
}
