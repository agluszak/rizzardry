//! Retail UI art: sprite sheets and bitmap fonts.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// A retail sprite sheet (`.sti`): button states, chrome, or a font's
/// glyphs.
#[derive(Asset, TypePath, Debug)]
pub struct SpriteSheet {
    /// The decoded frames, kept for drawing text.
    pub frames: Vec<Frame>,
    pub images: Vec<Handle<Image>>,
    /// Original indexed palette, used by retail font colour variants.
    pub palette: Option<Vec<[u8; 3]>>,
}

impl SpriteSheet {
    /// How wide a line of text is in this font.
    pub fn text_width(&self, text: &str) -> u32 {
        text_width(&self.frames, text)
    }

    /// Draw text using a retail replacement palette.
    pub fn text_image_with_palette(&self, text: &str, palette: &SpriteSheet) -> Image {
        let (width, height, mut rgba) = render_text(&self.frames, text);
        if let (Some(source), Some(target)) = (&self.palette, &palette.palette) {
            let mut colours = std::collections::HashMap::new();
            for (&from, &to) in source.iter().zip(target) {
                colours.entry(from).or_insert(to);
            }
            for pixel in rgba.as_chunks_mut::<4>().0.iter_mut().filter(|pixel| pixel[3] != 0) {
                if let Some(colour) = colours.get(&[pixel[0], pixel[1], pixel[2]]) {
                    pixel[..3].copy_from_slice(colour);
                }
            }
        }
        ui_image(width, height, rgba)
    }

    /// Draw a line of text with this sheet as a font.
    pub fn text_image(&self, text: &str) -> Image {
        let (width, height, rgba) = render_text(&self.frames, text);
        ui_image(width, height, rgba)
    }
}

/// One image of a sheet (a retail `.sti` frame), as RGBA8 rows. A sheet's frames are drawn at their
/// offsets from the sheet's origin.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub offset: [i16; 2],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The character each font glyph draws, by glyph index (`Font.cpp:21-124`).
pub fn glyph_index(c: char) -> Option<usize> {
    const PUNCTUATION: &str = "!@#$%^&*()-_+=|\\{}[]:;\"'<>,.?/ ";
    const ACCENTED: [u8; 56] = [
        193, 192, 193, 196, 195, 197, 199, 201, 200, 202, 203, 205, 204, 206, 207, 209, 211, 210,
        212, 214, 213, 216, 218, 217, 219, 220, 221, 225, 224, 226, 228, 227, 229, 231, 233, 232,
        234, 235, 237, 236, 238, 239, 241, 243, 242, 244, 246, 245, 248, 250, 249, 251, 252, 254,
        255, 223,
    ];
    match c {
        'A'..='Z' => Some(c as usize - 'A' as usize),
        'a'..='z' => Some(26 + c as usize - 'a' as usize),
        '0'..='9' => Some(52 + c as usize - '0' as usize),
        '¿' => Some(250),
        '¡' => Some(251),
        _ => PUNCTUATION
            .chars()
            .position(|p| p == c)
            .map(|i| 62 + i)
            .or_else(|| ACCENTED.iter().position(|&a| a as u32 == c as u32).map(|i| 93 + i)),
    }
}

/// How wide a line of text is in a font, in pixels.
pub fn text_width(font: &[Frame], text: &str) -> u32 {
    let glyph =
        |c: char| glyph_index(c).filter(|&i| i < font.len()).or(glyph_index('?')).map(|i| &font[i]);
    text.chars()
        .filter_map(glyph)
        .map(|g| (g.width as i32 + g.offset[0] as i32).max(0) as u32)
        .sum()
}

/// Draw text with a font sheet into an RGBA8 image, left to right with no
/// kerning: each glyph advances by its width plus x offset, and a line is as
/// tall as `A`. Unknown characters draw as `?`.
pub fn render_text(font: &[Frame], text: &str) -> (u32, u32, Vec<u8>) {
    let glyph =
        |c: char| glyph_index(c).filter(|&i| i < font.len()).or(glyph_index('?')).map(|i| &font[i]);
    let advance = |g: &Frame| (g.width as i32 + g.offset[0] as i32).max(0) as u32;
    let line = font.first().map_or(0, |a| (a.height as i32 + a.offset[1] as i32).max(0) as u32);
    let width = text.chars().filter_map(glyph).map(advance).sum::<u32>().max(1);
    let mut rgba = vec![0; (width * line.max(1) * 4) as usize];
    let mut pen = 0;
    for g in text.chars().filter_map(glyph) {
        for y in 0..g.height {
            for x in 0..g.width {
                let (dx, dy) =
                    (pen as i32 + g.offset[0] as i32 + x as i32, g.offset[1] as i32 + y as i32);
                if dx < 0 || dy < 0 || dx as u32 >= width || dy as u32 >= line {
                    continue;
                }
                let from = ((y * g.width + x) * 4) as usize;
                if g.rgba[from + 3] != 0 {
                    let to = ((dy as u32 * width + dx as u32) * 4) as usize;
                    rgba[to..to + 4].copy_from_slice(&g.rgba[from..from + 4]);
                }
            }
        }
        pen += advance(g);
    }
    (width, line.max(1), rgba)
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

/// Lines in the retail credits roll, decoded from the installation's UTF-16 file.
#[derive(Asset, TypePath, Debug)]
pub struct Credits(pub Vec<CreditLine>);

#[derive(Debug)]
pub enum CreditLine {
    Heading(String),
    Centred(String),
    Pair(String, String),
    Blank,
}

impl TextTable {
    pub fn get(&self, index: usize) -> Option<&str> {
        self.0.get(index)?.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_follow_the_retail_order() {
        assert_eq!(glyph_index('A'), Some(0));
        assert_eq!(glyph_index('a'), Some(26));
        assert_eq!(glyph_index('0'), Some(52));
        assert_eq!(glyph_index('!'), Some(62));
        assert_eq!(glyph_index(' '), Some(92));
        assert_eq!(glyph_index('À'), Some(94));
    }
}
