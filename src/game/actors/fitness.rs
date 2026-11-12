//! What a body is fit for, and what follows from its health, stamina and
//! conditions (`docs/rules/actors.md`, "Fitness").
//!
//! Nothing asks a body whether it can act. Whoever changes a body changes
//! its numbers and says nothing more; the small systems here see the change
//! and settle what follows from it: spent stamina leaves it out cold, health
//! run out fells it, its figures are written anew when what they are made
//! from has changed, and the markers below come and go. Rules filter on the markers, and react to their coming
//! and going.

use bevy::prelude::*;

use super::figures;
use crate::game::characters::{COMES_ROUND_ABOVE, Condition, Conditions, Dead, Fell, Vitals};

/// In its right mind: able to look, keep watch or identify (retail
/// `highest_condition < INSANE`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Lucid;

/// Able to guard a fellow (retail bodyguards, `highest_condition < BLIND`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CanGuard;

/// Able to act outside a combat round, such as to search, trade or rest
/// (retail `highest_condition < TURNCOAT`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CanAct;

/// Able to take or be the target of a combat action (retail
/// `highest_condition < ASLEEP`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CanFight;

/// Alive and not taken away (retail `highest_condition < DEAD`): time
/// passes for it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Present;

/// The schedule that settles what follows from a body's numbers and what
/// it is made from, in order. [`settle`] runs it before the frame's rules
/// read bodies, and on the fixed step once a fight's moment has been acted
/// out.
#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Settle;

/// Within [`Settle`]: every body's state is settled and its figures
/// written. What follows from the figures runs after it.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Written;

/// Run [`Settle`] now.
pub fn settle(world: &mut World) {
    world.run_schedule(Settle);
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Settle,
        (
            collapse,
            come_round,
            notice_the_fallen,
            notice_the_raised,
            crate::game::items::weigh,
            super::modifiers::gear_changed,
            figures::follow_calling,
            figures::attributes,
            figures::skills,
            figures::maxima,
            (figures::resistances, figures::armour, figures::attacks, figures::mending),
            (judge, super::time::tend, super::time::let_go),
        )
            .chain()
            .in_set(Written),
    )
    .add_systems(Update, settle.in_set(crate::game::GameplaySet::Bodies))
    .add_systems(FixedUpdate, settle.after(crate::game::GameplaySet::Combat));
}

/// Spent stamina leaves a body out cold (retail `FatigueCharacter`,
/// `FatigueMonster`).
fn collapse(mut bodies: Query<(&Vitals, &mut Conditions), Changed<Vitals>>) {
    for (vitals, mut conditions) in &mut bodies {
        let spent = vitals.stamina.max > 0 && vitals.stamina.current == 0;
        if spent && !vitals.is_dead() && !conditions.exhausted() {
            conditions.exhaust();
        }
    }
}

/// The exhausted come round once stamina is above 9.
fn come_round(mut bodies: Query<(&Vitals, &mut Conditions), Changed<Vitals>>) {
    for (vitals, mut conditions) in &mut bodies {
        if conditions.exhausted() && vitals.stamina.current > COMES_ROUND_ABOVE {
            conditions.lift(Condition::Unconscious);
        }
    }
}

/// A body whose health has run out has fallen.
fn notice_the_fallen(
    bodies: Query<(Entity, &Vitals), (Changed<Vitals>, Without<Dead>)>,
    mut commands: Commands,
) {
    for (body, vitals) in &bodies {
        if vitals.is_dead() {
            commands.trigger(Fell { body });
        }
    }
}

/// One of the dead with health again is dead no longer.
fn notice_the_raised(
    bodies: Query<(Entity, &Vitals), (Changed<Vitals>, With<Dead>)>,
    mut commands: Commands,
) {
    for (body, vitals) in &bodies {
        if !vitals.is_dead() {
            commands.entity(body).remove::<Dead>();
        }
    }
}

/// What a body is fit for follows the worst that ails it, and its life.
#[allow(clippy::type_complexity)]
fn judge(
    bodies: Query<
        (
            Entity,
            &Vitals,
            &Conditions,
            (Has<Lucid>, Has<CanGuard>, Has<CanAct>, Has<CanFight>, Has<Present>),
        ),
        Or<(Changed<Vitals>, Changed<Conditions>)>,
    >,
    mut commands: Commands,
) {
    for (body, vitals, conditions, (lucid, guards, acts, fights, present)) in &bodies {
        let fit = |below| !vitals.is_dead() && conditions.below(below);
        let mut body = commands.entity(body);
        mark::<Lucid>(&mut body, lucid, fit(Condition::Insane));
        mark::<CanGuard>(&mut body, guards, fit(Condition::Blind));
        mark::<CanAct>(&mut body, acts, fit(Condition::Turncoat));
        mark::<CanFight>(&mut body, fights, fit(Condition::Asleep));
        mark::<Present>(&mut body, present, fit(Condition::Missing));
    }
}

/// The marker is on the body exactly while it should be.
pub(super) fn mark<M: Component + Default>(body: &mut EntityCommands, has: bool, should: bool) {
    match (has, should) {
        (false, true) => {
            body.insert(M::default());
        }
        (true, false) => {
            body.remove::<M>();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Retail `FatigueCharacter`: spent stamina leaves a body out cold until
    /// it is above 9; out cold or dead, it cannot fight.
    #[test]
    fn a_body_is_fit_by_its_stamina_and_health() {
        let mut app = App::new();
        app.add_plugins(plugin);
        let body = app.world_mut().spawn(Vitals::full(18, 105)).id();
        let mut settled = |change: fn(&mut Vitals)| {
            change(&mut app.world_mut().get_mut::<Vitals>(body).unwrap());
            app.update();
            let world = app.world();
            (
                world.get::<Conditions>(body).unwrap().exhausted(),
                world.get::<CanFight>(body).is_some(),
            )
        };
        assert_eq!(settled(|_| ()), (false, true));
        assert_eq!(
            settled(|vitals| {
                vitals.tire(200);
            }),
            (true, false)
        );
        assert_eq!(
            settled(|vitals| {
                vitals.rest(9);
            }),
            (true, false),
            "not above 9 yet"
        );
        assert_eq!(
            settled(|vitals| {
                vitals.rest(1);
            }),
            (false, true)
        );
        assert_eq!(
            settled(|vitals| {
                vitals.hurt(100, None);
            }),
            (false, false),
            "the dead do not fight"
        );
    }
}
