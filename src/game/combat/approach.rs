//! Monsters closing in on the party during a fight. A monster in the fight
//! uses its existing world-owned walking body to head for the party
//! through the navmesh until one of its attacks reaches; the dead and those
//! in reach stand. Moves are steered and counted on the fixed step the
//! walking runs on, so how frames batch steps does not change where a move
//! ends. As in retail, moving is a monster's action within the
//! round: at its phase, with nothing in reach, it advances (or charges, when
//! hurt) up to `combat_move_range` × 20 metres, and acts again later in the
//! round (`Combat.cpp:2486-2551, 2855-2909`). The fight's next actor waits
//! for it to arrive or spend its move.

use crate::game::world::ActorPose;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;

use super::Combatant;
use crate::content::species::{MonsterCatalog, Reach, Species};
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
    pub goal: MoveGoal,
}

/// An accepted destination; navigation projects it into its current target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoveGoal {
    Party,
    Retreat(Vec3),
    Attack(Entity),
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
    let kind = &species[monster.species];
    let reach = kind.attacks.iter().map(|a| a.reach).max().unwrap_or(Reach::Touch);
    let feet = crate::game::world::navigation::feet(origin, Some(walker));
    crate::game::world::perception::party_distance(party_eye, monster, feet)
        > reach.metres() - REACH_MARGIN
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
    let kind = &species[monster.species];
    let charge = monster.charges();
    Some(MoveBudget { metres: move_metres(monster, kind, charge), charge })
}

/// How far a monster moves in one combat action (retail
/// `SetUpMonsterTurn`): its move range, half again when charging, raised
/// by Haste or halved when slowed.
pub(super) fn move_metres(monster: &Monster, kind: &Species, charge: bool) -> f32 {
    let mut budget = kind.combat_move_range * METRES_PER_MOVE_RANGE;
    if charge {
        budget *= 1.5;
    }
    if let Some(haste) = monster.enchantments.get(crate::content::spells::SpellId::HASTE) {
        budget += kind.combat_move_range * METRES_PER_MOVE_RANGE * haste.power as f32 * 0.1;
    } else if monster.conditions.has(Condition::Slowed) {
        budget *= 0.5;
    }
    budget
}

/// Where a monster running from the party heads: as far as its move
/// takes it, straight away from the party over the ground.
pub(super) fn away_from(feet: Vec3, party_eye: Vec3, metres: f32) -> Vec3 {
    let away = (feet - party_eye).with_y(0.0).normalize_or(Vec3::X);
    feet + away * metres
}

/// Metres per unit of a species' `combat_move_range`.
pub(super) const METRES_PER_MOVE_RANGE: f32 = 20.0;

/// Head monsters on their move, at the start of each fixed step before
/// navigation plans it; the move ends when it is spent, the monster
/// arrives, or it can no longer fight. The step's walking is counted after
/// physics moves it ([`spend_moves`]), so a move begun between steps is
/// charged nothing for time before it.
pub fn steer_moves(
    mut commands: Commands,
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut walkers: Query<(Entity, &Monster, ActorPose, &Walker, &mut AgentTarget3d, &CombatMove)>,
    content: If<Res<crate::game::data::Catalogs>>,
    targets: Query<(&Monster, ActorPose, Option<&Walker>)>,
) {
    let catalog = &content.species;
    let Ok((party, party_at)) = party.single() else { return };
    let party_eye = PartyBody::eye(party_at);
    for (entity, monster, at, walker, mut target, movement) in &mut walkers {
        let (needs_move, heading) = match movement.goal {
            MoveGoal::Retreat(point) => (true, AgentTarget3d::Point(point)),
            MoveGoal::Party => (
                needs_approach(monster, at.translation(), Some(walker), party_eye, catalog),
                AgentTarget3d::Entity(party),
            ),
            MoveGoal::Attack(entity) => {
                let needed = targets.get(entity).is_ok_and(|(target, target_at, target_walker)| {
                    let here = crate::game::world::navigation::feet(at.translation(), Some(walker));
                    let there = crate::game::world::navigation::feet(
                        target_at.translation(),
                        target_walker,
                    );
                    let reach = catalog[monster.species]
                        .attacks
                        .iter()
                        .map(|a| a.reach)
                        .max()
                        .unwrap_or(Reach::Touch)
                        .metres();
                    !target.is_dead() && monster.gap(here, target, there) > reach - REACH_MARGIN
                });
                (needed, AgentTarget3d::Entity(entity))
            }
        };
        let continues = monster.can_fight() && movement.moved < movement.budget && needs_move;
        let heading = if !continues {
            commands.entity(entity).remove::<CombatMove>();
            AgentTarget3d::None
        } else {
            heading
        };
        if *target != heading {
            *target = heading;
        }
    }
}

/// Charge each monster on its move the fixed step it has walked, at its
/// walking speed whether or not it got anywhere.
pub fn spend_moves(
    time: Res<Time<Fixed>>,
    mut walkers: Query<(&Walker, &AgentTarget3d, &mut CombatMove)>,
) {
    for (walker, target, mut movement) in &mut walkers {
        if !matches!(target, AgentTarget3d::None) {
            movement.moved += walker.speed * time.delta_secs();
        }
    }
}

/// Leaving the fight, as its end does, ends the monster's move.
pub(super) fn end_move(remove: On<Remove<Combatant>>, mut commands: Commands) {
    commands.entity(remove.entity).try_remove::<CombatMove>();
}

/// A monster whose move ends stops walking: without a target, neither its
/// navigation nor a doorway crossing it began moves it
/// ([`crate::game::world::navigation::walking`]).
pub(super) fn stop_walking(remove: On<Remove<CombatMove>>, mut targets: Query<&mut AgentTarget3d>) {
    if let Ok(mut target) = targets.get_mut(remove.entity)
        && !matches!(*target, AgentTarget3d::None)
    {
        *target = AgentTarget3d::None;
    }
}
