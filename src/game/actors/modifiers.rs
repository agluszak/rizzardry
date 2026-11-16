//! What the spells laid on an actor, and on its party, add to its numbers.
//! The actor carries the running total, written as each spell comes or
//! goes; the figures that read it are rewritten when it changes. What it
//! wears the figures read themselves ([`crate::game::items::Redressed`]).

use bevy::prelude::*;

use super::spells::{Bonus, LaidOn, Spells};
use crate::game::party::roster::{PartyMember, PartyRoster};

/// What is laid on an actor, summed, with what is laid on its party.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Modifiers {
    pub laid: Bonus,
}

/// The spells on bearers, to sum.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct Sums<'w, 's> {
    bearers: Query<'w, 's, &'static Spells>,
    spells: Query<'w, 's, &'static Bonus>,
    members: Query<'w, 's, &'static PartyMember>,
    rosters: Query<'w, 's, &'static PartyRoster>,
    totals: Query<'w, 's, &'static mut Modifiers>,
}

impl Sums<'_, '_> {
    /// What the spells on a bearer add together, but for one that is going.
    fn on(&self, bearer: Entity, going: Option<Entity>) -> Bonus {
        let laid = self.bearers.get(bearer).into_iter().flat_map(|spells| spells.iter());
        laid.filter(|&spell| Some(spell) != going)
            .filter_map(|spell| self.spells.get(spell).ok())
            .fold(Bonus::default(), |mut sum, bonus| {
                sum += *bonus;
                sum
            })
    }

    /// Write an actor's total anew: the spells on it and on its party.
    fn write(&mut self, actor: Entity, going: Option<Entity>) {
        let mut laid = self.on(actor, going);
        if let Ok(member) = self.members.get(actor) {
            laid += self.on(member.0, going);
        }
        if let Ok(mut total) = self.totals.get_mut(actor) {
            total.laid = laid;
        }
    }

    /// A spell on `bearer` has come or is going: the bearer's total, or
    /// every member's for a party.
    fn changed(&mut self, bearer: Entity, going: Option<Entity>) {
        let members: Vec<Entity> =
            self.rosters.get(bearer).map_or(vec![bearer], |roster| roster.iter().collect());
        for member in members {
            self.write(member, going);
        }
    }
}

fn laid(event: On<Add<LaidOn>>, on: Query<&LaidOn>, mut sums: Sums) {
    if let Ok(&LaidOn(bearer)) = on.get(event.entity) {
        sums.changed(bearer, None);
    }
}

fn lifted(event: On<Remove<LaidOn>>, on: Query<&LaidOn>, mut sums: Sums) {
    if let Ok(&LaidOn(bearer)) = on.get(event.entity) {
        sums.changed(bearer, Some(event.entity));
    }
}

/// One who joins a party is under its spells.
fn joined(event: On<Add<PartyMember>>, mut sums: Sums) {
    sums.write(event.entity, None);
}

/// One who leaves a party is under its spells no longer.
fn left(event: On<Remove<PartyMember>>, mut sums: Sums) {
    let own = sums.on(event.entity, None);
    if let Ok(mut total) = sums.totals.get_mut(event.entity) {
        total.laid = own;
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(laid).add_observer(lifted).add_observer(joined).add_observer(left);
}
