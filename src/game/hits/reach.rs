//! Who a burst reaches: the bodies within a radius of a point, in sight of
//! where it went off, and on the side it is against.

use bevy::prelude::*;

use crate::game::characters::Vitals;
use crate::game::party::roster::PartyRoster;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::{Side, Stance};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Untargetable};

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
/// line to, in burst order: the party's members by seat, then monsters by
/// placement.
pub fn within(
    centre: Vec3,
    origin: Vec3,
    radius: f32,
    recipients: Recipients,
    bodies: &Query<(Entity, &Vitals, &Side, &Footing, Option<&Monster>), Without<Untargetable>>,
    roster: &PartyRoster,
    obstacles: &WorldObstacles,
) -> Vec<Entity> {
    if radius <= 0.0 {
        return vec![];
    }
    let mut taken: Vec<((u8, i32), Entity)> = bodies
        .iter()
        .filter(|(_, vitals, side, ..)| !vitals.is_dead() && recipients.admits(side.0))
        .filter(|(.., at, _)| at.centre.distance(centre) - at.radius <= radius)
        .filter(|(.., at, _)| obstacles.clear(origin, at.centre))
        .map(|(entity, .., monster)| {
            let order = match roster.get_index_of(&entity) {
                Some(seat) => (0, seat as i32),
                None => (1, monster.map_or(i32::MAX, |monster| monster.location)),
            };
            (order, entity)
        })
        .collect();
    taken.sort_unstable_by_key(|(order, _)| *order);
    taken.into_iter().map(|(_, entity)| entity).collect()
}
