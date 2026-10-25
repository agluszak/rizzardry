//! The party's formation: five rows of three seats around the party, which
//! decide who reaches whom in melee (wizardry-8-decomp
//! `FormationAndFacing.cpp`, `Combat Range.cpp`).

use bevy::prelude::*;
use num_enum::{IntoPrimitive, TryFromPrimitive};

use crate::content::species::Reach;

/// A formation row, by the side of the party it faces, in retail's order
/// (front, right, rear, left, center): its number is retail's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(usize)]
pub enum Row {
    Front,
    Right,
    Rear,
    Left,
    Center,
}

const SEATS: usize = 3;

/// The row a seat table index is (0..5).
fn row_at(index: usize) -> Row {
    Row::try_from(index).expect("five rows")
}

/// Who sits where, by party slot.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct Formation {
    seats: [[Option<usize>; SEATS]; 5],
}

impl Formation {
    /// The row a party slot sits in, if it holds a place.
    pub fn row_of(&self, slot: usize) -> Option<Row> {
        self.seats.iter().position(|row| row.contains(&Some(slot))).map(row_at)
    }

    /// Whether anyone sits in a row.
    pub fn occupied(&self, row: Row) -> bool {
        self.seats[usize::from(row)].iter().any(Option::is_some)
    }

    /// Where a slot sits: its row and seat (0-2).
    pub fn place_of(&self, slot: usize) -> Option<(Row, usize)> {
        self.seats.iter().enumerate().find_map(|(r, row)| {
            row.iter().position(|&s| s == Some(slot)).map(|seat| (row_at(r), seat))
        })
    }

    /// The direction a slot faces: its row's side, ahead for the center
    /// (retail `SetFormationPosition`), as 0 front, 1 right, 2 rear, 3 left.
    pub fn facing(row: Row) -> usize {
        match row {
            Row::Center => 0,
            row => usize::from(row),
        }
    }

    /// Seat a slot without a place in the first row with room, front, then
    /// center, then rear (retail `PlaceCharacterInFormation`). Within a row
    /// (`MakeRoomInFormationRow`): the first takes the lead seat; a second
    /// moves the leader to the middle and takes the far seat; a third takes
    /// the lead seat again.
    pub fn seat(&mut self, slot: usize) {
        if self.row_of(slot).is_some() {
            return;
        }
        for row in [Row::Front, Row::Center, Row::Rear] {
            let seats = &mut self.seats[usize::from(row)];
            match seats.iter().flatten().count() {
                0 => seats[0] = Some(slot),
                1 if seats[0].is_some() => {
                    seats[1] = seats[0].take();
                    seats[2] = Some(slot);
                }
                3 => continue,
                _ => match seats.iter_mut().find(|s| s.is_none()) {
                    Some(free) => *free = Some(slot),
                    None => continue,
                },
            }
            return;
        }
    }

    /// The dead hold no place.
    pub fn unseat(&mut self, slot: usize) {
        for seat in self.seats.iter_mut().flatten() {
            if *seat == Some(slot) {
                *seat = None;
            }
        }
    }

    /// How many occupied rows stand between a slot and a monster in
    /// `quadrant` (retail `CountRowsBetween`): none in the same row; one when
    /// the monster's row is occupied; for the row opposite it, one more when
    /// that row or the center screens.
    pub fn rows_between(&self, slot: usize, quadrant: Row) -> u32 {
        let Some(row) = self.row_of(slot) else { return 0 };
        if row == quadrant {
            return 0;
        }
        let rows = u32::from(self.occupied(quadrant));
        if row == Row::Center || usize::from(row).abs_diff(usize::from(quadrant)) != 2 {
            return rows;
        }
        if rows != 0 || self.occupied(Row::Center) { rows + 1 } else { rows }
    }

