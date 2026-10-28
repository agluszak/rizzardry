//! Loads retail `.sti` images as [`SpriteSheet`]s.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;

use super::stci::{palette, read_stci};
use crate::content::ui::{SpriteSheet, ui_image};

#[derive(Default, TypePath)]
pub struct StiLoader;

impl AssetLoader for StiLoader {
    type Asset = SpriteSheet;
    type Settings = ();
    type Error = super::LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<SpriteSheet, super::LoadError> {
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
        Ok(SpriteSheet { frames, images, palette: palette(&data) })
    }

    fn extensions(&self) -> &[&str] {
        &["sti"]
    }
}
