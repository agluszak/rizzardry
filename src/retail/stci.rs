//! Sir-Tech Crazy Image (`.sti`): the retail UI's images and fonts.
//!
//! After Whizzardry `load_sti` (`src/canvas/himage.cpp`). Retail files are
//! either indexed sprite sheets, run-length encoded with transparent runs
//! (fonts, UI chrome), or plain RGB565 bitmaps (portraits, loading screens).

use thiserror::Error;

const RGB: u32 = 0x04;
const INDEXED: u32 = 0x08;
const ZLIB: u32 = 0x10;
const ETRLE: u32 = 0x20;
const HEADER: usize = 64;

#[derive(Debug, Error)]
#[error("invalid STCI image: {0}")]
pub struct StciError(&'static str);

/// One image of a sheet, as RGBA8 rows. A sheet's frames are drawn at their
/// offsets from the sheet's origin.
#[derive(Clone, Debug, PartialEq)]
pub struct StciFrame {
    pub offset: [i16; 2],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

pub fn read_stci(data: &[u8]) -> Result<Vec<StciFrame>, StciError> {
    if data.len() < HEADER || &data[..4] != b"STCI" {
        return Err(StciError("bad magic"));
    }
    let stored = u32_at(data, 8) as usize;
    let flags = u32_at(data, 16);
    let (height, width) = (u16_at(data, 20) as u32, u16_at(data, 22) as u32);
    let depth = data[44];
    if (flags & RGB != 0) == (flags & INDEXED != 0) {
        return Err(StciError("neither or both of RGB and indexed"));
    }
    if flags & ZLIB != 0 {
        return Err(StciError("zlib images are not used by retail"));
    }
    let mut at = HEADER;
    if flags & RGB != 0 {
        if depth != 16 || u32_at(data, 24) != 0xf800 {
            return Err(StciError("RGB images other than RGB565 are not used by retail"));
        }
        let count = (width * height) as usize;
        let pixels = data.get(at..at + count * 2).ok_or(StciError("bitmap truncated"))?;
        let mut rgba = Vec::with_capacity(count * 4);
        for pixel in pixels.as_chunks::<2>().0 {
            let v = u16::from_le_bytes(*pixel) as u32;
            let (r, g, b) = ((v >> 11) & 0x1f, (v >> 5) & 0x3f, v & 0x1f);
            rgba.extend([(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8, 255]);
        }
        return Ok(vec![StciFrame { offset: [0, 0], width, height, rgba }]);
    }
    if depth != 8 || u32_at(data, 24) != 256 {
        return Err(StciError("indexed images must have a 256-colour palette"));
    }
    if flags & ETRLE == 0 {
        return Err(StciError("unpacked indexed images are not used by retail"));
    }
    let frame_count = u16_at(data, 28) as usize;
    let palette = data.get(at..at + 768).ok_or(StciError("palette truncated"))?;
    at += 768;
    let table = data.get(at..at + frame_count * 16).ok_or(StciError("frame table truncated"))?;
    at += frame_count * 16;
    let pixels = data.get(at..at + stored).ok_or(StciError("pixels truncated"))?;
    table
        .as_chunks::<16>()
        .0
        .iter()
        .map(|entry| {
            let (offset, length) = (u32_at(entry, 0) as usize, u32_at(entry, 4) as usize);
            let x = i16::from_le_bytes([entry[8], entry[9]]);
            let y = i16::from_le_bytes([entry[10], entry[11]]);
            let (height, width) = (u16_at(entry, 12) as u32, u16_at(entry, 14) as u32);
            let runs =
                pixels.get(offset..offset + length).ok_or(StciError("frame outside data"))?;
            let rgba = decode_etrle(runs, width as usize, height as usize, palette)?;
            Ok(StciFrame { offset: [x, y], width, height, rgba })
        })
        .collect()
}

/// Rows of control bytes: 0 ends a row, a set high bit skips that many
/// transparent pixels, otherwise that many palette indices follow.
fn decode_etrle(
    runs: &[u8],
    width: usize,
    height: usize,
    palette: &[u8],
) -> Result<Vec<u8>, StciError> {
    let mut rgba = vec![0; width * height * 4];
    let mut runs = runs.iter().copied();
    for row in 0..height {
        let mut x = 0;
        loop {
            let control = runs.next().ok_or(StciError("run-length data ends early"))?;
            if control == 0 {
                break;
            }
            let count = (control & 0x7f) as usize;
            if count == 0 || x + count > width {
                return Err(StciError("run outside its row"));
            }
            if control & 0x80 == 0 {
                for i in 0..count {
                    let index = runs.next().ok_or(StciError("literal run truncated"))? as usize;
                    let at = (row * width + x + i) * 4;
                    rgba[at..at + 3].copy_from_slice(&palette[index * 3..index * 3 + 3]);
                    rgba[at + 3] = 255;
                }
            }
            x += count;
        }
        if x != width {
            return Err(StciError("row shorter than the frame"));
        }
    }
    Ok(rgba)
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

/// Draw text with a font sheet into an RGBA8 image, left to right with no
/// kerning: each glyph advances by its width plus x offset, and a line is as
/// tall as `A`. Unknown characters draw as `?`.
pub fn render_text(font: &[StciFrame], text: &str) -> (u32, u32, Vec<u8>) {
    let glyph =
        |c: char| glyph_index(c).filter(|&i| i < font.len()).or(glyph_index('?')).map(|i| &font[i]);
    let advance = |g: &StciFrame| (g.width as i32 + g.offset[0] as i32).max(0) as u32;
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

    #[test]
    #[ignore = "requires a retail installation"]
    fn every_retail_image_decodes() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let mut decoded = 0;
        for name in files.names().filter(|n| n.ends_with(".sti")) {
            let data = files.read(name).unwrap().unwrap();
            read_stci(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
            decoded += 1;
        }
        assert!(decoded >= 1327, "{decoded}");
        let font = files.read("data/fonts/wiz_text_font.sti").unwrap().unwrap();
        let font = read_stci(&font).unwrap();
        assert_eq!(font.len(), 154);
        let (_, height, _) = render_text(&font, "Arnika");
        assert_eq!(height, 11);
    }
}
