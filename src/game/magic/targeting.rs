//! Radius eligibility and visibility are one operation shared by casts,
//! secondary explosions and moving auras, separate from damage application.
use super::{Field, Victim};
use crate::game::world::hostility::Stance;
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub(crate) enum Recipients {
    Opponents(Stance),
    Everyone,
}
impl Recipients {
    fn admits(self, side: Stance) -> bool {
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
    let obstacles = &world.visibility;
    if radius <= 0.0 {
        return vec![];
    }
    let clear = |to: Vec3| obstacles.clear(origin, to);
    let mut victims = Vec::new();
    if party_at.is_some_and(|eye| {
        let centre_of_party = crate::game::party::body::PartyBody::centre(eye);
        centre_of_party.distance(centre) <= radius && clear(eye)
    }) {
        victims.extend(
            world
                .party
                .iter()
                .filter(|&member| {
                    world.members.get(member).is_ok_and(|(_, v, ..)| {
                        !v.is_dead()
                            && recipients.admits(crate::game::world::hostility::own_stance(
                                Stance::Friendly,
                                v.conditions.has(crate::game::characters::Condition::Turncoat),
                            ))
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
            (world.monsters.targetable(entity)
                && recipients.admits(world.monsters.stance(entity, world.now()))
                && at.translation().distance(centre) - body.body.radius <= radius
                && clear(at.translation()))
            .then_some((body.location, entity))
        })
        .collect();
    found.sort_unstable();
    victims.extend(found.into_iter().map(|(_, entity)| Victim::Monster(entity)));
    victims
}
