//! What an actor's ailments do to the action it was about to take
//! (`docs/rules/monsters.md` section 8; retail `GetConditionInterrupt`,
//! `Combat.cpp:2940-3051`).

use crate::game::rng::GameRng;

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
            Interrupt::Panics => 12,
        }
    }
}

/// What the ailments of an actor weigh.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ailing {
    pub nauseated: bool,
    pub webbed: bool,
    pub blind: bool,
    pub insane: bool,
    pub afraid: bool,
    pub turncoat: bool,
    pub strength: u32,
    /// Blindness does not stop it (a Monk monster).
    pub fights_blind: bool,
    /// Fear may send it running: a monster deciding for itself.
    pub runs: bool,
    /// It is running for its life already.
    pub running: bool,
    /// One of its arms reaches a friend.
    pub friend_in_reach: bool,
    /// Insanity has already turned it on its friends this round.
    pub turned_on_friends: bool,
    /// What it was about to do is aimed at its foes.
    pub against_foes: bool,
}

/// What its ailments do to its action, the first that applies:
///
/// - Nauseated: 25 percent it loses the action;
/// - Webbed: it breaks free with chance `Strength / 3` percent, and the action
///   is lost anyway;
/// - Blind: 20 percent, unless it fights blind, it loses the action;
/// - Insane: 80 percent; if an arm reaches a friend and it has not yet turned
///   this round, 40 percent it turns on its friends, else it loses the action
///   in one of three fits;
/// - Afraid: 25 percent, unless it is running, it loses the action; then, for
///   one that decides for itself, 40 percent it runs for its life;
/// - Turncoat: nothing, an action against its new foes stands.
pub fn interrupt(rng: &mut GameRng, facts: &Ailing) -> Option<Interrupt> {
    if facts.nauseated && rng.below(100) < 25 {
        return Some(Interrupt::Nauseated);
    }
    if facts.webbed {
        let freed = rng.below(100) < facts.strength / 3;
        return Some(if freed { Interrupt::Freed } else { Interrupt::Webbed });
    }
    if facts.blind && rng.below(100) < 20 && !facts.fights_blind {
        return Some(Interrupt::Blinded);
    }
    if facts.insane && rng.below(100) < 80 {
        if facts.friend_in_reach && !facts.turned_on_friends && rng.below(100) < 40 {
            return Some(Interrupt::Berserk);
        }
        return Some(Interrupt::Insane { fit: rng.below(3) as u8 });
    }
    if facts.afraid && rng.below(100) < 25 && !facts.running {
        return Some(Interrupt::Afraid);
    }
    if facts.runs && facts.afraid && rng.below(100) < 40 {
        return Some(Interrupt::Panics);
    }
    (facts.turncoat && facts.against_foes).then_some(Interrupt::Turncoat)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWS: usize = 1000;

    /// Every interrupt over many draws, with how many times each came.
    fn tally(facts: &Ailing) -> Vec<(Option<Interrupt>, usize)> {
        let mut rng = GameRng::seeded(17);
        let mut counts: Vec<(Option<Interrupt>, usize)> = Vec::new();
        for _ in 0..DRAWS {
            let got = interrupt(&mut rng, facts);
            match counts.iter_mut().find(|(seen, _)| *seen == got) {
                Some((_, n)) => *n += 1,
                None => counts.push((got, 1)),
            }
        }
        counts
    }

    /// An actor with nothing ailing it.
    fn calm() -> Ailing {
        Ailing::default()
    }

    #[test]
    fn turncoat_lets_an_action_against_its_new_foes_stand() {
        let turned = Ailing { turncoat: true, against_foes: true, ..calm() };
        assert_eq!(tally(&turned), [(Some(Interrupt::Turncoat), DRAWS)]);
        let elsewhere = Ailing { against_foes: false, ..turned };
        assert_eq!(tally(&elsewhere), [(None, DRAWS)]);
    }
}
