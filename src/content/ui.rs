//! Retail UI art: sprite sheets and bitmap fonts.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::retail::stci::{StciFrame, render_text};

/// A retail sprite sheet (`.sti`): button states, chrome, or a font's
/// glyphs.
#[derive(Asset, TypePath, Debug)]
pub struct SpriteSheet {
    /// The decoded frames, kept for drawing text.
    pub frames: Vec<StciFrame>,
    pub images: Vec<Handle<Image>>,
}

impl SpriteSheet {
    /// Draw a line of text with this sheet as a font.
    pub fn text_image(&self, text: &str) -> Image {
        let (width, height, rgba) = render_text(&self.frames, text);
        ui_image(width, height, rgba)
    }
}

/// An image for pixel art: unfiltered, so scaled UI stays crisp.
pub fn ui_image(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Retail text by index: the game's string table, or a level's messages.
#[derive(Asset, TypePath, Debug, Default)]
pub struct TextTable(pub Vec<Option<String>>);

impl TextTable {
    pub fn get(&self, index: usize) -> Option<&str> {
        self.0.get(index)?.as_deref()
    }
}
