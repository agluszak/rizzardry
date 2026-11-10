//! Who a burst reaches: the bodies within a radius of a point, in sight of
//! where it went off, and on the side it is against.

use bevy::prelude::*;

use crate::game::world::hostility::Stance;

/// Whom a burst may take: those standing against its source's side, or
/// anyone when it has no source.
#[derive(Clone, Copy, Debug)]
pub enum Recipients {
    Opponents(Stance),
    Everyone,
}

impl Recipients {
    pub fn admits(self, side: Stance) -> bool {
        match self {
            Self::Opponents(source) => source.opposes(side),
            Self::Everyone => true,
        }
    }

    /// The other side's: a spell turned back on its source.
    pub fn reversed(self) -> Self {
        match self {
            Self::Opponents(Stance::Hostile) => Self::Opponents(Stance::Friendly),
            Self::Opponents(Stance::Friendly) => Self::Opponents(Stance::Hostile),
            other => other,
        }
    }
}

/// A body a burst may reach, as the burst finds it.
#[derive(Clone, Copy, Debug)]
pub struct Struck {
    pub id: Entity,
    pub dead: bool,
    /// The side it is on now.
    pub stance: Stance,
    /// Where a blow or a missile strikes it, and how far its body spreads.
    pub centre: Vec3,
    pub radius: f32,
    /// Its place in the fight's order: the party's members by seat, then the
    /// combatants as they joined, then anyone else.
    pub order: (u8, usize),
    /// A clear line from where the burst went off to its centre.
    pub clear: bool,
}

/// Where a body stands in the fight's order: the party's members by seat, then
/// the combatants as they joined, then anyone else.
pub fn order(seat: Option<usize>, joined: Option<usize>) -> (u8, usize) {
    match (seat, joined) {
        (Some(seat), _) => (0, seat),
        (None, Some(joined)) => (1, joined),
        (None, None) => (2, 0),
    }
}

/// The living bodies within `radius` of `centre` that the burst has a clear
/// line to and may take, in the fight's order.
pub fn within(
    centre: Vec3,
    radius: f32,
    recipients: Recipients,
    bodies: impl IntoIterator<Item = Struck>,
) -> Vec<Entity> {
    if radius <= 0.0 {
        return vec![];
    }
    let mut taken: Vec<Struck> = bodies
        .into_iter()
        .filter(|body| !body.dead && recipients.admits(body.stance))
        .filter(|body| body.centre.distance(centre) - body.radius <= radius)
        .filter(|body| body.clear)
        .collect();
    taken.sort_by_key(|body| body.order);
    taken.into_iter().map(|body| body.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    fn at(index: u32, x: f32, stance: Stance, order: (u8, usize)) -> Struck {
        Struck {
            id: id(index),
            dead: false,
            stance,
            centre: Vec3::new(x, 0.0, 0.0),
            radius: 0.5,
            order,
            clear: true,
        }
    }

    /// The party by seat, then the combatants as they joined, then anyone else.
    #[test]
    fn the_fights_order_is_the_party_by_seat_then_the_combatants_then_the_rest() {
        assert_eq!(order(Some(2), Some(0)), (0, 2));
        assert_eq!(order(Some(0), None), (0, 0));
        assert_eq!(order(None, Some(5)), (1, 5));
        assert_eq!(order(None, None), (2, 0));
        assert!(order(Some(9), None) < order(None, Some(0)));
        assert!(order(None, Some(9)) < order(None, None));
    }

    /// Whom a burst is against: the opponents of a side, or everyone; turned
    /// back it is the other side's.
    #[test]
    fn recipients_are_the_opponents_of_a_side_or_everyone_and_turn_with_it() {
        let against = Recipients::Opponents(Stance::Hostile);
        assert!(against.admits(Stance::Friendly));
        assert!(!against.admits(Stance::Hostile) && !against.admits(Stance::Neutral));
        assert!(against.reversed().admits(Stance::Hostile));
        assert!(!against.reversed().admits(Stance::Friendly));
        let everyone = Recipients::Everyone;
        assert!(
            [Stance::Hostile, Stance::Friendly, Stance::Neutral]
                .into_iter()
                .all(|side| everyone.admits(side) && everyone.reversed().admits(side))
        );
        assert!(!Recipients::Opponents(Stance::Neutral).admits(Stance::Hostile));
    }

    /// A body whose centre is 3 off and whose radius is 0.5 is at 2.5: within a
    /// radius of 2.5, not of 2.4. Only the living, in sight, and admitted, in
    /// the order of the fight whatever the order given.
    #[test]
    fn a_burst_takes_the_living_admitted_bodies_in_sight_within_its_radius_in_order() {
        let against = Recipients::Opponents(Stance::Friendly);
        let hostile = |index, x, order| at(index, x, Stance::Hostile, order);
        let bodies = vec![
            hostile(3, 3.0, (1, 1)),
            hostile(2, 1.0, (1, 0)),
            Struck { dead: true, ..hostile(4, 0.0, (1, 2)) },
            Struck { clear: false, ..hostile(5, 0.0, (1, 3)) },
            at(6, 0.0, Stance::Friendly, (0, 0)),
            hostile(7, 0.0, (0, 1)),
        ];
        assert_eq!(within(Vec3::ZERO, 2.5, against, bodies.clone()), [id(7), id(2), id(3)]);
        assert_eq!(within(Vec3::ZERO, 2.4, against, bodies.clone()), [id(7), id(2)]);
        assert!(within(Vec3::ZERO, 0.0, against, bodies).is_empty());
    }
}
