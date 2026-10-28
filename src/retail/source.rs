//! The `retail://` Bevy asset source, and `navmesh://`: the inputs of
//! import-time navmesh baking.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, PathStream, Reader, VecReader,
};
use bevy::tasks::futures_lite::stream;

use super::files::RetailFiles;
use crate::content::navmesh::{LevelCollision, NAVMESH_EXTENSION, navmesh_input_path};

pub const RETAIL_SOURCE: &str = "retail";

struct RetailReader(Arc<RetailFiles>);

fn name(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl AssetReader for RetailReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let (files, name) = (self.0.clone(), name(path));
        // File reads block; keep them off the asset tasks.
        match blocking::unblock(move || files.read(&name)).await {
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

/// The retail installation, read in place.
pub fn retail_source(files: Arc<RetailFiles>) -> AssetSourceBuilder {
    AssetSourceBuilder::new(move || Box::new(RetailReader(files.clone())))
}

/// Every retail level `<path>.pvl` as `<path>_class<n>.navmesh`, one per
/// walker size class: the level's collision for that class, as
/// [`LevelCollision`] encodes it. The navmesh loader bakes them on request,
/// keyed by these bytes, so a change to the level's collision re-bakes.
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

/// The navmesh input of a level (`levels/x/y.pvl`) for a size class, or
/// `None` if the level or its collision is missing.
pub fn navmesh_input_bytes(
    files: &RetailFiles,
    level: &str,
    class: u8,
) -> Option<std::io::Result<Vec<u8>>> {
    let stem = level.strip_suffix(".pvl").unwrap_or(level);
    let read = |extension: &str| files.read(&format!("{stem}.{extension}"));
    let (octree, level) = match (read("oct")?, read("pvl")?) {
        (Ok(octree), Ok(level)) => (octree, level),
        (Err(error), _) | (_, Err(error)) => return Some(Err(error)),
    };
    Some(
        super::navmesh::level_collision(&octree, &level)
            .map(|collision| LevelCollision { collision, class: class.into() }.encode())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
    )
}

impl AssetReader for NavmeshInputReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let not_found = || AssetReaderError::NotFound(path.to_owned());
        let (stem, class) = level_and_class(path).ok_or_else(not_found)?;
        if class as usize >= self.classes {
            return Err(not_found());
        }
        let files = self.files.clone();
        match blocking::unblock(move || navmesh_input_bytes(&files, &stem, class)).await {
            Some(Ok(bytes)) => Ok(VecReader::new(bytes)),
            Some(Err(error)) => Err(AssetReaderError::Io(Arc::new(error))),
            None => Err(not_found()),
        }
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

/// Navmesh inputs for `classes` size classes.
pub fn navmesh_source(files: Arc<RetailFiles>, classes: usize) -> AssetSourceBuilder {
    AssetSourceBuilder::new(move || Box::new(NavmeshInputReader { files: files.clone(), classes }))
}
