//! The party's formation: five rows of three seats around the party, which
//! decide who reaches whom in melee (wizardry-8-decomp
//! `FormationAndFacing.cpp`, `Combat Range.cpp`).

use bevy::prelude::*;
use enum_map::{Enum, EnumMap};

use crate::content::species::Reach;

/// A formation row: the side of the party it faces, or its center.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enum)]
pub enum Row {
    Front,
    Right,
    Rear,
    Left,
    Center,
}

impl Row {
    /// The side across the party; the center has none.
    fn opposite(self) -> Option<Row> {
        match self {
            Row::Front => Some(Row::Rear),
            Row::Rear => Some(Row::Front),
            Row::Right => Some(Row::Left),
            Row::Left => Some(Row::Right),
            Row::Center => None,
        }
    }
}

/// Who sits where, by member entity: three seats a row.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct Formation {
    seats: EnumMap<Row, [Option<Entity>; 3]>,
}

impl Formation {
    /// The row a member sits in, if it holds a place.
    pub fn row_of(&self, member: Entity) -> Option<Row> {
        self.place_of(member).map(|(row, _)| row)
    }

    /// Whether anyone sits in a row.
    pub fn occupied(&self, row: Row) -> bool {
        self.seats[row].iter().any(Option::is_some)
    }

    /// Where a member sits: its row and seat (0-2).
    pub fn place_of(&self, member: Entity) -> Option<(Row, usize)> {
        self.seats.iter().find_map(|(row, seats)| {
            seats.iter().position(|&s| s == Some(member)).map(|seat| (row, seat))
        })
    }

    /// Seat a member without a place in the first row with room, front, then
    /// center, then rear (retail `PlaceCharacterInFormation`). Within a row
    /// (`MakeRoomInFormationRow`): the first takes the lead seat; a second
    /// moves the leader to the middle and takes the far seat; a third takes
    /// the lead seat again.
    pub fn seat(&mut self, member: Entity) {
        if self.row_of(member).is_some() {
            return;
        }
        for row in [Row::Front, Row::Center, Row::Rear] {
            let seats = &mut self.seats[row];
            match seats.iter().flatten().count() {
                0 => seats[0] = Some(member),
                1 if seats[0].is_some() => {
                    seats[1] = seats[0].take();
                    seats[2] = Some(member);
                }
                3 => continue,
                _ => match seats.iter_mut().find(|s| s.is_none()) {
                    Some(free) => *free = Some(member),
                    None => continue,
                },
            }
            return;
        }
    }

    /// The dead hold no place.
    pub fn unseat(&mut self, member: Entity) {
        for seat in self.seats.values_mut().flatten() {
            if *seat == Some(member) {
                *seat = None;
            }
        }
    }

    /// How many occupied rows stand between a member and a monster in
    /// `quadrant` (retail `CountRowsBetween`): none in the same row; one when
    /// the monster's row is occupied; for the row opposite it, one more when
    /// that row or the center screens.
    pub fn rows_between(&self, member: Entity, quadrant: Row) -> u32 {
        let Some(row) = self.row_of(member) else { return 0 };
        if row == quadrant {
            return 0;
        }
        let rows = u32::from(self.occupied(quadrant));
        if row.opposite() != Some(quadrant) {
            return rows;
        }
        if rows != 0 || self.occupied(Row::Center) { rows + 1 } else { rows }
    }

    /// The reach left to a member against a monster in `quadrant` (retail
    /// `CloseFormationGap`): each row between shortens touch and short
    /// reach a step, past touch to none; long and extreme reach are not
    /// shortened.
    pub fn reach(&self, member: Entity, quadrant: Row, reach: Reach) -> Option<Reach> {
        if reach >= Reach::Long {
            return Some(reach);
        }
        let mut reach = Some(reach);
        for _ in 0..self.rows_between(member, quadrant) {
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
    // In the party's frame forward is -Z and right is +X.
    let local = (Quat::from_rotation_y(-yaw) * (to - from)).xz();
    let bearing = Vec2::NEG_Y.angle_to(local).to_degrees().rem_euclid(360.0);
    match ((bearing + 45.0) % 360.0 / 90.0) as u32 {
        0 => Row::Front,
        1 => Row::Right,
        2 => Row::Rear,
        _ => Row::Left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn six() -> (Formation, Vec<Entity>) {
        let mut world = World::new();
        let members: Vec<_> = (0..6).map(|_| world.spawn_empty().id()).collect();
        let mut formation = Formation::default();
        for &member in &members {
            formation.seat(member);
        }
        (formation, members)
    }

    #[test]
    fn six_members_fill_the_front_then_the_center() {
        let (formation, members) = six();
        // The first moved aside for the second and the third took the lead.
        assert_eq!(formation.place_of(members[0]), Some((Row::Front, 1)));
        assert_eq!(formation.place_of(members[1]), Some((Row::Front, 2)));
        assert_eq!(formation.place_of(members[2]), Some((Row::Front, 0)));
        assert_eq!(formation.row_of(members[0]), Some(Row::Front));
        assert_eq!(formation.row_of(members[2]), Some(Row::Front));
        assert_eq!(formation.row_of(members[3]), Some(Row::Center));
        assert_eq!(formation.row_of(members[5]), Some(Row::Center));
    }

    #[test]
    fn rows_in_between_shorten_close_reach() {
        let (formation, members) = six();
        // A monster in front: the front reaches, the center needs a
        // polearm, long reach is unaffected.
        assert_eq!(formation.reach(members[0], Row::Front, Reach::Touch), Some(Reach::Touch));
        assert_eq!(formation.reach(members[4], Row::Front, Reach::Touch), None);
        assert_eq!(formation.reach(members[4], Row::Front, Reach::Short), Some(Reach::Touch));
        assert_eq!(formation.reach(members[4], Row::Front, Reach::Long), Some(Reach::Long));
        // From behind the rear row is empty: the front is two rows away and
        // the center screens it.
        assert_eq!(formation.rows_between(members[0], Row::Rear), 1);
        assert_eq!(formation.rows_between(members[4], Row::Rear), 0);
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
