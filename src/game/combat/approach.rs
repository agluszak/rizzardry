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
        budget += kind.combat_move_range * METRES_PER_MOVE_RANGE * f32::from(haste.power) * 0.1;
    } else if monster.conditions.iter().any(|&(c, _)| c == Condition::Slowed) {
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
    content: crate::game::data::Content,
    targets: Query<(&Monster, ActorPose, Option<&Walker>)>,
) {
    let Some(catalog) = content.species() else { return };
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
                    let reach = catalog
                        .kind(monster.species)
                        .and_then(|kind| kind.attacks.iter().map(|a| a.reach).max())
                        .unwrap_or(Reach::Touch)
                        .metres();
                    !target.is_dead()
                        && here.distance(there) - monster.body.radius - target.body.radius
                            > reach - REACH_MARGIN
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
pub fn spend_moves(time: Res<Time<Fixed>>, mut walkers: Query<(&Walker, &AgentTarget3d, &mut CombatMove)>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct Ended(Option<(Duration, f32)>);

    /// A 1 m move at 2 m/s, begun after some frames of `frame` each: the
    /// fixed time from its start to its end, and the walking it counted.
    fn run_move(frame: Duration) -> (Duration, f32) {
        use crate::content::species::Species;
        use crate::game::data::GameData;
        use crate::game::world::population::ScaledBody;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(frame))
            .init_resource::<Assets<MonsterCatalog>>()
            .init_resource::<Ended>()
            .add_systems(FixedPreUpdate, steer_moves)
            .add_systems(FixedPostUpdate, spend_moves)
            .add_observer(stop_walking)
            .add_observer(
                |remove: On<Remove<CombatMove>>,
                 moves: Query<&CombatMove>,
                 time: Res<Time<Fixed>>,
                 mut ended: ResMut<Ended>| {
                    let moved = moves.get(remove.entity).unwrap().moved;
                    ended.0 = Some((time.elapsed(), moved));
                },
            );
        let species = app
            .world_mut()
            .resource_mut::<Assets<MonsterCatalog>>()
            .add(MonsterCatalog(vec![Species::default()]));
        app.insert_resource(GameData {
            items: default(),
            loot: default(),
            species,
            spells: default(),
        });
        app.world_mut().spawn((PartyBody::default(), Position::default()));
        let monster = app
            .world_mut()
            .spawn((
                Monster {
                    enchantments: default(),
                    location: 0,
                    species: 0,
                    hp: 10,
                    max_hp: 10,
                    stamina: 0,
                    max_stamina: 0,
                    conditions: vec![],
                    spell_points: 0,
                    body: ScaledBody { scale: 1.0, radius: 1.0, eye_height: 1.0, speed: 2.0 },
                },
                Walker { speed: 2.0, float_height: 1.0 },
                Transform::from_xyz(0.0, 0.0, -10.0),
                AgentTarget3d::None,
            ))
            .id();
        // Time passes before the move begins, as it does while a fight
        // waits for the monster's phase.
        for _ in 0..5 {
            app.update();
        }
        let start = app.world().resource::<Time<Fixed>>().elapsed();
        app.world_mut().entity_mut(monster).insert(CombatMove {
            budget: 1.0,
            moved: 0.0,
            away: Some(Vec3::new(0.0, 0.0, -20.0)),
        });
        while app.world().resource::<Ended>().0.is_none() {
            app.update();
            // Headed off from the first step it walks.
            if app.world().get::<CombatMove>(monster).is_some()
                && app.world().resource::<Time<Fixed>>().elapsed() > start
            {
                assert_eq!(
                    *app.world().get::<AgentTarget3d>(monster).unwrap(),
                    AgentTarget3d::Point(Vec3::new(0.0, 0.0, -20.0))
                );
            }
        }
        assert_eq!(*app.world().get::<AgentTarget3d>(monster).unwrap(), AgentTarget3d::None);
        let (end, moved) = app.world().resource::<Ended>().0.unwrap();
        (end - start, moved)
    }

    /// A move is counted on fixed steps alone: fast frames that run a step
    /// now and then, and slow ones that run several, end it at the same
    /// step, having counted the same walking, none of it from before the
    /// move began.
    #[test]
    fn a_move_ends_at_the_same_fixed_step_whatever_the_frame_rate() {
        let fast = run_move(Duration::from_micros(4_167));
        let slow = run_move(Duration::from_millis(143));
        assert_eq!(fast, slow);
        // 2 m/s over 1/64 s steps spends 1 m in 32 steps; the 33rd ends it.
        let step = Time::<Fixed>::default().timestep();
        assert_eq!(fast, (step * 33, 1.0));
    }
}
