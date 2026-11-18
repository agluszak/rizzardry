//! What a blow sounds like (retail `HitSounds.txt`, `Combat Sound.cpp`): a
//! swing by the weapon's sound class, and a blow landing by the weapon's
//! class and what is struck is made of.

use bevy::prelude::*;

/// The sounds of blows, by name under the game's combat sounds.
#[derive(Asset, TypePath, Debug, Default)]
pub struct HitSounds {
    /// A swing, by the weapon's sound class.
    pub swings: Vec<String>,
    /// A blow landing, by the weapon's sound class and then the material
    /// struck.
    pub impacts: Vec<Vec<Option<String>>>,
}

impl HitSounds {
    /// What a weapon of a class striking a material sounds like: the
    /// class's own, the first class's where it has none, or a plain hit
    /// past the table (retail `GetMaterialImpactSound`).
    pub fn impact(&self, class: u8, material: u8) -> Option<&str> {
        let (class, material) = (usize::from(class), usize::from(material));
        let Some(row) = self.impacts.get(class).filter(|row| material < row.len()) else {
            return Some("HIT");
        };
        let first = || self.impacts.first()?.get(material)?.as_deref();
        row[material].as_deref().or_else(first)
    }
}
