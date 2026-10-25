//! The `retail://` Bevy asset source, and `navmesh://`: the inputs of
//! import-time navmesh baking.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSource, AssetSourceBuilder, PathStream, Reader, VecReader,
};
use bevy::tasks::futures_lite::stream;

use super::files::RetailFiles;
use crate::content::navmesh::{NAVMESH_EXTENSION, navmesh_input_path};

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

/// Every retail level `<path>.pvl` as `<path>_class<n>.navmesh`, one per
/// walker size class, whose bytes are the class and the level's collision
/// (`.oct`) and placement (`.pvl`) files; see [`navmesh_input`]. The asset
/// processor turns these into baked navmeshes, re-baking when either file
/// changes.
struct NavmeshInputReader {
    files: Arc<RetailFiles>,
    classes: usize,
}

/// The level stem and size class of a navmesh input name.
fn level_and_class(path: &Path) -> Option<(String, u8)> {
    let stem = name(path);
    let stem = stem.strip_suffix(&format!(".{NAVMESH_EXTENSION}"))?;
    let (level, class) = stem.rsplit_once("_class")?;
    Some((level.to_owned(), class.parse().ok()?))
}

const NAVMESH_INPUT_MAGIC: &[u8; 4] = b"RZN2";

/// A navmesh input's size class and its `.oct` and `.pvl` bytes.
pub fn navmesh_input(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let rest = bytes.strip_prefix(NAVMESH_INPUT_MAGIC)?;
    let (&class, rest) = rest.split_first()?;
    let (length, rest) = rest.split_first_chunk::<4>()?;
    let length = u32::from_le_bytes(*length) as usize;
    (length <= rest.len()).then(|| {
        let (octree, level) = rest.split_at(length);
        (class, octree, level)
    })
}

impl AssetReader for NavmeshInputReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let not_found = || AssetReaderError::NotFound(path.to_owned());
        let (stem, class) = level_and_class(path).ok_or_else(not_found)?;
        if class as usize >= self.classes {
            return Err(not_found());
        }
        let read = |extension: &str| match self.files.read(&format!("{stem}.{extension}")) {
            Some(Ok(bytes)) => Ok(bytes),
            Some(Err(error)) => Err(AssetReaderError::Io(Arc::new(error))),
            None => Err(not_found()),
        };
        let (octree, level) = (read("oct")?, read("pvl")?);
        let mut bytes = NAVMESH_INPUT_MAGIC.to_vec();
        bytes.push(class);
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
        if !self.files.is_directory(&name) {
            return Err(AssetReaderError::NotFound(path.to_owned()));
        }
        let mut children: Vec<PathBuf> = Vec::new();
        for child in self.files.list_directory(&name) {
            if child.ends_with(".pvl") {
                children.extend(
                    (0..self.classes).map(|c| PathBuf::from(navmesh_input_path(&child, c))),
                );
            } else if self.files.is_directory(&child) {
                children.push(PathBuf::from(child));
            }
        }
        Ok(Box::new(stream::iter(children)))
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(self.files.is_directory(&name(path)))
    }
}

/// Navmesh inputs for `classes` size classes, processed into `cache` (e.g.
/// `imported_assets/navmesh`).
pub fn navmesh_source(files: Arc<RetailFiles>, classes: usize, cache: &Path) -> AssetSourceBuilder {
    let cache = cache.to_string_lossy().into_owned();
    AssetSourceBuilder::new(move || Box::new(NavmeshInputReader { files: files.clone(), classes }))
        .with_processed_reader(AssetSource::get_default_reader(cache.clone()))
        .with_processed_writer(AssetSource::get_default_writer(cache))
}
