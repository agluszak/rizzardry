//! What bears on an actor's numbers from outside it: the spells laid on it
//! and on its party, and what it wears. The actor carries the running
//! total, written as each comes or goes; the figures that read it are
//! rewritten when it changes.

use bevy::prelude::*;

use super::spells::{Bonus, LaidOn, Spells};
use crate::game::items::{Charges, Item, Worn};
use crate::game::party::roster::{PartyMember, PartyRoster};

/// What is laid on an actor, summed, with what is laid on its party. What
/// it wears is read through its relationships; their changing marks this
/// changed.
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

fn put_on(event: On<Add<Worn>>, worn: Query<&Worn>, mut wearers: Query<&mut Modifiers>) {
    if let Ok(mut total) = worn.get(event.entity).and_then(|worn| wearers.get_mut(worn.by)) {
        total.set_changed();
    }
}

fn taken_off(event: On<Remove<Worn>>, worn: Query<&Worn>, mut wearers: Query<&mut Modifiers>) {
    if let Ok(mut total) = worn.get(event.entity).and_then(|worn| wearers.get_mut(worn.by)) {
        total.set_changed();
    }
}

/// A worn thing that is itself no longer what it was (out of shots, grown
/// into a better one) bears differently on its wearer.
pub(super) fn gear_changed(
    gear: Query<&Worn, Or<(Changed<Item>, Changed<Charges>)>>,
    mut wearers: Query<&mut Modifiers>,
) {
    for worn in &gear {
        if let Ok(mut total) = wearers.get_mut(worn.by) {
            total.set_changed();
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(laid)
        .add_observer(lifted)
        .add_observer(joined)
        .add_observer(left)
        .add_observer(put_on)
        .add_observer(taken_off);
}
