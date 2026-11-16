//! How a monster decides (`docs/rules/monsters.md`). Each decision is a
//! function of the monster's body, of the plain data of its kind (its
//! temper, its special attack, its spells) and of what it makes out
//! ([`sight::Sight`]); one that rolls takes the random-number generator; none
//! changes anything. [`deliberate`], the observer of [`Deliberates`], reads
//! the kind's components and asks the decisions in the order of section 4;
//! the observers of [`super::turn`], [`super::engage`] and
//! [`super::round_end`] make happen what they decide.
//!
//! - [`conduct`]: run, withdraw, hold back, or fight on (4)
//! - [`special`]: a breath or a summoning (4, 7)
//! - [`spell`]: which spell, at whom and how hard (5)
//! - [`guard`]: guarding a fellow, holding back warily (4)
//! - [`close`]: reaching, closing in, withdrawing when it can strike nobody,
//!   and how far a move goes (4, 6)
//! - [`ailments`]: what a monster's conditions do to its action (8)
//! - [`coming`]: groups coming into a fight and leaving it (3)
//! - [`noticing`]: whether a watcher makes out someone watched (2)

pub mod ailments;
pub mod close;
pub mod conduct;
pub mod guard;
pub mod noticing;
pub mod sight;
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

/// A deliberation: whose. Its facts are added in the order of section 4 by
/// the observers of this module; what it comes to is said as [`Decided`],
/// and it is gone.
#[derive(Component, Clone, Copy, Debug)]
pub struct Deliberating(pub Entity);

/// How far the party is from the monster deliberating, at its moment.
#[derive(Component, Clone, Copy, Debug)]
pub struct PartyGap(pub f32);

/// Whether the monster closes in rather than consider its special attack or
/// a spell: it prefers blows, has some, and none reaches the party.
#[derive(Component, Clone, Copy, Debug)]
pub struct ClosesIn(pub bool);

/// A monster deliberates at its moment, considering the fight's combatants,
/// with the party as far off as it is and its eyes making it out or not.
pub(super) fn deliberate(
    deliberates: On<Deliberates>,
    sight: Sight,
    fight: Single<Option<&Combatants>, With<Battle>>,
    mut commands: Commands,
) {
    let monster = deliberates.monster;
    let considered = fight.map_or(Vec::new(), |fight| fight.entities().to_vec());
    commands.spawn((
        PartyGap(sight.party_gap(monster)),
        spell::Considers { by: monster, bodies: considered },
        spell::SeesParty(sight.sees_party(monster)),
        Deliberating(monster),
    ));
}

/// One that does not fight retreats as its conduct says; one that fights
/// may close in.
fn conducted(
    conducted: On<Add<conduct::Conducts>>,
    mut commands: Commands,
    deliberations: Query<(&Deliberating, &PartyGap, &conduct::Conducts)>,
    monsters: Query<(&Temper, &crate::game::actors::arm::Furthest)>,
) {
    let deliberation = conducted.entity;
    let Ok((&Deliberating(monster), &PartyGap(gap), &conduct::Conducts(conduct))) =
        deliberations.get(deliberation)
    else {
        return;
    };
    if conduct != Conduct::Fight {
        let retreats = Decision::Retreat(Retreats { conduct, quiet: false });
        commands.trigger(Decided { actor: monster, decision: retreats, own_will: true });
        commands.entity(deliberation).despawn();
        return;
    }
    let closes =
        monsters.get(monster).is_ok_and(|(temper, &reach)| close::closes_in(reach, &temper.0, gap));
    commands.entity(deliberation).insert(ClosesIn(closes));
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(deliberate)
        .add_observer(conduct::conduct)
        .add_observer(conducted)
        .add_observer(guard::strike_or_guard)
        .add_plugins((special::plugin, spell::plugin))
        .add_systems(
            crate::game::actors::Settle,
            special::ready.in_set(crate::game::actors::Written),
        );
}
