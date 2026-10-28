//! Drawing retail screen art in its 640×480 coordinates: sprite frames at
//! their offsets, lines of text in a retail font, and buttons that show
//! their hover frame.

use bevy::prelude::*;

use crate::content::ui::SpriteSheet;
use crate::game::characters::{Race, Vitals};

/// The two empty hand sprites used by the HUD and character review.
pub fn empty_hands(race: crate::game::characters::Race) -> [&'static str; 2] {
    use crate::game::characters::Race;
    match race {
        Race::Faerie => ["fhandright", "fhandleft"],
        Race::Lizardman | Race::Dracon | Race::TRang | Race::Rapax => ["clawright", "clawleft"],
        Race::Felpurr | Race::Rawulf | Race::Mook | Race::Trynnie => ["pawright", "pawleft"],
        Race::Android => ["rhandright", "rhandleft"],
        _ => ["handright", "handleft"],
    }
}

/// The worst condition's icon name, `dead` for the dead.
pub fn condition_icon(character: &Vitals) -> Option<String> {
    if character.is_dead() {
        Some("dead".into())
    } else {
        character.worst_condition().map(|c| format!("{c:?}").to_lowercase())
    }
}

/// The dead's medium skull by race (retail `g_dead_portrait_catalog_ids`).
pub fn skull(race: Race) -> &'static str {
    match race {
        Race::Lizardman => "mskull_liz",
        Race::Dracon => "mskull_drac",
        Race::Felpurr => "mskull_felp",
        Race::Rawulf => "mskull_raw",
        Race::Mook => "mskull_mook",
        Race::Trynnie => "mskull_tryn",
        Race::TRang => "mskull_trang",
        Race::Umpani => "mskull_ump",
        Race::Rapax => "mskull_rap",
        Race::Android => "mskull_and",
        _ => "mskull",
    }
}

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

/// A rectangle of colour at `at` of `size`.
pub fn rect(at: Vec2, size: Vec2, colour: Color) -> impl Scene {
    let node = absolute(at, size);
    bsn! { node BackgroundColor({colour}) Pickable::IGNORE }
}

/// The source frame of a retained retail picture. The ImageNode owns the
/// displayed image; this keeps its source sheet alive while it loads/changes.
#[derive(Component, Clone, Default, PartialEq)]
#[require(ImageNode, Node = Node { position_type: PositionType::Absolute, ..default() }, Pickable = Pickable::IGNORE)]
pub struct Picture {
    pub sheet: Handle<SpriteSheet>,
    pub frame: usize,
    /// Hover frames keep the button's resting hit rectangle.
    pub bounds: Option<usize>,
    pub at: Vec2,
}

/// Visible region in sprite-local coordinates, including the authored offset.
/// Cropping preserves texel alignment rather than stretching a shortened node.
#[derive(Component, Clone, Copy, PartialEq)]
pub struct PictureCrop(pub Rect);
impl PictureCrop {
    pub fn horizontal(start: f32, end: f32) -> Self {
        Self(Rect::from_corners(Vec2::new(start, f32::MIN), Vec2::new(end, f32::MAX)))
    }
}

/// A retained bitmap-font label. Only its rasterized image is cached.
#[derive(Component, Clone, Default, PartialEq)]
#[require(ImageNode, Node = Node { position_type: PositionType::Absolute, ..default() }, Pickable = Pickable::IGNORE)]
pub struct BitmapText {
    pub font: Handle<SpriteSheet>,
    pub palette: Option<Handle<SpriteSheet>>,
    pub value: String,
    pub at: Vec2,
    pub width: f32,
    pub justify: bevy::text::Justify,
}

/// Spawn a retained image; callers attach screen-specific roles.
pub fn spawn_picture(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<SpriteSheet>,
    frame: usize,
    at: Vec2,
) -> Entity {
    commands.spawn((Picture { sheet, frame, at, bounds: None }, ChildOf(parent))).id()
}

/// Spawn a fully described label, including initial text, alignment and palette.
pub fn spawn_label(commands: &mut Commands, parent: Entity, text: BitmapText) -> Entity {
    commands.spawn((text, ChildOf(parent))).id()
}

pub fn picture(sheet: Handle<SpriteSheet>, frame: usize, at: Vec2) -> impl Scene {
    bsn! {
        template(move |_| Ok(Picture { sheet: sheet.clone(), frame, at, bounds: None }))
    }
}

pub fn label(font: Handle<SpriteSheet>, at: Vec2, width: f32) -> impl Scene {
    bsn! {
        template(move |_| Ok(BitmapText { font: font.clone(), value: String::new(), at, width, justify: bevy::text::Justify::Center, ..default() }))
    }
}

