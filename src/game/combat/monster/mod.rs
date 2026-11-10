//! How a monster decides (`docs/rules/monsters.md`), each decision a function
//! of the facts it is about and nothing else: no query, no world, no catalog.
//! A decision that rolls takes the random-number generator; none changes
//! anything. [`facts`] reads what a monster knows, [`deliberate`] asks the
//! decisions in the order of section 4, and the observers of
//! [`super::turn`], [`super::engage`] and [`super::round_end`] make happen what
//! they decide.
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
pub mod facts;
pub mod guard;
pub mod noticing;
pub mod special;
pub mod spell;

use bevy::prelude::Entity;

pub use self::ailments::Interrupt;
use self::close::Striking;
use self::conduct::{Conduct, Conducting};
use self::guard::{Guarding, Watching};
use self::special::{Made, Making};
use self::spell::{Cast, Casting};
use crate::game::rng::GameRng;

/// Everything a monster at its moment in a fight decides from.
pub struct Deliberation<'a> {
    pub conducting: Conducting,
    pub striking: Striking<'a>,
    pub special: Making<'a>,
    pub casting: Casting<'a>,
    pub guarding: Guarding<'a>,
}

/// What a monster decides to do with its moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deliberated {
    /// Run, withdraw or hold back, by its conduct.
    Conducts(Conduct),
    /// Make its special attack.
    Breathes(Made),
    Casts(Cast),
    /// Withdraw, as it can strike nobody.
    Withdraws,
    Guards(Entity),
    /// Hold back warily.
    HoldsBack,
    /// Strike, or close in to.
    Strikes,
}

/// A decision, and whether the monster is running from now on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thought {
    pub deliberated: Deliberated,
    pub fleeing: bool,
}

