//! How a monster decides (`docs/rules/monsters.md`). Each decision is a
//! function of the monster's body, of the plain data of its kind (its
//! temper, its special attack, its spells) and of what it makes out
//! ([`sight::Sight`]); one that rolls takes the random-number generator; none
//! changes anything. [`deliberate`], the observer of [`Deliberates`], begins a
//! deliberation, whose steps decide in the order of section 4, each saying
//! which comes next by an event;
//! the observers of [`super::turn`], [`super::engage`] and
//! [`super::round_end`] make happen what they decide.
//!
//! - [`conduct`]: run, withdraw, hold back, or fight on (4)
//! - [`special`]: a breath or a summoning (4, 7)
//! - [`slot`]: its spell slots, as entities (5)
//! - [`aim`]: whom a slot could be cast at, and whether it is worth it (5)
//! - [`spell`]: whether it casts, which spell and how hard (5)
//! - [`guard`]: guarding a fellow, holding back warily (4)
//! - [`close`]: reaching, closing in, withdrawing when it can strike nobody,
//!   and how far a move goes (4, 6)
//! - [`ailments`]: what a monster's conditions do to its action (8)
//! - [`noticing`]: whether a watcher makes out someone watched (2)

pub mod ailments;
pub mod aim;
pub mod close;
pub mod conduct;
pub mod guard;
pub mod noticing;
pub mod sight;
pub mod slot;
pub mod special;
pub mod spell;

use bevy::prelude::*;

pub use self::ailments::Interrupt;
use self::conduct::Conduct;
use self::sight::Sight;
use super::turn::{Decided, Decision, Retreats};
use super::{Battle, Combatants};
use crate::game::world::population::Temper;

/// A monster with no orders decides what to do with its moment.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Deliberates {
    #[event_target]
    pub monster: Entity,
}

/// A deliberation, an entity that [`aim::Considers`] its monster, is begun:
/// its conduct and whether it closes in are worked out.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Begun {
    #[event_target]
    pub deliberation: Entity,
}

/// A deliberation's conduct is known: it retreats, or goes on to weigh its
/// special attack.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Conducted {
    #[event_target]
    pub deliberation: Entity,
}

/// How far the party is from the monster deliberating, at its moment.
#[derive(Component, Clone, Copy, Debug)]
pub struct PartyGap(pub f32);

/// Whether the monster closes in rather than consider its special attack or
/// a spell: it prefers blows, has some, and none reaches the party.
#[derive(Component, Clone, Copy, Debug)]
pub struct ClosesIn(pub bool);

/// A monster deliberates at its moment, considering the fight's combatants,
/// with the party as far off as it is and its eyes making it out or not.
/// It views each living combatant, and the deliberation is begun; once its
/// conduct is known, it goes on. Each step decides the next; what it comes
/// to is said as [`Decided`], and it is gone.
pub(super) fn deliberate(
    deliberates: On<Deliberates>,
    sight: Sight,
    fight: Single<Option<&Combatants>, With<Battle>>,
    mut commands: Commands,
) {
    let monster = deliberates.monster;
    let considered = fight.map_or(&[][..], |fight| fight.entities());
    let deliberation =
        commands.spawn((PartyGap(sight.party_gap(monster)), aim::Considers(monster))).id();
    let living = considered.iter().copied().filter(|&body| sight.alive(body));
    aim::view(&mut commands, deliberation, living);
    commands.trigger(Begun { deliberation });
    commands.trigger(Conducted { deliberation });
}

/// Whether a monster that fights closes in (section 4).
fn closing(
    begun: On<Begun>,
    mut commands: Commands,
    deliberations: Query<(&aim::Considers, &PartyGap)>,
    monsters: Query<(&Temper, &crate::game::actors::arm::Furthest)>,
) -> Result {
    let deliberation = begun.deliberation;
    let (&aim::Considers(monster), &PartyGap(gap)) = deliberations.get(deliberation)?;
    let (temper, &reach) = monsters.get(monster)?;
    commands.entity(deliberation).insert(ClosesIn(close::closes_in(reach, &temper.0, gap)));
    Ok(())
}

/// One that does not fight retreats as its conduct says, and its
/// deliberation is over; one that fights weighs its special attack.
fn conducted(
    conducted: On<Conducted>,
    mut commands: Commands,
    deliberations: Query<(&aim::Considers, &conduct::Conducts)>,
) -> Result {
    let deliberation = conducted.deliberation;
    let (&aim::Considers(monster), &conduct::Conducts(conduct)) =
        deliberations.get(deliberation)?;
    if conduct == Conduct::Fight {
        commands.trigger(special::WeighsSpecial { deliberation });
        return Ok(());
    }
    let retreats = Decision::Retreat(Retreats { conduct, quiet: false });
    commands.trigger(Decided { actor: monster, decision: retreats, own_will: true });
    commands.entity(deliberation).despawn();
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(deliberate)
        .add_observer(conduct::conduct)
        .add_observer(closing)
        .add_observer(conducted)
        .add_observer(guard::strike_or_guard)
        .add_observer(slot::fill)
        .add_plugins((special::plugin, spell::plugin, aim::plugin))
        .add_systems(
            crate::game::actors::Settle,
            (special::ready, slot::castable).in_set(crate::game::actors::Written),
        );
}