pub fn render_pictures(
    sheets: Res<Assets<SpriteSheet>>,
    mut removed_crops: RemovedComponents<PictureCrop>,
    mut events: MessageReader<AssetEvent<SpriteSheet>>,
    mut pictures: Query<(
        Entity,
        Ref<Picture>,
        Option<Ref<PictureCrop>>,
        &mut ImageNode,
        &mut Node,
    )>,
) {
    let removed: Vec<_> = removed_crops.read().collect();
    let changed: std::collections::HashSet<_> = events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::LoadedWithDependencies { id } => Some(*id),
            AssetEvent::Unused { .. } => None,
        })
        .collect();
    for (entity, source, crop, mut image, mut node) in &mut pictures {
        if !source.is_changed()
            && !removed.contains(&entity)
            && !crop.as_ref().is_some_and(|c| c.is_changed())
            && !changed.contains(&source.sheet.id())
        {
            continue;
        }
        let frame = sheets.get(&source.sheet).and_then(|sheet| {
            Some((
                sheet.images.get(source.frame)?,
                sheet.frames.get(source.bounds.unwrap_or(source.frame))?,
            ))
        });
        let Some((handle, frame)) = frame else {
            image.color.set_alpha(0.0);
            continue;
        };
        if image.image != *handle {
            image.image = handle.clone();
        }
        let offset = Vec2::new(f32::from(frame.offset[0]), f32::from(frame.offset[1]));
        let full =
            Rect::from_corners(offset, offset + Vec2::new(frame.width as f32, frame.height as f32));
        let shown = crop.as_ref().map_or(full, |crop| full.intersect(crop.0));
        let size = shown.size().max(Vec2::ZERO);
        image.color.set_alpha(if size.x > 0.0 && size.y > 0.0 { 1.0 } else { 0.0 });
        image.rect =
            crop.as_ref().map(|_| Rect::from_corners(shown.min - offset, shown.max - offset));
        let desired =
            (px(source.at.x + shown.min.x), px(source.at.y + shown.min.y), px(size.x), px(size.y));
        if node.left != desired.0 {
            node.left = desired.0;
        }
        if node.top != desired.1 {
            node.top = desired.1;
        }
        if node.width != desired.2 {
            node.width = desired.2;
        }
        if node.height != desired.3 {
            node.height = desired.3;
        }
    }
}

pub fn render_text(
    sheets: Res<Assets<SpriteSheet>>,
    mut events: MessageReader<AssetEvent<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    mut labels: Query<(Ref<BitmapText>, &mut ImageNode, &mut Node), Without<Picture>>,
) {
    let changed: std::collections::HashSet<_> = events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::LoadedWithDependencies { id } => Some(*id),
            AssetEvent::Unused { .. } => None,
        })
        .collect();
    for (text, mut image, mut node) in &mut labels {
        if !text.is_changed()
            && !changed.contains(&text.font.id())
            && !text.palette.as_ref().is_some_and(|palette| changed.contains(&palette.id()))
        {
            continue;
        }
        let Some(font) = sheets.get(&text.font) else {
            image.color.set_alpha(0.0);
            continue;
        };
        let raster = if let Some(palette) = &text.palette {
            let Some(palette) = sheets.get(palette) else {
                image.color.set_alpha(0.0);
                continue;
            };
            font.text_image_with_palette(&text.value, palette)
        } else {
            font.text_image(&text.value)
        };
        let size = raster.size_f32();
        image.image = images.add(raster);
        image.color.set_alpha(1.0);
        let x = match text.justify {
            bevy::text::Justify::Right if text.width > 0.0 => text.at.x + text.width - size.x,
            bevy::text::Justify::Center if text.width > 0.0 => {
                text.at.x + ((text.width - size.x) / 2.0).floor()
            }
            _ => text.at.x,
        };
        node.left = px(x);
        node.top = px(text.at.y);
        node.width = px(size.x);
        node.height = px(size.y);
    }
}

/// Fit the 640×480 screen layout into what the interface draws into (the
/// window, or an offscreen image).
pub fn scale_ui(
    interface: Query<&Camera, With<bevy::ui::IsDefaultUiCamera>>,
    mut scale: ResMut<UiScale>,
) {
    if let Some(size) = interface.single().ok().and_then(Camera::logical_target_size) {
        let fit = (size.x / SCREEN.x).min(size.y / SCREEN.y);
        if scale.0 != fit {
            scale.0 = fit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ui::Frame;

    #[test]
    fn crop_keeps_source_pixels_offsets_and_restores_full_picture() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<SpriteSheet>>()
            .add_message::<AssetEvent<SpriteSheet>>()
            .add_systems(Update, render_pictures);
        let sheet = app.world_mut().resource_mut::<Assets<SpriteSheet>>().add(SpriteSheet {
            frames: vec![Frame { offset: [2, -3], width: 125, height: 14, rgba: vec![] }],
            images: vec![Handle::default()],
            palette: None,
        });
        let entity = app
            .world_mut()
            .spawn((
                Picture { sheet, at: Vec2::new(100.0, 20.0), ..default() },
                PictureCrop::horizontal(40.0, 65.0),
            ))
            .id();
        app.update();
        let image = app.world().get::<ImageNode>(entity).unwrap();
        assert_eq!(image.rect, Some(Rect::new(38.0, 0.0, 63.0, 14.0)));
        let node = app.world().get::<Node>(entity).unwrap();
        assert_eq!(
            (node.left, node.top, node.width, node.height),
            (px(140), px(17), px(25), px(14))
        );
        app.world_mut().entity_mut(entity).insert(PictureCrop::horizontal(0.0, 0.0));
        app.update();
        assert_eq!(app.world().get::<ImageNode>(entity).unwrap().color.alpha(), 0.0);
        app.world_mut().entity_mut(entity).remove::<PictureCrop>();
        app.update();
        assert_eq!(app.world().get::<ImageNode>(entity).unwrap().rect, None);
        assert_eq!(app.world().get::<Node>(entity).unwrap().width, px(125));
    }
}
