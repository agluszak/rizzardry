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

use super::Battle;
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::data::GameData;
use crate::game::party::body::PartyBody;
use crate::game::world::navigation::Walker;
use crate::game::world::population::Monster;

/// Stop this far inside an attack's reach.
const REACH_MARGIN: f32 = 0.3;

/// A monster's move this round: how far it may go, and how far it has.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct CombatMove {
    pub budget: f32,
    pub moved: f32,
    pub last: Option<Vec3>,
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

/// Walk monsters on their move toward the party, counting the ground they
/// cover; the move ends when they arrive, spend it, fall, or the fight ends.
pub fn close_in(
    mut commands: Commands,
    battle: Option<Res<Battle>>,
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut walkers: Query<(
        Entity,
        &Monster,
        ActorPose,
        &Walker,
        &mut AgentTarget3d,
        Option<&mut CombatMove>,
    )>,
    data: Option<Res<GameData>>,
    species: Res<Assets<MonsterCatalog>>,
) {
    let catalog = data.and_then(|d| species.get(&d.species));
    let Ok((party, party_at)) = party.single() else { return };
    let party_eye = PartyBody::eye(party_at);
    for (entity, monster, at, walker, mut target, movement) in &mut walkers {
        let here = at.translation();
        let going = match movement {
            Some(mut movement) => {
                if let Some(last) = movement.last {
                    movement.moved += last.with_y(0.0).distance(here.with_y(0.0));
                }
                movement.last = Some(here);
                let continues = battle.is_some()
                    && super::monster_can_attack(monster)
                    && movement.moved < movement.budget
                    && catalog.is_some_and(|species| {
                        needs_approach(monster, here, Some(walker), party_eye, species)
                    });
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
