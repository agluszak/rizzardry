//! Import retail glyphs as an OpenType bitmap font for native Bevy text editing.
//! The editor keeps its normal shaping, caret, selection and IME behavior;
//! `sbix` carries the original pixels instead of approximating their outlines.
use crate::content::ui::{Frame, glyph_index};
use bevy::text::Font;
use write_fonts::{
    FontBuilder,
    tables::{
        cmap::Cmap,
        head::Head,
        hhea::Hhea,
        hmtx::{Hmtx, LongMetric},
        maxp::Maxp,
        name::{Name, NameRecord},
        os2::Os2,
        post::Post,
    },
    types::{FWord, GlyphId, NameId, Tag, UfWord, Version16Dot16},
};

pub fn import(frames: &[Frame], name: &str) -> Result<Font, String> {
    let height = frames.first().map_or(0, |g| i32::from(g.offset[1]) + g.height as i32);
    if !(1..=255).contains(&height) || frames.len() > 65534 {
        return Err("invalid bitmap font dimensions".into());
    }
    if frames.iter().any(|g| {
        g.width > 511 || g.height > 255 || g.offset.iter().any(|v| !(-255..=255).contains(v))
    }) {
        return Err("bitmap glyph metrics exceed font bounds".into());
    }
    let count = frames.len() + 1;
    let units = height as u16 * 64;
    let metrics: Vec<_> = std::iter::once(LongMetric { advance: units / 2, side_bearing: 0 })
        .chain(frames.iter().map(|g| LongMetric {
            advance: ((g.width as i32 + i32::from(g.offset[0])).max(0) * 64) as u16,
            side_bearing: g.offset[0] * 64,
        }))
        .collect();
    let max_width = metrics.iter().map(|m| m.advance).max().unwrap_or(0);
    let cmap = Cmap::from_mappings((0..=255).chain(0xfff0..=0xfff4).filter_map(|v| {
        let ch = char::from_u32(v)?;
        let index = glyph_index(ch).filter(|i| *i < frames.len())?;
        Some((ch, GlyphId::new(index as u32 + 1)))
    }))
    .map_err(|e| e.to_string())?;
    let names = Name::new(
        [(1, name), (2, "Regular"), (4, name), (6, name)]
            .into_iter()
            .map(|(id, value)| {
                NameRecord::new(3, 1, 0x409, NameId::new(id), value.to_owned().into())
            })
            .collect(),
    );
    // Supply exact integer-size strikes so native text does not blur these
    // pixel glyphs when the 640x480 UI is displayed at 2x, 3x or 4x.
    let strikes =
        (1..=4).map(|scale| strike(frames, height as u16, scale)).collect::<Result<Vec<_>, _>>()?;
    let mut sbix = Vec::from([0, 1, 0, 1]);
    sbix.extend((strikes.len() as u32).to_be_bytes());
    let mut offset = 8 + 4 * strikes.len() as u32;
    for strike in &strikes {
        sbix.extend(offset.to_be_bytes());
        offset += strike.len() as u32;
    }
    for strike in strikes {
        sbix.extend(strike);
    }
    let mut builder = FontBuilder::new();
    builder
        .add_table(&Head {
            units_per_em: units,
            y_max: height as i16 * 64,
            x_max: max_width as i16,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    builder
        .add_table(&Hhea {
            ascender: FWord::new(units as i16),
            advance_width_max: UfWord::new(max_width),
            number_of_h_metrics: count as u16,
            caret_slope_rise: 1,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    builder
        .add_table(&Hmtx { h_metrics: metrics, left_side_bearings: vec![] })
        .map_err(|e| e.to_string())?;
    builder.add_table(&Maxp::new(count as u16)).map_err(|e| e.to_string())?;
    builder.add_table(&cmap).map_err(|e| e.to_string())?;
    builder.add_table(&names).map_err(|e| e.to_string())?;
    builder
        .add_table(&Os2 {
            us_weight_class: 400,
            us_width_class: 5,
            s_typo_ascender: units as i16,
            us_win_ascent: units,
            us_first_char_index: 32,
            us_last_char_index: 255,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    builder
        .add_table(&Post { version: Version16Dot16::VERSION_3_0, ..Default::default() })
        .map_err(|e| e.to_string())?;
    builder.add_raw(Tag::new(b"sbix"), sbix);
    Ok(Font::from_bytes(builder.build()))
}

// write-fonts exposes strike headers but not trailing sbix glyph payloads.
fn strike(frames: &[Frame], height: u16, scale: u16) -> Result<Vec<u8>, String> {
    let mut pixels = Vec::new();
    let mut offsets = vec![4 + 4 * (frames.len() as u32 + 2)];
    offsets.push(offsets[0]); // .notdef has no bitmap
    for frame in frames {
        if frame.width != 0 && frame.height != 0 {
            pixels.extend((frame.offset[0] * scale as i16).to_be_bytes());
            pixels.extend(
                ((height as i16 - frame.offset[1] - frame.height as i16) * scale as i16)
                    .to_be_bytes(),
            );
            pixels.extend(b"png ");
            let (width, height) = (frame.width * u32::from(scale), frame.height * u32::from(scale));
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    let source = (((y / u32::from(scale)) * frame.width + x / u32::from(scale)) * 4)
                        as usize;
                    rgba.extend_from_slice(&frame.rgba[source..source + 4]);
                }
            }
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, width, height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
            }
            pixels.extend(png);
        }
        offsets.push(offsets[0] + pixels.len() as u32);
    }
    let mut result = Vec::new();
    result.extend((height * scale).to_be_bytes());
    result.extend(72u16.to_be_bytes());
    for offset in offsets {
        result.extend(offset.to_be_bytes());
    }
    result.extend(pixels);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use write_fonts::read::{FontRef, TableProvider};
    #[test]
    fn native_font_keeps_unicode_mapping_metrics_and_exact_scaled_pixels() {
        let rgba = vec![255, 80, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 255, 80, 0, 255];
        let font =
            import(&[Frame { offset: [1, 0], width: 2, height: 2, rgba: rgba.clone() }], "test")
                .unwrap();
        let parsed = FontRef::new(font.data.as_ref()).unwrap();
        let glyph = parsed.cmap().unwrap().map_codepoint('A').unwrap();
        assert_eq!(glyph, GlyphId::new(1));
        assert_eq!(parsed.hmtx().unwrap().h_metrics()[1].advance(), 3 * 64);
        let table = parsed.sbix().unwrap();
        let strike = table.strikes().get(1).unwrap();
        assert_eq!(strike.ppem(), 4);
        let image = strike.glyph_data(glyph).unwrap().unwrap();
        assert_eq!(image.origin_offset_x(), 2);
        let mut png = png::Decoder::new(std::io::Cursor::new(image.data())).read_info().unwrap();
        let mut out = vec![0; png.output_buffer_size().unwrap()];
        let info = png.next_frame(&mut out).unwrap();
        assert_eq!((info.width, info.height), (4, 4));
        assert_eq!(&out[0..4], &rgba[0..4]);
        assert_eq!(&out[4..8], &rgba[0..4]);
        assert_eq!(&out[16..20], &rgba[0..4]);
        assert_eq!(&out[8..12], &rgba[4..8]);
    }
}
