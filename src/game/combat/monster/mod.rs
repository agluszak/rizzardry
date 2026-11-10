//! How a monster decides (`docs/rules/monsters.md`). Each decision is a
//! function of the monster itself (what every actor is, and what its kind
//! and the fight make of it: [`sight::Mind`]) and of what it makes out
//! ([`sight::Sight`]); one that rolls takes the random-number generator; none
//! changes anything. [`deliberate`] asks them in the order of section 4, and
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
pub mod coming;
pub mod conduct;
pub mod guard;
pub mod noticing;
pub mod sight;
pub mod special;
pub mod spell;

use bevy::prelude::Entity;

pub use self::ailments::Interrupt;
use self::conduct::Conduct;
use self::guard::Watching;
use self::sight::{Fray, Sight};
use self::special::Made;
use self::spell::Cast;
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::rng::GameRng;

/// What a monster decided.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deliberated {
    /// Run, withdraw or hold back.
    Conducts(Conduct),
    Breathes(Made),
    Casts(Cast),
    /// It can strike nobody.
    Withdraws,
    Guards(Entity),
    HoldsBack,
    Strikes,
}

/// A decision, and whether the monster is running for its life from now on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thought {
    pub deliberated: Deliberated,
    pub fleeing: bool,
}

/// A monster decides what to do with its moment, in the order of section 4:
/// its conduct, which takes the moment when it is not to fight; its special
/// attack, a breath aimed as a cone spell is; a spell; withdrawing when it
/// can strike nobody; guarding a fellow or holding back; and otherwise
/// striking, which is closing in when nothing reaches.
pub fn deliberate(
    rng: &mut GameRng,
    sight: &Sight,
    monster: Entity,
    fray: Fray,
    content: &Catalogs,
) -> Option<Thought> {
    let (own, mind) = sight.monster(monster)?;
    let gap = sight.party_gap(monster);
    let conducted = conduct::conduct(rng, &own, &mind, gap);
    let thought = |deliberated| Some(Thought { deliberated, fleeing: conducted.fleeing });
    if conducted.conduct != Conduct::Fight {
        return thought(Deliberated::Conducts(conducted.conduct));
    }
    let breath = match content.spells.get(SpellId::SPECIAL_ATTACK_CONE) {
        Some(cone) if !special::summons(&mind) => spell::aims(sight, &own, cone, content, fray),
        _ => Vec::new(),
    };
    if let Some(made) = special::makes(rng, &own, &mind, gap, fray.fighting, &breath) {
        return thought(Deliberated::Breathes(made));
    }
    if let Some(cast) = spell::casts(rng, sight, &own, &mind, gap, content, fray) {
        return thought(Deliberated::Casts(cast));
    }
    if close::cannot_strike(&own, gap) {
        return thought(Deliberated::Withdraws);
    }
    thought(match guard::guards(rng, sight, &own, &mind, gap, fray.bodies) {
        Some(Watching::Guards(ward)) => Deliberated::Guards(ward),
        Some(Watching::HoldsBack) => Deliberated::HoldsBack,
        None => Deliberated::Strikes,
    })
}
