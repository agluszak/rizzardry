//! What an actor's ailments do to the action it was about to take
//! (`docs/rules/monsters.md` section 8; retail `GetConditionInterrupt`,
//! `Combat.cpp:2940-3051`).

use bevy::prelude::Entity;

use super::sight::Sight;
use crate::content::characters::Attribute;
use crate::game::actors::{Arm, reaches};
use crate::game::characters::Condition;
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::party::formation::Formation;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// What an actor's conditions did to its action as it began.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupt {
    /// A web held it.
    Webbed,
    /// Fear froze it.
    Afraid,
    Blinded,
    Nauseated,
    /// Insanity took it, in one of three fits.
    Insane {
        fit: u8,
    },
    /// It broke out of its web.
    Freed,
    /// Insanity turned it on its friends; it attacks them.
    Berserk,
    /// Turncoat holds it; its action against its new foes stands.
    Turncoat,
    /// Fear sent it running.
    Panics,
    /// A lure holds it: it walks toward the lure.
    Lured,
}

impl Interrupt {
    /// Retail's interrupt number, which numbers its notice.
    pub fn number(self) -> u8 {
        match self {
            Interrupt::Webbed => 0,
            Interrupt::Afraid => 1,
            Interrupt::Blinded => 2,
            Interrupt::Nauseated => 3,
            Interrupt::Insane { fit } => 4 + fit,
            Interrupt::Freed => 7,
            Interrupt::Berserk => 8,
            Interrupt::Turncoat => 9,
            Interrupt::Lured => 11,
            Interrupt::Panics => 12,
        }
    }
}

/// What an actor is about to do, as its ailments weigh it.
#[derive(Clone, Copy, Debug)]
pub struct Plans {
    /// Fear may send it running: it decides for itself.
    pub runs: bool,
    /// It is running for its life already.
    pub running: bool,
    /// Insanity has already turned it on its friends this round.
    pub turned_on_friends: bool,
    /// What it was about to do is aimed at its foes.
    pub against_foes: bool,
    /// A lure holds it.
    pub lured: bool,
}

/// Whether one of an actor's arms reaches a friend among `combatants`: a
/// living body on its side.
pub fn friend_in_reach(
    sight: &Sight,
    own: &BodyReadOnlyItem,
    combatants: &[Entity],
    formation: &Formation,
) -> bool {
    let actor = own.entity;
    let at = sight.places.of(actor);
    combatants
        .iter()
        .copied()
        .filter(|&other| other != actor && sight.toward(own, other) == Some(Stance::Friendly))
        .any(|other| {
            let there = sight.places.of(other);
            let reached = |arm: &Arm| reaches(formation, (actor, &at), (other, &there), arm.reach);
            own.attacks.arms.iter().any(reached)
        })
}

/// What an actor's ailments do to the action it was about to take, the first
/// that applies winning (retail `GetConditionInterrupt`):
///
/// - Nauseated: the action is lost a quarter of the time.
/// - Webbed: it breaks free with a chance of a third of its Strength in
///   percent, and is held otherwise; either way the action is lost.
/// - Blind, and not one who fights blind: lost a fifth of the time.
/// - Insane: lost four times in five; with a friend in reach, and not having
///   turned on its friends this round already, two times in five of those it
///   turns on them instead; otherwise one of three fits.
/// - Afraid: frozen a quarter of the time, unless it is running already; one
///   that decides for itself then runs for its life two times in five.
/// - A lure holds it: it walks toward the lure instead.
/// - Turncoat: an action against its new foes stands, and is told.
pub fn interrupt(
    rng: &mut GameRng,
    own: &BodyReadOnlyItem,
    plans: Plans,
    friend_in_reach: bool,
) -> Option<Interrupt> {
    let has = |condition| own.conditions.has(condition);
    if has(Condition::Nauseated) && rng.below(100) < 25 {
        return Some(Interrupt::Nauseated);
    }
    if has(Condition::Webbed) {
        let freed = rng.below(100) < own.attributes[Attribute::Strength] / 3;
        return Some(if freed { Interrupt::Freed } else { Interrupt::Webbed });
    }
    if has(Condition::Blind) && own.attacks.blind_fighting.is_none() && rng.below(100) < 20 {
        return Some(Interrupt::Blinded);
    }
    if has(Condition::Insane) && rng.below(100) < 80 {
        if friend_in_reach && !plans.turned_on_friends && rng.below(100) < 40 {
            return Some(Interrupt::Berserk);
        }
        return Some(Interrupt::Insane { fit: rng.below(3) as u8 });
    }
    if has(Condition::Afraid) && !plans.running && rng.below(100) < 25 {
        return Some(Interrupt::Afraid);
    }
    if plans.runs && has(Condition::Afraid) && rng.below(100) < 40 {
        return Some(Interrupt::Panics);
    }
    if plans.lured {
        return Some(Interrupt::Lured);
    }
    (has(Condition::Turncoat) && plans.against_foes).then_some(Interrupt::Turncoat)
}
