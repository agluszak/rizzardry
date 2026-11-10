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
use crate::content::species::Reach;
use crate::game::characters::Condition;
use crate::game::characters::Vitals;
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

/// Whether a living walker `gap` from its foe still needs to close to
/// attack range, its arms reaching `reach`. Combat's passive-round decision
/// and navigation use the same current actor state.
pub(super) fn needs_approach(
    reach: Reach,
    vitals: &Vitals,
    walker: Option<&Walker>,
    gap: f32,
) -> bool {
    let Some(walker) = walker else { return false };
    if vitals.is_dead() || walker.speed <= 0.0 {
        return false;
    }
    gap > reach.metres() - REACH_MARGIN
}

/// A body's move this round, and whether it is a charge.
pub(super) struct MoveBudget {
    pub metres: f32,
    pub charge: bool,
}

/// How far a body `gap` from the party with nobody in reach may advance
/// this round: its stride, half again when charging (hurt, at 95% health
/// or less), halved when slowed (retail `StepMonsterCombatAction`).
pub(super) fn move_budget(
    (reach, stride): (Reach, f32),
    vitals: &Vitals,
    walker: Option<&Walker>,
    gap: f32,
) -> Option<MoveBudget> {
    if !needs_approach(reach, vitals, walker, gap) {
        return None;
    }
    let charge = vitals.charges();
    Some(MoveBudget { metres: move_metres(vitals, stride, charge), charge })
}

/// How far a body moves in one combat action (retail `SetUpMonsterTurn`):
/// its stride, half again when charging, raised by Haste or halved when
/// slowed.
pub(super) fn move_metres(vitals: &Vitals, stride: f32, charge: bool) -> f32 {
    let mut budget = stride;
    if charge {
        budget *= 1.5;
    }
    if let Some(haste) = vitals.enchantments.get(crate::content::spells::SpellId::HASTE) {
        budget += stride * haste.power as f32 * 0.1;
    } else if vitals.conditions.has(Condition::Slowed) {
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
pub(crate) const METRES_PER_MOVE_RANGE: f32 = 20.0;

/// Head monsters on their move, at the start of each fixed step before
/// navigation plans it; the move ends when it is spent, the monster
/// arrives, or it can no longer fight. The step's walking is counted after
/// physics moves it ([`spend_moves`]), so a move begun between steps is
/// charged nothing for time before it.
pub fn steer_moves(
    mut commands: Commands,
    party: Query<(Entity, &Position), With<PartyBody>>,
    mut walkers: Query<(
        Entity,
        (&Monster, &Vitals, &crate::game::actors::Attacks),
        ActorPose,
        &Walker,
        &mut AgentTarget3d,
        &CombatMove,
    )>,
    targets: Query<(&Monster, &Vitals, ActorPose, Option<&Walker>)>,
) {
    let Ok((party, party_at)) = party.single() else { return };
    let party_eye = PartyBody::eye(party_at);
    for (entity, (monster, vitals, arms), at, walker, mut target, movement) in &mut walkers {
        let (needs_move, heading) = match movement.goal {
            MoveGoal::Retreat(point) => (true, AgentTarget3d::Point(point)),
            MoveGoal::Party => {
                let here = crate::game::world::navigation::feet(at.translation(), Some(walker));
                let gap = crate::game::world::perception::party_distance(party_eye, monster, here);
                (
                    needs_approach(arms.reach(), vitals, Some(walker), gap),
                    AgentTarget3d::Entity(party),
                )
            }
            MoveGoal::Attack(entity) => {
                let needed = targets.get(entity).is_ok_and(
                    |(target, target_vitals, target_at, target_walker)| {
                        let here =
                            crate::game::world::navigation::feet(at.translation(), Some(walker));
                        let there = crate::game::world::navigation::feet(
                            target_at.translation(),
                            target_walker,
                        );
                        !target_vitals.is_dead()
                            && monster.gap(here, target, there)
                                > arms.reach().metres() - REACH_MARGIN
                    },
                );
                (needed, AgentTarget3d::Entity(entity))
            }
        };
        let continues = vitals.can_fight() && movement.moved < movement.budget && needs_move;
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
