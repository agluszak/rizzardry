//! Monsters closing in on the party during a fight. A monster in the fight
//! uses its existing world-owned walking body to head for the party
//! through the navmesh until one of its attacks reaches; the dead and those
//! in reach stand. As in retail, moving is a monster's action within the
//! round: at its phase, with nothing in reach, it advances (or charges, when
//! hurt) up to `combat_move_range` × 20 metres, and acts again later in the
//! round (`Combat.cpp:2486-2551, 2855-2909`). The fight's next actor waits
//! for it to arrive or spend its move.

use crate::game::world::ActorPose;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;

use super::{Battle, Combatant};
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::data::GameData;
use crate::game::party::body::PartyBody;
use crate::game::world::navigation::Walker;
use crate::game::world::population::Monster;

/// Stop this far inside an attack's reach.
const REACH_MARGIN: f32 = 0.3;

/// A monster's move this round: how far it may go, and how far its walking
/// has taken it. Retail counts time at walking speed, not ground covered
/// (`Navigator.cpp:2202-2211`), so a blocked monster's move still ends.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct CombatMove {
    pub budget: f32,
    pub moved: f32,
}

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

/// Walk monsters on their move toward the party; the move ends when they
/// arrive, spend it, fall, or the fight ends.
pub fn close_in(
    mut commands: Commands,
    time: Res<Time>,
    battle: Option<Single<&Battle>>,
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut walkers: Query<
        (Entity, &Monster, ActorPose, &Walker, &mut AgentTarget3d, Option<&mut CombatMove>),
        With<Combatant>,
    >,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
) {
    let Some(catalog) = data.and_then(|d| species.get(&d.species)) else { return };
    let Ok((party, party_at)) = party.single() else { return };
    let party_eye = PartyBody::eye(party_at);
    for (entity, monster, at, walker, mut target, movement) in &mut walkers {
        let here = at.translation();
        let going = match movement {
            Some(mut movement) => {
                movement.moved += walker.speed * time.delta_secs();
                let continues = battle.is_some()
                    && monster.can_fight()
                    && movement.moved < movement.budget
                    && needs_approach(monster, here, Some(walker), party_eye, catalog);
                if !continues {
                    commands.entity(entity).remove::<CombatMove>();
                }
                continues
            }
            None => false,
        };
        let wanted = if going { AgentTarget3d::Entity(party) } else { AgentTarget3d::None };
        if !matches!((&*target, &wanted), (AgentTarget3d::None, AgentTarget3d::None))
            && !matches!((&*target, &wanted), (AgentTarget3d::Entity(a), AgentTarget3d::Entity(b)) if a == b)
        {
            *target = wanted;
        }
    }
}
