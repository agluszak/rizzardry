//! Indexed PCX artwork used by the retail interface.
use crate::bytes::DecodeError;
use crate::content::ui::ui_image;
use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::reflect::TypePath;

#[derive(Default, TypePath)]
pub struct PcxLoader;

impl AssetLoader for PcxLoader {
    type Asset = Image;
    type Settings = ();
    type Error = super::LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Image, super::LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        Ok(decode(&data)?)
    }

    fn extensions(&self) -> &[&str] {
        &["pcx"]
    }
}

fn decode(data: &[u8]) -> Result<Image, DecodeError> {
    if data.len() < 128 + 769
        || data[0] != 10
        || data[2] != 1
        || data[3] != 8
        || data[65] != 1
        || data[data.len() - 769] != 12
    {
        return Err(DecodeError::Invalid("header or palette", 0));
    }
    // The header is long enough for every word read here.
    let word = |at| usize::from(u16::from_le_bytes([data[at], data[at + 1]]));
    if word(8) < word(4) || word(10) < word(6) {
        return Err(DecodeError::Invalid("dimensions", 0));
    }
    let width = word(8) - word(4) + 1;
    let height = word(10) - word(6) + 1;
    let stride = word(66);
    if width > 8192 || height > 8192 || stride < width || stride > 8192 {
        return Err(DecodeError::Invalid("dimensions", 0));
    }
    let palette_start = data.len() - 769;
    let palette = &data[palette_start + 1..];
    let mut rgba = Vec::with_capacity(width * height * 4);
    let mut at = 128;
    for _ in 0..height {
        let mut x = 0;
        while x < stride {
            let byte = *data
                .get(at)
                .filter(|_| at < palette_start)
                .ok_or(DecodeError::Invalid("truncated pixels", 0))?;
            at += 1;
            let (count, index) = if byte & 0xc0 == 0xc0 {
                let index = *data
                    .get(at)
                    .filter(|_| at < palette_start)
                    .ok_or(DecodeError::Invalid("truncated run", 0))?;
                at += 1;
                (usize::from(byte & 0x3f), index)
            } else {
                (1, byte)
            };
            if count == 0 || x + count > stride {
                return Err(DecodeError::Invalid("run outside row", 0));
            }
            for _ in 0..count {
                if x < width {
                    let i = usize::from(index) * 3;
                    rgba.extend_from_slice(&[palette[i], palette[i + 1], palette[i + 2], 255]);
                }
                x += 1;
            }
        }
    }
    Ok(ui_image(width as u32, height as u32, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_pcx_preserves_palette_pixels_and_rejects_truncated_runs() {
        let mut data = vec![0; 128];
        data[0] = 10;
        data[2] = 1;
        data[3] = 8;
        data[8] = 2; // three pixels, one padding byte per row
        data[65] = 1;
        data[66] = 4;
        data.extend([0xc2, 1, 2, 0]);
        data.push(12);
        let mut palette = [0; 768];
        palette[3..6].copy_from_slice(&[20, 30, 40]);
        palette[6..9].copy_from_slice(&[50, 60, 70]);
        data.extend(palette);
        let image = decode(&data).unwrap();
        assert_eq!(image.data.unwrap(), vec![20, 30, 40, 255, 20, 30, 40, 255, 50, 60, 70, 255]);
        data[128] = 0xc5;
        assert!(decode(&data).is_err());
    }
}
