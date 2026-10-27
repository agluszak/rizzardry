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

/// A draft belongs to one editing session and map. The index is bounded by
/// that session. The expected committed value prevents delayed confirmation
/// from retargeting an entry that was removed or changed in the meantime.
#[derive(Debug)]
pub struct NoteDraft {
    pub level: Entity,
    pub original: Option<(usize, MapNote)>,
    pub note: MapNote,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NoteRefusal {
    InactiveMap,
    MissingNote,
    Full,
    InvalidText,
}

impl MapNote {
    pub fn type_char(&mut self, c: char) {
        if self.text.encode_utf16().count() + c.len_utf16() <= MAX_NOTE_LENGTH
            && crate::content::ui::glyph_index(c).is_some()
        {
            self.text.push(c);
        }
    }
}

/// Commit an identified map's draft. Departure/cancellation cannot capture
/// unfinished editing in the campaign's notes or a dormant level record.
pub fn commit_note(world: &mut World, draft: NoteDraft) -> Result<(), NoteRefusal> {
    use crate::game::world::level::Level;
    if world.get::<Level>(draft.level).is_none() {
        return Err(NoteRefusal::InactiveMap);
    }
    let notes = world.get_mut::<MapNotes>(draft.level);
    if let Some((index, expected)) = &draft.original
        && notes.as_ref().and_then(|notes| notes.notes.get(*index)) != Some(expected)
    {
        return Err(NoteRefusal::MissingNote);
    }
    notes
        .ok_or(NoteRefusal::InactiveMap)?
        .commit(draft.original.map(|(index, _)| index), draft.note)
}

impl MapNotes {
    pub fn commit(&mut self, original: Option<usize>, note: MapNote) -> Result<(), NoteRefusal> {
        if note.text.encode_utf16().count() > MAX_NOTE_LENGTH
            || note.text.chars().any(|c| crate::content::ui::glyph_index(c).is_none())
            || !note.position.is_finite()
        {
            return Err(NoteRefusal::InvalidText);
        }
        if let Some(index) = original {
            let current = self.notes.get_mut(index).ok_or(NoteRefusal::MissingNote)?;
            if note.text.is_empty() {
                self.notes.remove(index);
            } else {
                *current = note;
            }
        } else if !note.text.is_empty() {
            if self.notes.len() >= MAX_NOTES {
                return Err(NoteRefusal::Full);
            }
            self.notes.push(note);
        }
        Ok(())
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
        notes.commit(None, MapNote { position: Vec2::ZERO, layer: 0, text: "U".into() }).unwrap();
        notes.commit(None, MapNote { position: Vec2::ZERO, layer: 1, text: "D".into() }).unwrap();
        let (upstairs, downstairs) = (0, 1);
        assert_eq!(notes.nearest(Vec2::ZERO, 1.0, 0), Some(upstairs));
        assert_eq!(notes.nearest(Vec2::ZERO, 1.0, 1), Some(downstairs));
        assert_eq!(notes.nearest(Vec2::ZERO, 1.0, 2), None);
    }

    #[test]
    fn notes_keep_retail_limits() {
        let mut notes = MapNotes::default();
        let mut draft = MapNote { position: Vec2::ZERO, layer: 0, text: String::new() };
        for _ in 0..50 {
            draft.type_char('a');
        }
        draft.type_char('\u{1f600}');
        assert_eq!(draft.text.len(), MAX_NOTE_LENGTH);
        assert!(notes.notes.is_empty(), "editing is not a committed note");
        notes.commit(None, draft).unwrap();
        for i in 1..MAX_NOTES {
            notes
                .commit(
                    None,
                    MapNote { position: Vec2::splat(i as f32), layer: 0, text: "N".into() },
                )
                .unwrap();
        }
        let extra = MapNote { position: Vec2::ZERO, layer: 0, text: "N".into() };
        assert_eq!(notes.commit(None, extra), Err(NoteRefusal::Full));
        assert_eq!(notes.nearest(Vec2::new(3.2, 3.0), 1.0, 0), Some(3));
        assert_eq!(notes.nearest(Vec2::new(-5.0, 0.0), 1.0, 0), None);
    }
}
