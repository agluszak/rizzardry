//! Who a burst reaches: the bodies within a radius of a point, in sight of
//! where it went off, and on the side it is against.

use bevy::prelude::*;

use crate::game::characters::Vitals;
use crate::game::party::roster::PartyRoster;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::Stance;
use crate::game::world::obstacles::WorldObstacles;

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

/// The living bodies within `radius` of `centre` that `origin` has a clear
/// line to, in the fight's order: the party's members by seat, then the
/// combatants as they joined, then anyone else.
pub fn within<'a>(
    centre: Vec3,
    origin: Vec3,
    radius: f32,
    recipients: Recipients,
    bodies: impl IntoIterator<Item = (Entity, &'a Vitals, Stance, &'a Footing)>,
    (roster, combatants): (&PartyRoster, &[Entity]),
    obstacles: &WorldObstacles,
) -> Vec<Entity> {
    if radius <= 0.0 {
        return vec![];
    }
    let mut taken: Vec<((u8, usize), Entity)> = bodies
        .into_iter()
        .filter(|(_, vitals, side, _)| !vitals.is_dead() && recipients.admits(*side))
        .filter(|(.., at)| at.centre.distance(centre) - at.radius <= radius)
        .filter(|(.., at)| obstacles.clear(origin, at.centre))
        .map(|(entity, ..)| {
            let order = match (
                roster.get_index_of(&entity),
                combatants.iter().position(|&c| c == entity),
            ) {
                (Some(seat), _) => (0, seat),
                (None, Some(joined)) => (1, joined),
                (None, None) => (2, 0),
            };
            (order, entity)
        })
        .collect();
    taken.sort_by_key(|(order, _)| *order);
    taken.into_iter().map(|(_, entity)| entity).collect()
}
