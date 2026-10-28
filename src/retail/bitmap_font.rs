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
    // write-fonts exposes sbix strike headers but not their trailing glyph
    // payload. Assemble this one flat offset table per the OpenType sbix spec.
    let mut pixels = Vec::new();
    let mut offsets = vec![4 + 4 * (count as u32 + 1)];
    offsets.push(offsets[0]); // .notdef has no bitmap
    for frame in frames {
        if frame.width != 0 && frame.height != 0 {
            pixels.extend(frame.offset[0].to_be_bytes());
            pixels.extend((height as i16 - frame.offset[1] - frame.height as i16).to_be_bytes());
            pixels.extend(b"png ");
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, frame.width, frame.height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                writer.write_image_data(&frame.rgba).map_err(|e| e.to_string())?;
            }
            pixels.extend(png);
        }
        offsets.push(offsets[0] + pixels.len() as u32);
    }
    let mut sbix = Vec::from([0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 12]);
    sbix.extend((height as u16).to_be_bytes());
    sbix.extend(72u16.to_be_bytes());
    for offset in offsets {
        sbix.extend(offset.to_be_bytes());
    }
    sbix.extend(pixels);
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
