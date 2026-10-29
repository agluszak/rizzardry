//! Radius eligibility and visibility are one operation shared by casts,
//! secondary explosions and moving auras, separate from damage application.
use super::{Field, Victim};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub(crate) enum Recipients {
    Opponents(crate::game::world::hostility::Stance),
    Everyone,
}
impl Recipients {
    fn admits(self, side: crate::game::world::hostility::Stance) -> bool {
        match self {
            Self::Opponents(source) => source.opposes(side),
            Self::Everyone => true,
        }
    }
}

pub(crate) fn radius(
    world: &Field,
    centre: Vec3,
    origin: Vec3,
    radius: f32,
    party_at: Option<Vec3>,
    recipients: Recipients,
) -> Vec<Victim> {
    let (Some(obstacles), Some(species)) = (world.visibility, world.species) else { return vec![] };
    if radius <= 0.0 {
        return vec![];
    }
    let clear = |to: Vec3| {
        Dir3::new(to - origin).map_or(true, |direction| {
            obstacles.first_hit(origin, direction, to.distance(origin)).is_none()
        })
    };
    let mut victims = Vec::new();
    if party_at.is_some_and(|eye| {
        let centre_of_party = eye - Vec3::Y * crate::game::party::body::EYE_ABOVE_CENTRE;
        centre_of_party.distance(centre) <= radius && clear(eye)
    }) {
        victims.extend(
            world
                .party
                .iter()
                .copied()
                .filter(|&member| {
                    world.members.get(member).is_ok_and(|(_, v, ..)| {
                        !v.is_dead()
                            && recipients.admits(
                                if v.has(crate::game::characters::Condition::Turncoat) {
                                    crate::game::world::hostility::Stance::Hostile
                                } else {
                                    crate::game::world::hostility::Stance::Friendly
                                },
                            )
                    })
                })
                .map(Victim::Member),
        );
    }
    let mut found: Vec<_> = world
        .monsters
        .monsters()
        .iter()
        .filter_map(|(body, at, _, _, entity)| {
            let definition = species.kind(body.species)?;
            (world.monsters.active(entity)
                && !body.is_dead()
                && !definition.untargetable
                && recipients.admits(world.monsters.stance(entity, world.now))
                && at.translation().distance(centre) - body.body.radius <= radius
                && clear(at.translation()))
            .then_some((body.location, entity))
        })
        .collect();
    found.sort_unstable();
    victims.extend(found.into_iter().map(|(_, entity)| Victim::Monster(entity)));
    victims
}
