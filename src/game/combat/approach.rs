//! Monsters closing in on the party during a fight. A monster in the fight
//! uses its existing world-owned walking body to head for the party
//! through the navmesh until one of its attacks reaches; the dead and those
//! in reach stand. Retail moves monsters as a scheduled action within the
//! round (advance and charge); here they close in continuously, a
//! deliberate simplification until those actions exist.

use crate::game::world::ActorPose;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;

use super::Combatant;
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::data::GameData;
use crate::game::party::body::PartyBody;
use crate::game::world::navigation::Walker;
use crate::game::world::population::Monster;

/// Stop this far inside an attack's reach.
const REACH_MARGIN: f32 = 0.3;

/// Whether this living walker still needs to close to attack range. Combat's
/// passive-round decision and navigation use the same current actor state.
pub(super) fn needs_approach(
    monster: &Monster,
    origin: Vec3,
    walker: Option<&Walker>,
    party_eye: Vec3,
    species: &MonsterCatalog,
) -> bool {
    let Some(walker) = walker else { return false };
    if monster.is_dead() || walker.speed <= 0.0 {
        return false;
    }
    let Some(kind) = species.0.get(monster.species as usize) else { return false };
    let reach = kind.attacks.iter().map(|a| a.reach).max().unwrap_or(Reach::Touch);
    let feet = crate::game::world::navigation::feet(origin, Some(walker));
    super::distance_to_party(party_eye, monster, feet) > reach.metres() - REACH_MARGIN
}

/// Head for the party while no attack reaches it; stand otherwise and when
/// dead. Only combatants: the fight owns their movement while it lasts and
/// stops it when it ends; others are none of its business.
pub fn close_in(
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut combatants: Query<(&Monster, ActorPose, &Walker, &mut AgentTarget3d), With<Combatant>>,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
) {
    let Some(catalog) = data.and_then(|d| species.get(&d.species)) else { return };
    let Ok((party, party_at)) = party.single() else { return };
    let party_eye = PartyBody::eye(party_at);
    for (monster, at, walker, mut target) in &mut combatants {
        let wanted = if needs_approach(monster, at.translation(), Some(walker), party_eye, catalog)
        {
            AgentTarget3d::Entity(party)
        } else {
            AgentTarget3d::None
        };
        if !matches!((&*target, &wanted), (AgentTarget3d::None, AgentTarget3d::None))
            && !matches!((&*target, &wanted), (AgentTarget3d::Entity(a), AgentTarget3d::Entity(b)) if a == b)
        {
            *target = wanted;
        }
    }
}