    /// The reach left to a slot against a monster in `quadrant` (retail
    /// `CloseFormationGap`): each row between shortens touch and short
    /// reach a step, past touch to none; long and extreme reach are not
    /// shortened.
    pub fn reach(&self, slot: usize, quadrant: Row, reach: Reach) -> Option<Reach> {
        if reach >= Reach::Long {
            return Some(reach);
        }
        let mut reach = Some(reach);
        for _ in 0..self.rows_between(slot, quadrant) {
            reach = match reach {
                Some(Reach::Short) => Some(Reach::Touch),
                _ => None,
            };
        }
        reach
    }
}

/// The side of the party something at `to` is on, seen from `from` facing
/// `yaw` (retail `GetQuadrantForPosition`: the bearing from the party's
/// facing, clockwise, in quarters centred on the four sides).
pub fn quadrant(from: Vec3, yaw: f32, to: Vec3) -> Row {
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let d = (to - from).with_y(0.0);
    let bearing = d.dot(right).atan2(d.dot(forward)).to_degrees().rem_euclid(360.0);
    row_at((((bearing + 45.0) % 360.0) / 90.0) as usize)
}

/// Keep the formation in step with the roster: the living hold a place,
/// the dead do not.
pub fn seat_party(
    roster: Res<super::roster::PartyRoster>,
    members: Query<&crate::game::characters::Character>,
    mut formation: ResMut<Formation>,
) {
    let mut wanted = formation.clone();
    for (slot, &member) in roster.0.iter().enumerate() {
        match members.get(member) {
            Ok(character) if !character.is_dead() => wanted.seat(slot),
            _ => wanted.unseat(slot),
        }
    }
    if wanted != *formation {
        *formation = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn six() -> Formation {
        let mut formation = Formation::default();
        for slot in 0..6 {
            formation.seat(slot);
        }
        formation
    }

    #[test]
    fn six_members_fill_the_front_then_the_center() {
        let formation = six();
        // The first moved aside for the second and the third took the lead.
        assert_eq!(formation.place_of(0), Some((Row::Front, 1)));
        assert_eq!(formation.place_of(1), Some((Row::Front, 2)));
        assert_eq!(formation.place_of(2), Some((Row::Front, 0)));
        assert_eq!(formation.row_of(0), Some(Row::Front));
        assert_eq!(formation.row_of(2), Some(Row::Front));
        assert_eq!(formation.row_of(3), Some(Row::Center));
        assert_eq!(formation.row_of(5), Some(Row::Center));
    }

    #[test]
    fn rows_in_between_shorten_close_reach() {
        let formation = six();
        // A monster in front: the front reaches, the center needs a
        // polearm, long reach is unaffected.
        assert_eq!(formation.reach(0, Row::Front, Reach::Touch), Some(Reach::Touch));
        assert_eq!(formation.reach(4, Row::Front, Reach::Touch), None);
        assert_eq!(formation.reach(4, Row::Front, Reach::Short), Some(Reach::Touch));
        assert_eq!(formation.reach(4, Row::Front, Reach::Long), Some(Reach::Long));
        // From behind the rear row is empty: the front is two rows away and
        // the center screens it.
        assert_eq!(formation.rows_between(0, Row::Rear), 1);
        assert_eq!(formation.rows_between(4, Row::Rear), 0);
    }

    #[test]
    fn quadrants_are_quarters_around_the_facing() {
        let at = Vec3::ZERO;
        assert_eq!(quadrant(at, 0.0, Vec3::new(0.0, 0.0, -5.0)), Row::Front);
        assert_eq!(quadrant(at, 0.0, Vec3::new(5.0, 0.0, 0.0)), Row::Right);
        assert_eq!(quadrant(at, 0.0, Vec3::new(0.0, 0.0, 5.0)), Row::Rear);
        assert_eq!(quadrant(at, 0.0, Vec3::new(-5.0, 0.0, 0.0)), Row::Left);
        // Facing +X (yaw −90°), +X is ahead.
        assert_eq!(
            quadrant(at, -std::f32::consts::FRAC_PI_2, Vec3::new(5.0, 0.0, 0.0)),
            Row::Front
        );
    }
}
