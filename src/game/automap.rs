//! The party's map notes: short texts the player pins to the automap.
//!
//! Retail keeps them per level in the save's `AUTO` chunk
//! (`src/world/automap.cpp`): at most 200, each at most 39 characters plus
//! its terminator.

use bevy::prelude::*;

pub const MAX_NOTES: usize = 200;
/// Characters, not counting the terminator retail stores.
pub const MAX_NOTE_LENGTH: usize = 39;

#[derive(Debug, Clone, PartialEq)]
pub struct MapNote {
    /// World x and z, in metres.
    pub position: Vec2,
    pub layer: usize,
    pub text: String,
}

/// The notes pinned on a level's map, on its root (retail saves them in
/// each level's state, the `AUTO` chunk).
#[derive(Component, Clone, Debug, Default)]
pub struct MapNotes {
    pub notes: Vec<MapNote>,
}

impl MapNotes {
    /// Pin an empty note; none when the map is full.
    pub fn add(&mut self, position: Vec2, layer: usize) -> Option<usize> {
        if self.notes.len() >= MAX_NOTES {
            return None;
        }
        self.notes.push(MapNote { position, layer, text: String::new() });
        Some(self.notes.len() - 1)
    }

    /// Type a character into a note, within retail's length and its font.
    pub fn type_char(&mut self, note: usize, c: char) {
        if let Some(note) = self.notes.get_mut(note)
            && note.text.encode_utf16().count() < MAX_NOTE_LENGTH
            && crate::retail::stci::glyph_index(c).is_some()
        {
            note.text.push(c);
        }
    }

    /// The note nearest a point, within `reach` metres.
    pub fn nearest(&self, point: Vec2, reach: f32, layer: usize) -> Option<usize> {
        self.notes
            .iter()
            .enumerate()
            .filter(|(_, note)| note.layer == layer)
            .map(|(i, note)| (i, note.position.distance(point)))
            .filter(|&(_, distance)| distance <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_on_overlapping_floors_do_not_select_or_erase_each_other() {
        let mut notes = MapNotes::default();
        let upstairs = notes.add(Vec2::ZERO, 0).unwrap();
        let downstairs = notes.add(Vec2::ZERO, 1).unwrap();
        notes.type_char(upstairs, 'U');
        notes.type_char(downstairs, 'D');
        assert_eq!(notes.nearest(Vec2::ZERO, 1.0, 0), Some(upstairs));
        assert_eq!(notes.nearest(Vec2::ZERO, 1.0, 1), Some(downstairs));
        assert_eq!(notes.nearest(Vec2::ZERO, 1.0, 2), None);
        let erased = notes.nearest(Vec2::ZERO, 1.0, 1).unwrap();
        notes.notes.remove(erased);
        assert_eq!(notes.notes.len(), 1);
        assert_eq!(notes.notes[0].text, "U");
    }

    #[test]
    fn notes_keep_retail_limits() {
        let mut notes = MapNotes::default();
        let note = notes.add(Vec2::ZERO, 0).unwrap();
        for _ in 0..50 {
            notes.type_char(note, 'a');
        }
        notes.type_char(note, '\u{1f600}');
        assert_eq!(notes.notes[note].text.len(), MAX_NOTE_LENGTH);
        notes.notes[note].text.clear();
        notes.type_char(note, '\u{1f600}');
        assert!(notes.notes[note].text.is_empty());
        for i in 1..MAX_NOTES {
            notes.add(Vec2::splat(i as f32), 0).unwrap();
        }
        assert!(notes.add(Vec2::ZERO, 0).is_none());
        assert_eq!(notes.nearest(Vec2::new(3.2, 3.0), 1.0, 0), Some(3));
        assert_eq!(notes.nearest(Vec2::new(-5.0, 0.0), 1.0, 0), None);
    }
}
