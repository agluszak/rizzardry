//! The party's map notes: short texts the player pins to the automap.
//!
//! Retail keeps them per level in the save's `AUTO` chunk
//! (`src/world/automap.cpp`): at most 200, each at most 39 characters plus
//! its terminator.

use bevy::prelude::*;

pub const MAX_NOTES: usize = 200;
/// UTF-16 code units, not counting the terminator retail stores.
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

/// A note being written on a level's map: a new one, or the one at
/// `original` retyped. The map only starts one while the notes have room.
#[derive(Debug)]
pub struct NoteDraft {
    pub level: Entity,
    pub original: Option<usize>,
    pub note: MapNote,
}

/// The player accepts a note draft.
#[derive(Message, Debug)]
pub struct CommitNote(pub NoteDraft);

/// A note too long for retail's notes (the editor counts characters,
/// retail stores UTF-16).
#[derive(Debug, PartialEq, Eq)]
pub struct NoteTooLong;

/// Pin accepted drafts to their level's map.
pub fn commit_notes(mut requests: MessageReader<CommitNote>, mut levels: Query<&mut MapNotes>) {
    for CommitNote(draft) in requests.read() {
        let mut notes = levels.get_mut(draft.level).unwrap();
        if let Err(why) = notes.commit(draft.original, draft.note.clone()) {
            debug!("note refused: {why:?}");
        }
    }
}

impl MapNotes {
    /// Pin a new note, or retype or (committed empty) erase the one at
    /// `original`.
    pub fn commit(&mut self, original: Option<usize>, note: MapNote) -> Result<(), NoteTooLong> {
        if note.text.encode_utf16().count() > MAX_NOTE_LENGTH {
            return Err(NoteTooLong);
        }
        match original {
            Some(index) if note.text.is_empty() => {
                self.notes.remove(index);
            }
            Some(index) => self.notes[index] = note,
            None if !note.text.is_empty() => self.notes.push(note),
            None => {}
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
