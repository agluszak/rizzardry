//! Loads retail text files as [`TextTable`]s: the string table
//! (`stringdata.dat`) and level message files (`.msg`).

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use thiserror::Error;

use super::pvl::PvlError;
use super::strings::{read_messages, read_string_table};
use crate::content::ui::TextTable;

#[derive(Default, TypePath)]
pub struct TextLoader;

#[derive(Debug, Error)]
pub enum TextLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Strings(#[from] PvlError),
    #[error("malformed message file")]
    Messages,
}

impl AssetLoader for TextLoader {
    type Asset = TextTable;
    type Settings = ();
    type Error = TextLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<TextTable, TextLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        if context.path().get_full_extension().is_some_and(|e| e.eq_ignore_ascii_case("msg")) {
            Ok(TextTable(read_messages(&data).ok_or(TextLoadError::Messages)?))
        } else {
            Ok(TextTable(read_string_table(&data)?.into_iter().map(Some).collect()))
        }
    }

    fn extensions(&self) -> &[&str] {
        &["dat", "msg"]
    }
}
