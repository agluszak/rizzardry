//! Sir-tech library (SLF) archives.
//!
//! An archive is a 532-byte header, the member bytes, then a directory of
//! 280-byte entries at the end of the file. All fields are little endian.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::path::{RetailPathError, normalize};
use crate::bytes::{Truncated, u32_at};

const HEADER_SIZE: u64 = 532;
const ENTRY_SIZE: u64 = 280;

/// One live member of an archive, named relative to the asset root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlfEntry {
    pub name: String,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug)]
pub struct SlfArchive {
    /// The archive's own path prefix, e.g. `levels` for `LEVELS.SLF`.
    pub prefix: String,
    pub entries: Vec<SlfEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum SlfError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("unterminated text field")]
    Unterminated,
    #[error("invalid entry counts (entries {entries}, used {used})")]
    Counts { entries: i32, used: i32 },
    #[error("directory exceeds the archive")]
    DirectoryBounds,
    #[error("entry {name:?} lies outside the archive data")]
    EntryBounds { name: String },
    #[error("live entry count {found} differs from the header's {expected}")]
    LiveCount { found: usize, expected: usize },
    #[error("duplicate entry {0:?}")]
    Duplicate(String),
    #[error(transparent)]
    Path(#[from] RetailPathError),
    #[error(transparent)]
    Truncated(#[from] Truncated),
}

fn text(field: &[u8]) -> Result<&[u8], SlfError> {
    let end = field.iter().position(|&b| b == 0).ok_or(SlfError::Unterminated)?;
    Ok(&field[..end])
}

impl SlfArchive {
    pub fn open(path: &Path) -> Result<Self, SlfError> {
        let mut file = File::open(path)?;
        Self::read(&mut file)
    }

    pub fn read(input: &mut (impl Read + Seek)) -> Result<Self, SlfError> {
        let size = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;
        let mut header = [0u8; HEADER_SIZE as usize];
        input.read_exact(&mut header)?;
        text(&header[..256])?;
        let prefix = String::from_utf8_lossy(text(&header[256..512])?).into_owned();
        let prefix = if prefix.is_empty() { String::new() } else { normalize(&prefix)? };
        let entries = u32_at(&header, 512)? as i32;
        let used = u32_at(&header, 516)? as i32;
        if entries < 0 || used < 0 || used > entries || used > i32::from(u16::MAX) {
            return Err(SlfError::Counts { entries, used });
        }
        let table_size = entries as u64 * ENTRY_SIZE;
        if table_size > size - HEADER_SIZE {
            return Err(SlfError::DirectoryBounds);
        }
        let table_start = size - table_size;
        input.seek(SeekFrom::Start(table_start))?;
        let mut table = vec![0u8; table_size as usize];
        input.read_exact(&mut table)?;

        let mut live = Vec::with_capacity(used as usize);
        for record in table.as_chunks::<{ ENTRY_SIZE as usize }>().0 {
            // ubState: 0 is a live entry; anything else was deleted.
            if record[264] != 0 {
                continue;
            }
            let raw = String::from_utf8_lossy(text(&record[..256])?).into_owned();
            let member = normalize(&raw)?;
            let name = if prefix.is_empty() { member } else { format!("{prefix}/{member}") };
            let offset = u64::from(u32_at(record, 256)?);
            let length = u64::from(u32_at(record, 260)?);
            if offset < HEADER_SIZE || offset > table_start || length > table_start - offset {
                return Err(SlfError::EntryBounds { name });
            }
            live.push(SlfEntry { name, offset, length });
        }
        if live.len() != used as usize {
            return Err(SlfError::LiveCount { found: live.len(), expected: used as usize });
        }
        live.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(pair) = live.windows(2).find(|pair| pair[0].name == pair[1].name) {
            return Err(SlfError::Duplicate(pair[0].name.clone()));
        }
        Ok(SlfArchive { prefix, entries: live })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn archive(prefix: &str, members: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_SIZE as usize];
        out[..4].copy_from_slice(b"TEST");
        out[256..256 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let live = members.iter().filter(|m| m.2 == 0).count() as u32;
        out[512..516].copy_from_slice(&(members.len() as u32).to_le_bytes());
        out[516..520].copy_from_slice(&live.to_le_bytes());
        let mut directory = Vec::new();
        for (name, bytes, state) in members {
            let mut record = vec![0u8; ENTRY_SIZE as usize];
            record[..name.len()].copy_from_slice(name.as_bytes());
            record[256..260].copy_from_slice(&(out.len() as u32).to_le_bytes());
            record[260..264].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            record[264] = *state;
            directory.extend(record);
            out.extend_from_slice(bytes);
        }
        out.extend(directory);
        out
    }

    #[test]
    fn reads_live_entries_under_the_library_prefix() {
        let bytes = archive(
            "Levels\\",
            &[("Monastery\\A.PVL", b"abc", 0), ("old.txt", b"x", 1), ("B.txt", b"", 0)],
        );
        let archive = SlfArchive::read(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(archive.prefix, "levels");
        let names: Vec<_> = archive.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["levels/b.txt", "levels/monastery/a.pvl"]);
        assert_eq!(archive.entries[1].length, 3);
    }

    #[test]
    fn rejects_case_insensitive_duplicates() {
        let bytes = archive("", &[("a.txt", b"1", 0), ("A.TXT", b"2", 0)]);
        assert!(matches!(SlfArchive::read(&mut Cursor::new(bytes)), Err(SlfError::Duplicate(_))));
    }
}