/// What a monster does with its moment, the first that applies (retail
/// `UpdateMonsterAI`, `BuildMonsterActionQueue`, `MonsterAI.cpp:817-945`):
/// its conduct, if that is not to fight; its special attack; a spell; withdraw
/// if it can strike nobody; guard a fellow or hold back; strike.
pub fn deliberate(rng: &mut GameRng, facts: &Deliberation) -> Thought {
    let conducted = conduct::conduct(rng, &facts.conducting);
    let thought = |deliberated| Thought { deliberated, fleeing: conducted.fleeing };
    if conducted.conduct != Conduct::Fight {
        return thought(Deliberated::Conducts(conducted.conduct));
    }
    if let Some(made) = special::makes(rng, &facts.special) {
        return thought(Deliberated::Breathes(made));
    }
    if let Some(cast) = spell::casts(rng, &facts.casting) {
        return thought(Deliberated::Casts(cast));
    }
    if close::cannot_strike(&facts.striking) {
        return thought(Deliberated::Withdraws);
    }
    thought(match guard::guards(rng, &facts.guarding) {
        Some(Watching::Guards(ward)) => Deliberated::Guards(ward),
        Some(Watching::HoldsBack) => Deliberated::HoldsBack,
        None => Deliberated::Strikes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::species::Reach;
    use crate::game::world::hostility::Stance;

    const CLAWS: [Reach; 1] = [Reach::Touch];

    /// A steady hostile monster with claws, the party `gap` off, that has no
    /// special attack, no spell, no ward and is not wary.
    fn plain(gap: f32) -> Deliberation<'static> {
        let striking =
            Striking { arms: &CLAWS, party_gap: gap, prefers_range: false, blind: false };
        Deliberation {
            conducting: Conducting {
                health: 100,
                flee_chance: 0,
                craven: false,
                camouflaged: false,
                npc: false,
                bystander: false,
                stance: Stance::Hostile,
                fleeing: false,
                party_gap: gap,
            },
            striking,
            special: Making {
                has: false,
                summons: false,
                chance: 0,
                cooling: false,
                stamina: 10,
                stamina_most: 10,
                silenced: false,
                honours_silence: false,
                striking,
                fighting: true,
                stance: Stance::Hostile,
                aims: &[],
            },
            casting: Casting {
                chance: 0,
                striking,
                silenced: false,
                silent_alchemy: false,
                budget: None,
                slots: &[],
            },
            guarding: Guarding {
                striking,
                bodyguard: false,
                able: true,
                wary: false,
                intelligence: 10,
                level: 1,
                fellows: &[],
            },
        }
    }

    fn think(facts: &Deliberation) -> Thought {
        deliberate(&mut GameRng::seeded(1), facts)
    }

    #[test]
    fn a_steady_monster_with_nothing_else_to_do_strikes() {
        assert_eq!(
            think(&plain(1.0)),
            Thought { deliberated: Deliberated::Strikes, fleeing: false }
        );
    }

    #[test]
    fn conduct_comes_first_and_takes_the_moment_when_it_is_not_to_fight() {
        let mut facts = plain(1.0);
        facts.conducting.craven = true;
        let got = think(&facts);
        assert_eq!(got.deliberated, Deliberated::Conducts(Conduct::Flee));
        assert!(got.fleeing);
        // Even with a special attack ready.
        facts.special = Making { has: true, chance: 100, ..facts.special };
        facts.special.aims = &[crate::game::magic::casting::Aim::Ahead];
        assert_eq!(think(&facts).deliberated, Deliberated::Conducts(Conduct::Flee));
    }

    #[test]
    fn the_special_attack_comes_before_a_spell_and_a_spell_before_withdrawing_or_striking() {
        use crate::content::spells::{SpellDefinition, SpellId, Targeting};
        use crate::game::magic::casting::Aim;
        let blast = SpellDefinition {
            id: SpellId(1),
            targeting: Targeting::Enemy,
            monsters_cast: true,
            ..Default::default()
        };
        let slots = [Some(spell::Slot { spell: &blast, modelled: true, aims: vec![Aim::Ahead] })];
        let mut facts = plain(1.0);
        facts.casting = Casting { chance: 100, budget: Some(30), slots: &slots, ..facts.casting };
        let cast = think(&facts).deliberated;
        assert!(matches!(cast, Deliberated::Casts(_)), "{cast:?}");
        facts.special = Making { has: true, chance: 100, aims: &[Aim::Ahead], ..facts.special };
        let breathes = think(&facts).deliberated;
        assert!(matches!(breathes, Deliberated::Breathes(_)), "{breathes:?}");
    }

    #[test]
    fn one_that_prefers_blows_and_cannot_reach_closes_in_instead_of_casting_or_breathing() {
        use crate::game::magic::casting::Aim;
        let mut facts = plain(10.0);
        facts.special = Making { has: true, chance: 100, aims: &[Aim::Ahead], ..facts.special };
        // The claws reach 2 m: at 10 m the monster closes in, which is striking.
        assert_eq!(think(&facts).deliberated, Deliberated::Strikes);
    }

    #[test]
    fn a_monster_with_no_arm_withdraws() {
        let mut facts = plain(1.0);
        facts.striking.arms = &[];
        assert_eq!(think(&facts).deliberated, Deliberated::Withdraws);
    }

    #[test]
    fn guarding_and_holding_back_come_before_striking() {
        use self::guard::Fellow;
        let fellows = [Fellow {
            body: Entity::PLACEHOLDER,
            friendly: true,
            in_reach: true,
            level: 9,
            health: 10,
        }];
        let mut facts = plain(1.0);
        facts.guarding =
            Guarding { bodyguard: true, intelligence: 100, fellows: &fellows, ..facts.guarding };
        assert_eq!(think(&facts).deliberated, Deliberated::Guards(Entity::PLACEHOLDER));
        let mut facts = plain(1.0);
        facts.guarding = Guarding { wary: true, intelligence: 100, ..facts.guarding };
        assert_eq!(think(&facts).deliberated, Deliberated::HoldsBack);
    }
}
