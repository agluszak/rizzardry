//! Indexed PCX artwork used by the retail interface.
use crate::content::ui::ui_image;
use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::reflect::TypePath;

#[derive(Default, TypePath)]
pub struct PcxLoader;

#[derive(Debug, thiserror::Error)]
pub enum PcxError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid or unsupported indexed PCX: {0}")]
    Invalid(&'static str),
}

impl AssetLoader for PcxLoader {
    type Asset = Image;
    type Settings = ();
    type Error = PcxError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Image, PcxError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        decode(&data)
    }

    fn extensions(&self) -> &[&str] {
        &["pcx"]
    }
}

fn decode(data: &[u8]) -> Result<Image, PcxError> {
    if data.len() < 128 + 769
        || data[0] != 10
        || data[2] != 1
        || data[3] != 8
        || data[65] != 1
        || data[data.len() - 769] != 12
    {
        return Err(PcxError::Invalid("header or palette"));
    }
    let word = |at| u16::from_le_bytes([data[at], data[at + 1]]) as usize;
    let width = word(8)
        .checked_sub(word(4))
        .and_then(|n| n.checked_add(1))
        .ok_or(PcxError::Invalid("width"))?;
    let height = word(10)
        .checked_sub(word(6))
        .and_then(|n| n.checked_add(1))
        .ok_or(PcxError::Invalid("height"))?;
    let stride = word(66);
    if width > 8192 || height > 8192 || stride < width || stride > 8192 {
        return Err(PcxError::Invalid("dimensions"));
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
                .ok_or(PcxError::Invalid("truncated pixels"))?;
            at += 1;
            let (count, index) = if byte & 0xc0 == 0xc0 {
                let index = *data
                    .get(at)
                    .filter(|_| at < palette_start)
                    .ok_or(PcxError::Invalid("truncated run"))?;
                at += 1;
                (usize::from(byte & 0x3f), index)
            } else {
                (1, byte)
            };
            if count == 0 || x + count > stride {
                return Err(PcxError::Invalid("run outside row"));
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
