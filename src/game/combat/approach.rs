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

use super::Combatant;
use crate::content::species::{MonsterCatalog, Reach};
use crate::game::characters::Condition;
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
    let Some(kind) = species.kind(monster.species) else { return false };
    let reach = kind.attacks.iter().map(|a| a.reach).max().unwrap_or(Reach::Touch);
    let feet = crate::game::world::navigation::feet(origin, Some(walker));
    super::distance_to_party(party_eye, monster, feet) > reach.metres() - REACH_MARGIN
}

/// A monster's move this round, and whether it is a charge.
pub(super) struct MoveBudget {
    pub metres: f32,
    pub charge: bool,
}

/// How far a monster with nobody in reach may advance this round: up to
/// `combat_move_range` × 20 metres, half again when charging (hurt, at 95%
/// health or less), halved when slowed (retail `StepMonsterCombatAction`).
pub(super) fn move_budget(
    monster: &Monster,
    origin: Vec3,
    walker: Option<&Walker>,
    party_eye: Vec3,
    species: &MonsterCatalog,
) -> Option<MoveBudget> {
    if !needs_approach(monster, origin, walker, party_eye, species) {
        return None;
    }
    let kind = species.kind(monster.species)?;
    let charge = monster.hp * 100 <= monster.max_hp * 95;
    let mut budget = kind.combat_move_range * METRES_PER_MOVE_RANGE;
    if charge {
        budget *= 1.5;
    }
    if monster.conditions.iter().any(|&(c, _)| c == Condition::Slowed) {
        budget *= 0.5;
    }
    Some(MoveBudget { metres: budget, charge })
}

/// Metres per unit of a species' `combat_move_range`.
const METRES_PER_MOVE_RANGE: f32 = 20.0;

/// Walk monsters on their move toward the party; the move ends when they
/// arrive, spend it or fall.
pub fn close_in(
    mut commands: Commands,
    time: Res<Time>,
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut walkers: Query<(Entity, &Monster, ActorPose, &Walker, &mut AgentTarget3d, &mut CombatMove)>,
    content: crate::game::data::Content,
) {
    let Some(catalog) = content.species() else { return };
    let Ok((party, party_at)) = party.single() else { return };
    let party_eye = PartyBody::eye(party_at);
    for (entity, monster, at, walker, mut target, mut movement) in &mut walkers {
        movement.moved += walker.speed * time.delta_secs();
        let continues = monster.can_fight()
            && movement.moved < movement.budget
            && needs_approach(monster, at.translation(), Some(walker), party_eye, catalog);
        if !continues {
            commands.entity(entity).remove::<CombatMove>();
        } else if !matches!(*target, AgentTarget3d::Entity(e) if e == party) {
            *target = AgentTarget3d::Entity(party);
        }
    }
}

/// Leaving the fight, as its end does, ends the monster's move.
pub(super) fn end_move(remove: On<Remove<Combatant>>, mut commands: Commands) {
    commands.entity(remove.entity).try_remove::<CombatMove>();
}

/// A monster whose move ends stops walking.
pub(super) fn stop_walking(remove: On<Remove<CombatMove>>, mut targets: Query<&mut AgentTarget3d>) {
    if let Ok(mut target) = targets.get_mut(remove.entity)
        && !matches!(*target, AgentTarget3d::None)
    {
        *target = AgentTarget3d::None;
    }
}
