//! Retail textures (`.tga`, `.jpg`) as ordinary shared Bevy images.
//!
//! Materials load `<file>#image`: the [`RetailTexture`] loader decodes it
//! with retail sampling (linear filtering, nearest mip selection, repeat)
//! and a full mip chain. Loading the file as a plain `Image` (UI art) still
//! uses Bevy's own loader.

use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages, io::Reader};
use bevy::image::{
    CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
    ImageSamplerDescriptor, ImageType,
};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use crate::presentation::mipmaps::generate_mipmaps;

/// A retail texture file; its decoded image is the `image` sub-asset.
#[derive(Asset, TypePath, Debug)]
pub struct RetailTexture {
    pub image: Handle<Image>,
}

#[derive(Default, TypePath)]
pub struct RetailTextureLoader;

impl AssetLoader for RetailTextureLoader {
    type Asset = RetailTexture;
    type Settings = ();
    type Error = super::LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<RetailTexture, super::LoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let extension = context.path().get_full_extension().unwrap_or_default().to_lowercase();
        let sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mipmap_filter: ImageFilterMode::Nearest,
            ..ImageSamplerDescriptor::linear()
        });
        let mut image = Image::from_buffer(
            &bytes,
            ImageType::Extension(&extension),
            CompressedImageFormats::NONE,
            true,
            sampler,
            RenderAssetUsages::RENDER_WORLD,
        )?;
        generate_mipmaps(&mut image);
        Ok(RetailTexture { image: context.add_labeled_asset("image", image) })
    }

    fn extensions(&self) -> &[&str] {
        &["tga", "jpg"]
    }
}

/// Whether retail treats a texture file as carrying alpha: its pixel format
/// has an alpha channel (Whizzardry `Texture::hasAlpha`), which for retail
/// files means a 32-bit TGA. The texels are not inspected.
pub fn file_has_alpha(bytes: &[u8], extension: &str) -> bool {
    extension.eq_ignore_ascii_case("tga") && bytes.get(16) == Some(&32)
}
