//! Loads retail text files as [`TextTable`]s: the string table
//! (`stringdata.dat`) and level message files (`.msg`).

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;

use super::LoadError;
use super::strings::{read_messages, read_string_table};
use crate::content::ui::TextTable;

#[derive(Default, TypePath)]
pub struct TextLoader;

impl AssetLoader for TextLoader {
    type Asset = TextTable;
    type Settings = ();
    type Error = LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> Result<TextTable, LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        if context.path().get_full_extension().is_some_and(|e| e.eq_ignore_ascii_case("msg")) {
            Ok(TextTable(read_messages(&data).ok_or_else(malformed_messages)?))
        } else {
            Ok(TextTable(read_string_table(&data)?.into_iter().map(Some).collect()))
        }
    }

    fn extensions(&self) -> &[&str] {
        &["dat", "msg"]
    }
}

/// A message file that does not decode.
pub(super) fn malformed_messages() -> LoadError {
    LoadError::Invalid("malformed message file".into())
}
