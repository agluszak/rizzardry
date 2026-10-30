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
    /// How far apart lines of text are in this font: its first glyph's
    /// height below the baseline offset, 12 for an empty font.
    pub fn line_height(&self) -> f32 {
        self.frames.first().map_or(12.0, |f| (f.height as i32 + f.offset[1] as i32).max(1) as f32)
    }

    /// How wide a line of text is in this font.
    pub fn text_width(&self, text: &str) -> u32 {
        text_width(&self.frames, text)
    }

    pub fn text_image_spaced(
        &self,
        text: &str,
        spacing: u32,
        palette: Option<&SpriteSheet>,
        ink: Option<(u8, Option<u8>)>,
    ) -> Image {
        let (width, height, mut rgba) = render_text_spaced(&self.frames, text, spacing);
        if let (Some(source), Some((foreground, shadow))) = (&self.palette, ink) {
            // Retail mprintf/MonoShadow: index 0 is background, index 1 is
            // shadow, other opaque pixels use the chosen foreground index.
            for pixel in rgba.as_chunks_mut::<4>().0.iter_mut().filter(|p| p[3] != 0) {
                if pixel[..3] == source[0] {
                    pixel[3] = 0;
                } else if pixel[..3] == source[1] {
                    if let Some(shadow) = shadow {
                        pixel[..3].copy_from_slice(&source[shadow as usize]);
                    } else {
                        pixel[3] = 0;
                    }
                } else {
                    pixel[..3].copy_from_slice(&source[foreground as usize]);
                }
            }
        } else if let (Some(source), Some(target)) =
            (&self.palette, palette.and_then(|p| p.palette.as_ref()))
        {
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
        // Retail's spell-target glyphs (`Font.cpp:1230-1244`): point, cone,
        // single, group and none.
        '\u{fff0}'..='\u{fff4}' => Some(149 + c as usize - 0xfff0),
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
    text.split('\n')
        .map(|line| {
            line.chars()
                .filter(|&c| c != '\r')
                .filter_map(glyph)
                .map(|g| (g.width as i32 + g.offset[0] as i32).max(0) as u32)
                .sum()
        })
        .max()
        .unwrap_or(0)
}

/// Draw text with a font sheet into an RGBA8 image, left to right with no
/// kerning: each glyph advances by its width plus x offset, and a line is as
/// tall as `A`. Unknown characters draw as `?`.
pub fn render_text(font: &[Frame], text: &str) -> (u32, u32, Vec<u8>) {
    render_text_spaced(font, text, 0)
}
fn render_text_spaced(font: &[Frame], text: &str, spacing: u32) -> (u32, u32, Vec<u8>) {
    let glyph =
        |c: char| glyph_index(c).filter(|&i| i < font.len()).or(glyph_index('?')).map(|i| &font[i]);
    let advance = |g: &Frame| (g.width as i32 + g.offset[0] as i32).max(0) as u32;
    let line = font.first().map_or(0, |a| (a.height as i32 + a.offset[1] as i32).max(0) as u32);
    let width = text_width(font, text).max(1);
    let rows = text.split('\n').count() as u32;
    let height = line.max(1) * rows + spacing * rows.saturating_sub(1);
    let mut rgba = vec![0; (width * height * 4) as usize];
    for (row, text) in text.split('\n').enumerate() {
        let mut pen = 0;
        for g in text.chars().filter(|&c| c != '\r').filter_map(glyph) {
            for y in 0..g.height {
                for x in 0..g.width {
                    let (dx, dy) =
                        (pen as i32 + g.offset[0] as i32 + x as i32, g.offset[1] as i32 + y as i32);
                    if dx < 0 || dy < 0 || dx as u32 >= width || dy as u32 >= line {
                        continue;
                    }
                    let from = ((y * g.width + x) * 4) as usize;
                    if g.rgba[from + 3] != 0 {
                        let to = (((row as u32 * (line + spacing) + dy as u32) * width + dx as u32)
                            * 4) as usize;
                        rgba[to..to + 4].copy_from_slice(&g.rgba[from..from + 4]);
                    }
                }
            }
            pen += advance(g);
        }
    }
    (width, height, rgba)
}

/// Wrap paragraphs at word boundaries, splitting overlong words to keep them
/// inside the viewport. Explicit empty lines are retained.
pub fn wrap_text(font: &[Frame], text: &str, width: u32) -> String {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate =
                if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if text_width(font, &candidate) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for c in word.chars() {
                let candidate = format!("{line}{c}");
                if !line.is_empty() && text_width(font, &candidate) > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    lines.join("\n")
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

/// Where name lists start in the game's string table, each indexed by its
/// enum's retail order.
pub const RACE_NAMES: usize = 0x284;
pub const PROFESSION_NAMES: usize = 0x2a4;
pub const GENDER_NAMES: usize = 0x2d1;
pub const SKILL_NAMES: usize = 0x2e2;
pub const ABILITY_NAMES: usize = 0x328;
pub const TRAP_NAMES: usize = 0x542;

#[cfg(test)]
mod bitmap_layout_tests {
    use super::*;
    #[test]
    fn monochrome_quote_font_uses_white_ink_without_its_green_shadow() {
        let mut palette = vec![[0, 0, 0]; 256];
        palette[1] = [0, 54, 0];
        palette[208] = [255, 255, 255];
        palette[209] = [222, 222, 222];
        let font = SpriteSheet {
            frames: vec![Frame {
                offset: [0, 0],
                width: 2,
                height: 1,
                rgba: vec![0, 54, 0, 255, 222, 222, 222, 255],
            }],
            images: vec![],
            palette: Some(palette),
        };
        let image = font.text_image_spaced("A", 0, None, Some((208, None)));
        let pixels = image.data.unwrap();
        assert_eq!(pixels[3], 0);
        assert_eq!(&pixels[4..8], &[255; 4]);
    }

    #[test]
    fn paragraphs_and_wrapped_words_have_real_pixel_height() {
        let font = vec![Frame { offset: [0, 0], width: 2, height: 3, rgba: vec![255; 24] }; 150];
        assert_eq!(wrap_text(&font, "AB CD\n\nABCDEF", 6), "AB\nCD\n\nABC\nDEF");
        let (width, height, pixels) = render_text(&font, "AB\r\nC");
        assert_eq!((width, height), (4, 6));
        assert_eq!(&pixels[(4 * 3 * 4)..(4 * 3 * 4 + 4)], &[255; 4]);
        assert_eq!(&pixels[(4 * 3 * 4 + 8)..(4 * 3 * 4 + 12)], &[0; 4]);
    }
}
