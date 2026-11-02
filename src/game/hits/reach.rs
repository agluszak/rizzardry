//! Who a burst reaches: the bodies within a radius of a point, in sight of
//! where it went off, and on the side it is against.

use bevy::prelude::*;

use super::Body;
use crate::content::species::MonsterCatalog;
use crate::game::factions::Factions;
use crate::game::party::roster::PartyRoster;
use crate::game::world::ActorPose;
use crate::game::world::hostility::Stance;
use crate::game::world::identify::MonsterGroup;
use crate::game::world::level::{Level, LevelOwned};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;

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
}

/// The living bodies within `radius` of `centre` that `origin` has a clear
/// line to: the party's members, when the party stands within it, then the
/// targetable monsters in their placement order.
#[allow(clippy::too_many_arguments)]
pub fn radius(
    (centre, origin, radius): (Vec3, Vec3, f32),
    recipients: Recipients,
    party: Option<(Vec3, &PartyRoster)>,
    bodies: &mut Query<Body>,
    monsters: &Query<(Entity, ActorPose, &LevelOwned), With<Monster>>,
    levels: &Query<(), With<Level>>,
    (species, sides): (&MonsterCatalog, (&Query<&MonsterGroup>, &Factions, u64)),
    obstacles: &WorldObstacles,
) -> Vec<Entity> {
    if radius <= 0.0 {
        return vec![];
    }
    let clear = |to: Vec3| obstacles.clear(origin, to);
    let mut victims = Vec::new();
    if let Some((eye, roster)) = party
        && crate::game::party::body::PartyBody::centre(eye).distance(centre) <= radius
        && clear(eye)
    {
        for member in roster.iter() {
            let body = bodies.get_mut(member).unwrap();
            if !body.is_dead() && recipients.admits(body.side(species, sides)) {
                victims.push(member);
            }
        }
    }
    let mut found: Vec<_> = monsters
        .iter()
        .filter(|(_, _, owner)| levels.contains(owner.0))
        .filter_map(|(entity, at, _)| {
            let body = bodies.get_mut(entity).ok()?;
            let monster = body.monster.as_ref()?;
            (!body.is_dead()
                && !species[monster.species].untargetable
                && recipients.admits(body.side(species, sides))
                && at.translation().distance(centre) - monster.body.radius <= radius
                && clear(at.translation()))
            .then_some((monster.location, entity))
        })
        .collect();
    found.sort_unstable();
    victims.extend(found.into_iter().map(|(_, entity)| entity));
    victims
}
