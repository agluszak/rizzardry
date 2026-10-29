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

/// The player accepts a note draft.
#[derive(Message, Debug)]
pub struct CommitNote(pub NoteDraft);

pub fn commit_notes(world: &mut World) {
    let requests: Vec<_> = world.resource_mut::<Messages<CommitNote>>().drain().collect();
    for CommitNote(draft) in requests {
        if let Err(why) = commit_note(world, draft) {
            debug!("note refused: {why:?}");
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
        // Retail stores a note as a terminated wide string: at most 39 UTF-16
        // units, none of them the terminator. Which of them the HUD font can
        // draw is the editor's concern.
        if note.text.encode_utf16().count() > MAX_NOTE_LENGTH
            || note.text.contains('\0')
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

    /// Whether another note fits.
    pub fn has_room(&self) -> bool {
        self.notes.len() < MAX_NOTES
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
        let note = |text: String| MapNote { position: Vec2::ZERO, layer: 0, text };
        let too_long = format!("{}\u{1f600}", "a".repeat(MAX_NOTE_LENGTH - 1));
        assert_eq!(notes.commit(None, note(too_long)), Err(NoteRefusal::InvalidText));
        assert_eq!(notes.commit(None, note("a\0b".into())), Err(NoteRefusal::InvalidText));
        assert!(notes.notes.is_empty());
        // Storage takes text the HUD font cannot draw.
        notes.commit(None, note(format!("{}\u{1f600}", "a".repeat(MAX_NOTE_LENGTH - 2)))).unwrap();
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
