//! Sir-Tech Crazy Image (`.sti`): the retail UI's images and fonts.
//!
//! After Whizzardry `load_sti` (`src/canvas/himage.cpp`). Retail files are
//! either indexed sprite sheets, run-length encoded with transparent runs
//! (fonts, UI chrome), or plain RGB565 bitmaps (portraits, loading screens).

use crate::bytes::{DecodeError, i16_at, u16_at, u32_at};

use crate::content::ui::Frame;

const RGB: u32 = 0x04;
const INDEXED: u32 = 0x08;
const ZLIB: u32 = 0x10;
const ETRLE: u32 = 0x20;
const HEADER: usize = 64;

pub fn read_stci(data: &[u8]) -> Result<Vec<Frame>, DecodeError> {
    if data.len() < HEADER || &data[..4] != b"STCI" {
        return Err(DecodeError::Invalid("bad magic", 0));
    }
    let stored = u32_at(data, 8)? as usize;
    let flags = u32_at(data, 16)?;
    let (height, width) = (u16_at(data, 20)? as u32, u16_at(data, 22)? as u32);
    let depth = data[44];
    if (flags & RGB != 0) == (flags & INDEXED != 0) {
        return Err(DecodeError::Invalid("neither or both of RGB and indexed", 0));
    }
    if flags & ZLIB != 0 {
        return Err(DecodeError::Invalid("zlib images are not used by retail", 0));
    }
    let mut at = HEADER;
    if flags & RGB != 0 {
        if depth != 16 || u32_at(data, 24)? != 0xf800 {
            return Err(DecodeError::Invalid(
                "RGB images other than RGB565 are not used by retail",
                0,
            ));
        }
        let count = (width * height) as usize;
        let pixels =
            data.get(at..at + count * 2).ok_or(DecodeError::Invalid("bitmap truncated", 0))?;
        let mut rgba = Vec::with_capacity(count * 4);
        for pixel in pixels.as_chunks::<2>().0 {
            let v = u16::from_le_bytes(*pixel) as u32;
            let (r, g, b) = ((v >> 11) & 0x1f, (v >> 5) & 0x3f, v & 0x1f);
            rgba.extend([(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8, 255]);
        }
        return Ok(vec![Frame { offset: [0, 0], width, height, rgba }]);
    }
    if depth != 8 || u32_at(data, 24)? != 256 {
        return Err(DecodeError::Invalid("indexed images must have a 256-colour palette", 0));
    }
    if flags & ETRLE == 0 {
        return Err(DecodeError::Invalid("unpacked indexed images are not used by retail", 0));
    }
    let frame_count = u16_at(data, 28)? as usize;
    let palette = data.get(at..at + 768).ok_or(DecodeError::Invalid("palette truncated", 0))?;
    at += 768;
    let table = data
        .get(at..at + frame_count * 16)
        .ok_or(DecodeError::Invalid("frame table truncated", 0))?;
    at += frame_count * 16;
    let pixels = data.get(at..at + stored).ok_or(DecodeError::Invalid("pixels truncated", 0))?;
    table
        .as_chunks::<16>()
        .0
        .iter()
        .map(|entry| {
            let (offset, length) = (u32_at(entry, 0)? as usize, u32_at(entry, 4)? as usize);
            let (x, y) = (i16_at(entry, 8)?, i16_at(entry, 10)?);
            let (height, width) = (u16_at(entry, 12)? as u32, u16_at(entry, 14)? as u32);
            let runs = pixels
                .get(offset..offset + length)
                .ok_or(DecodeError::Invalid("frame outside data", 0))?;
            let rgba = decode_etrle(runs, width as usize, height as usize, palette)?;
            Ok(Frame { offset: [x, y], width, height, rgba })
        })
        .collect()
}

/// An indexed image's palette; `None` for an RGB bitmap or a malformed file.
pub fn palette(data: &[u8]) -> Option<Vec<[u8; 3]>> {
    let flags = u32_at(data, 16).ok()?;
    let palette = data.get(HEADER..HEADER + 768).filter(|_| flags & INDEXED != 0)?;
    Some(palette.as_chunks::<3>().0.to_vec())
}

/// Rows of control bytes: 0 ends a row, a set high bit skips that many
/// transparent pixels, otherwise that many palette indices follow.
fn decode_etrle(
    runs: &[u8],
    width: usize,
    height: usize,
    palette: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let mut rgba = vec![0; width * height * 4];
    let mut runs = runs.iter().copied();
    for row in 0..height {
        let mut x = 0;
        loop {
            let control =
                runs.next().ok_or(DecodeError::Invalid("run-length data ends early", 0))?;
            if control == 0 {
                break;
            }
            let count = (control & 0x7f) as usize;
            if count == 0 || x + count > width {
                return Err(DecodeError::Invalid("run outside its row", 0));
            }
            if control & 0x80 == 0 {
                for i in 0..count {
                    let index =
                        runs.next().ok_or(DecodeError::Invalid("literal run truncated", 0))?
                            as usize;
                    let at = (row * width + x + i) * 4;
                    rgba[at..at + 3].copy_from_slice(&palette[index * 3..index * 3 + 3]);
                    rgba[at + 3] = 255;
                }
            }
            x += count;
        }
        if x != width {
            return Err(DecodeError::Invalid("row shorter than the frame", 0));
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn every_retail_image_decodes() {
        let files = crate::retail::files::retail_files();
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
        let (_, height, _) = crate::content::ui::render_text(&font, "Arnika");
        assert_eq!(height, 11);
    }
}
