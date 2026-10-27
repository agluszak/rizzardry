//! One namespace over a retail installation's archives and loose files.
//!
//! Override order follows the original loader: the base libraries (more
//! specific prefixes win), then `patches/patch.NNN` in ascending order, then
//! loose files in the installation directory.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::path::normalize;
use super::slf::{SlfArchive, SlfError};

const BASE_LIBRARIES: [&str; 6] = [
    "data/data.slf",
    "data/sound/sound.slf",
    "data/sound/monsters/monstersound.slf",
    "data/music/music.slf",
    "data/monsters/monsters.slf",
    "levels/levels.slf",
];
const PATCH_LIMIT: u32 = 50;

#[derive(Clone, Debug)]
enum Location {
    Loose(PathBuf),
    Archive { archive: usize, offset: u64, length: u64 },
}

#[derive(Debug)]
pub struct RetailFiles {
    root: PathBuf,
    archives: Vec<PathBuf>,
    files: BTreeMap<String, Location>,
}

#[derive(Debug, thiserror::Error)]
pub enum RetailFilesError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {source}")]
    Archive { path: PathBuf, source: SlfError },
    #[error("loose files {0:?} and {1:?} differ only in case")]
    CaseCollision(PathBuf, PathBuf),
}

fn is_excluded_loose_file(name: &str) -> bool {
    name.ends_with(".slf")
        || name.ends_with(".sav")
        || name.starts_with("patches/")
        || name.starts_with("config/")
        || matches!(name, "cd.rom" | "wiz8.cfg" | "3dvideo.cfg")
}

impl RetailFiles {
    /// Index an installation directory (or an already extracted copy of one).
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, RetailFilesError> {
        let root = root.into();
        let io_error = |path: &Path| {
            let path = path.to_owned();
            move |source| RetailFilesError::Io { path, source }
        };
        let mut loose = BTreeMap::new();
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).map_err(io_error(&directory))? {
                let entry = entry.map_err(io_error(&directory))?;
                let path = entry.path();
                let kind = entry.file_type().map_err(io_error(&path))?;
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() {
                    let relative = path.strip_prefix(&root).expect("walked below the root");
                    let Ok(name) = normalize(&relative.to_string_lossy()) else { continue };
                    if let Some(previous) = loose.insert(name, path.clone()) {
                        return Err(RetailFilesError::CaseCollision(previous, path));
                    }
                }
            }
        }

        let mut libraries = Vec::new();
        for name in BASE_LIBRARIES {
            if let Some(path) = loose.get(name) {
                let archive = SlfArchive::open(path)
                    .map_err(|source| RetailFilesError::Archive { path: path.clone(), source })?;
                libraries.push((path.clone(), archive));
            }
        }
        libraries.sort_by_key(|(_, archive)| archive.prefix.len());
        for n in 0..PATCH_LIMIT {
            if let Some(path) = loose.get(&format!("patches/patch.{n:03}")) {
                let archive = SlfArchive::open(path)
                    .map_err(|source| RetailFilesError::Archive { path: path.clone(), source })?;
                libraries.push((path.clone(), archive));
            }
        }

        let mut files = BTreeMap::new();
        let mut archives = Vec::new();
        for (path, archive) in libraries {
            let index = archives.len();
            archives.push(path);
            for entry in archive.entries {
                let location = Location::Archive {
                    archive: index,
                    offset: entry.offset,
                    length: entry.length,
                };
                files.insert(entry.name, location);
            }
        }
        for (name, path) in loose {
            if !is_excluded_loose_file(&name) {
                files.insert(name, Location::Loose(path));
            }
        }
        Ok(RetailFiles { root, archives, files })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn contains(&self, name: &str) -> bool {
        normalize(name).is_ok_and(|name| self.files.contains_key(&name))
    }

    /// Whether any indexed file lies below `name`.
    pub fn is_directory(&self, name: &str) -> bool {
        let Ok(name) = normalize(name) else { return name.trim_matches(['/', '\\']).is_empty() };
        let prefix = format!("{name}/");
        self.files.range(prefix.clone()..).next().is_some_and(|(key, _)| key.starts_with(&prefix))
    }

    /// The names directly below a directory, as full retail names.
    pub fn list_directory(&self, name: &str) -> Vec<String> {
        let prefix = match normalize(name) {
            Ok(name) => format!("{name}/"),
            Err(_) => String::new(),
        };
        let mut children: Vec<String> = Vec::new();
        let keys = self.files.range(prefix.clone()..).map(|(key, _)| key);
        for key in keys.take_while(|key| key.starts_with(&prefix)) {
            let rest = &key[prefix.len()..];
            let child = match rest.find('/') {
                Some(end) => &key[..prefix.len() + end],
                None => key.as_str(),
            };
            if children.last().is_none_or(|last| last != child) {
                children.push(child.to_owned());
            }
        }
        children
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Read a whole file; `None` if the name is not in the namespace.
    pub fn read(&self, name: &str) -> Option<io::Result<Vec<u8>>> {
        let location = self.files.get(&normalize(name).ok()?)?;
        Some(match location {
            Location::Loose(path) => std::fs::read(path),
            Location::Archive { archive, offset, length } => (|| {
                let mut file = File::open(&self.archives[*archive])?;
                file.seek(SeekFrom::Start(*offset))?;
                let mut bytes = vec![0; *length as usize];
                file.read_exact(&mut bytes)?;
                Ok(bytes)
            })(),
        })
    }
}

/// The installation `RIZZARDRY_RETAIL` names, for retail tests; panics without one.
#[cfg(test)]
pub fn retail_files() -> RetailFiles {
    let root = std::env::var("RIZZARDRY_RETAIL")
        .expect("retail tests need RIZZARDRY_RETAIL set to a Wizardry 8 installation");
    RetailFiles::open(&root).unwrap_or_else(|e| panic!("cannot open RIZZARDRY_RETAIL={root}: {e}"))
}
