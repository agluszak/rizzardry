//! CPU mipmap generation for small retail textures.
//!
//! Retail generates mip levels for world textures and selects the nearest
//! level. Bevy does not generate mips for loaded images; the textures are
//! small enough to box-filter on load.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

/// Replace an RGBA8 image's data with a full mip chain. Other formats are
/// left unchanged.
pub fn generate_mipmaps(image: &mut Image) {
    if !matches!(
        image.texture_descriptor.format,
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
    ) || image.texture_descriptor.mip_level_count != 1
        || image.texture_descriptor.size.depth_or_array_layers != 1
    {
        return;
    }
    let Some(base) = image.data.take() else { return };
    let (mut width, mut height) = (image.width() as usize, image.height() as usize);
    let mut data = base.clone();
    let mut level = base;
    let mut levels = 1;
    while width > 1 || height > 1 {
        let (next_width, next_height) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = vec![0u8; next_width * next_height * 4];
        for y in 0..next_height {
            for x in 0..next_width {
                for channel in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (2 * x + dx).min(width - 1);
                        let sy = (2 * y + dy).min(height - 1);
                        sum += u32::from(level[(sy * width + sx) * 4 + channel]);
                    }
                    next[(y * next_width + x) * 4 + channel] = ((sum + 2) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        (level, width, height) = (next, next_width, next_height);
        levels += 1;
    }
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension};

    #[test]
    fn builds_a_box_filtered_chain_down_to_one_texel() {
        let texels: Vec<u8> = [[0, 0, 0, 255], [100, 0, 0, 255], [0, 200, 0, 255], [0, 0, 40, 255]]
            .into_iter()
            .flatten()
            .collect();
        let mut image = Image::new(
            Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            texels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        generate_mipmaps(&mut image);
        assert_eq!(image.texture_descriptor.mip_level_count, 2);
        assert_eq!(&image.data.unwrap()[16..], &[25, 50, 10, 255]);
    }
}
