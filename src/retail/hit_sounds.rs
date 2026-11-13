//! The combat sound table (`data/databases/hitsounds.txt`, retail
//! `LoadHitSoundDatabase`): 38 swing sounds, then for each of 12 materials,
//! after a `#` line, 28 impact sounds by weapon class. Anything after an
//! asterisk is a comment.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;

pub use crate::content::hit_sounds::HitSounds;

/// Swing sounds, and impact rows (weapon classes) per material column.
const SWINGS: usize = 38;
const CLASSES: usize = 28;
const MATERIALS: usize = 12;

#[derive(Default, TypePath)]
pub struct HitSoundsLoader;

impl AssetLoader for HitSoundsLoader {
    type Asset = HitSounds;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<HitSounds, std::io::Error> {
        let mut text = Vec::new();
        reader.read_to_end(&mut text).await?;
        Ok(read_hit_sounds(&String::from_utf8_lossy(&text)))
    }

    fn extensions(&self) -> &[&str] {
        &["txt"]
    }
}

pub fn read_hit_sounds(text: &str) -> HitSounds {
    let uncommented = |line: &str| line.split('*').next().unwrap_or("").trim().to_owned();
    let mut lines = text.lines();
    let mut swings = Vec::new();
    for line in lines.by_ref() {
        let line = uncommented(line.split('#').next().unwrap_or(""));
        if !line.is_empty() {
            swings.push(line);
        }
        if swings.len() == SWINGS {
            break;
        }
    }
    let mut impacts = vec![vec![None; MATERIALS]; CLASSES];
    let (mut column, mut row) = (None::<usize>, 0);
    for line in lines {
        if line.contains('#') {
            column = Some(column.map_or(0, |c| c + 1));
            row = 0;
            continue;
        }
        let line = uncommented(line);
        if line.is_empty() {
            continue;
        }
        if let Some(column) = column.filter(|&c| c < MATERIALS)
            && row < CLASSES
        {
            impacts[row][column] = Some(line);
        }
        row += 1;
    }
    HitSounds { swings, impacts }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped table reads whole: every swing, and a sound for each
    /// class striking each material, the first class's standing in for any
    /// that has none.
    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_hit_sounds_read_whole() {
        let files = crate::retail::files::retail_files();
        let data = files.read("data/databases/hitsounds.txt").unwrap().unwrap();
        let table = read_hit_sounds(&String::from_utf8_lossy(&data));
        assert_eq!(table.swings.len(), SWINGS);
        for class in 0..CLASSES as u8 {
            for material in 0..MATERIALS as u8 {
                assert!(table.impact(class, material).is_some(), "class {class} on {material}");
            }
        }
    }
}
