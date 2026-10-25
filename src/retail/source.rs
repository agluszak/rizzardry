//! The `retail://` Bevy asset source, and `navmesh://`: the inputs of
//! import-time navmesh baking.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSource, AssetSourceBuilder, PathStream, Reader, VecReader,
};
use bevy::tasks::futures_lite::stream;

use super::files::RetailFiles;

pub const RETAIL_SOURCE: &str = "retail";

struct RetailReader(Arc<RetailFiles>);

fn name(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl AssetReader for RetailReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        match self.0.read(&name(path)) {
            Some(Ok(bytes)) => Ok(VecReader::new(bytes)),
            Some(Err(error)) => Err(AssetReaderError::Io(Arc::new(error))),
            None => Err(AssetReaderError::NotFound(path.to_owned())),
        }
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // Retail assets never carry Bevy `.meta` files; loaders use defaults.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let name = name(path);
        if !self.0.is_directory(&name) {
            return Err(AssetReaderError::NotFound(path.to_owned()));
        }
        let children: Vec<PathBuf> =
            self.0.list_directory(&name).into_iter().map(PathBuf::from).collect();
        Ok(Box::new(stream::iter(children)))
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(self.0.is_directory(&name(path)))
    }
}

/// The retail installation. It is never processed: in processed mode the
/// same reader serves as its "processed" reader, so nothing is copied.
pub fn retail_source(files: Arc<RetailFiles>) -> AssetSourceBuilder {
    let processed = files.clone();
    AssetSourceBuilder::new(move || Box::new(RetailReader(files.clone())))
        .with_processed_reader(move || Box::new(RetailReader(processed.clone())))
}

pub const NAVMESH_SOURCE: &str = "navmesh";
pub const NAVMESH_EXTENSION: &str = "navmesh";

/// Every retail level `<path>.pvl` as `<path>.navmesh`, whose bytes are the
/// level's collision (`.oct`) and placement (`.pvl`) files together; see
/// [`navmesh_input`]. The asset processor turns these into baked navmeshes,
/// re-baking when either file changes.
struct NavmeshInputReader(Arc<RetailFiles>);

fn level_stem(path: &Path) -> Option<String> {
    name(path).strip_suffix(&format!(".{NAVMESH_EXTENSION}")).map(str::to_owned)
}

const NAVMESH_INPUT_MAGIC: &[u8; 4] = b"RZNI";

/// Split a navmesh input into its `.oct` and `.pvl` bytes.
pub fn navmesh_input(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = bytes.strip_prefix(NAVMESH_INPUT_MAGIC)?;
    let (length, rest) = rest.split_first_chunk::<4>()?;
    let length = u32::from_le_bytes(*length) as usize;
    (length <= rest.len()).then(|| rest.split_at(length))
}

impl AssetReader for NavmeshInputReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let not_found = || AssetReaderError::NotFound(path.to_owned());
        let stem = level_stem(path).ok_or_else(not_found)?;
        let read = |extension: &str| match self.0.read(&format!("{stem}.{extension}")) {
            Some(Ok(bytes)) => Ok(bytes),
            Some(Err(error)) => Err(AssetReaderError::Io(Arc::new(error))),
            None => Err(not_found()),
        };
        let (octree, level) = (read("oct")?, read("pvl")?);
        let mut bytes = NAVMESH_INPUT_MAGIC.to_vec();
        bytes.extend((octree.len() as u32).to_le_bytes());
        bytes.extend(octree);
        bytes.extend(level);
        Ok(VecReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // Inputs use the default processor and bake settings.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let name = name(path);
        if !self.0.is_directory(&name) {
            return Err(AssetReaderError::NotFound(path.to_owned()));
        }
        let children: Vec<PathBuf> = self
            .0
            .list_directory(&name)
            .into_iter()
            .filter_map(|child| match child.strip_suffix(".pvl") {
                Some(stem) => Some(format!("{stem}.{NAVMESH_EXTENSION}")),
                None => self.0.is_directory(&child).then_some(child),
            })
            .map(PathBuf::from)
            .collect();
        Ok(Box::new(stream::iter(children)))
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(self.0.is_directory(&name(path)))
    }
}

/// Navmesh inputs, processed into `cache` (e.g. `imported_assets/navmesh`).
pub fn navmesh_source(files: Arc<RetailFiles>, cache: &Path) -> AssetSourceBuilder {
    let cache = cache.to_string_lossy().into_owned();
    AssetSourceBuilder::new(move || Box::new(NavmeshInputReader(files.clone())))
        .with_processed_reader(AssetSource::get_default_reader(cache.clone()))
        .with_processed_writer(AssetSource::get_default_writer(cache))
}
