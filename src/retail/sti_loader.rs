//! Loads retail `.sti` images as [`SpriteSheet`]s.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use thiserror::Error;

use super::stci::{StciError, read_stci};
use crate::content::ui::{SpriteSheet, ui_image};

#[derive(Default, TypePath)]
pub struct StiLoader;

#[derive(Debug, Error)]
pub enum StiLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Stci(#[from] StciError),
}

impl AssetLoader for StiLoader {
    type Asset = SpriteSheet;
    type Settings = ();
    type Error = StiLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<SpriteSheet, StiLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let frames = read_stci(&data)?;
        let images = frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let image = ui_image(frame.width, frame.height, frame.rgba.clone());
                context.add_labeled_asset(format!("frame/{index}"), image)
            })
            .collect();
        // `read_stci` has already validated the header and indexed palette.
        let flags = u32::from_le_bytes(data[16..20].try_into().unwrap());
        let palette = (flags & 0x08 != 0).then(|| data[64..832].as_chunks::<3>().0.to_vec());
        Ok(SpriteSheet { frames, images, palette })
    }

    fn extensions(&self) -> &[&str] {
        &["sti"]
    }
}
