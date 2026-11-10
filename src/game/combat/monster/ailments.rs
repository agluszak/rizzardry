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

    fn count(facts: &Ailing, interrupt: Option<Interrupt>) -> usize {
        tally(facts).iter().find(|(seen, _)| *seen == interrupt).map_or(0, |&(_, n)| n)
    }

    /// Whether `count` is within 8 points of the expected share (%) of `DRAWS`.
    fn about(count: usize, percent: usize) -> bool {
        let want = DRAWS * percent / 100;
        count.abs_diff(want) <= DRAWS * 8 / 100
    }

    #[test]
    fn a_clear_head_is_not_interrupted() {
        assert_eq!(tally(&Ailing { runs: true, ..calm() }), [(None, DRAWS)]);
    }

    /// An actor with nothing ailing it.
    fn calm() -> Ailing {
        Ailing::default()
    }

    #[test]
    fn nausea_takes_the_action_a_quarter_of_the_time() {
        let sick = Ailing { nauseated: true, ..calm() };
        // 25 of 100 lose it; the rest carry on.
        assert!(about(count(&sick, Some(Interrupt::Nauseated)), 25));
        assert!(about(count(&sick, None), 75));
    }

    #[test]
    fn a_web_holds_it_or_it_breaks_free_by_a_third_of_its_strength_percent() {
        // Strength 60: 60 / 3 = 20 percent freed, the rest held. Both lose the action.
        let webbed = Ailing { webbed: true, strength: 60, ..calm() };
        assert!(about(count(&webbed, Some(Interrupt::Freed)), 20));
        assert!(about(count(&webbed, Some(Interrupt::Webbed)), 80));
        assert_eq!(count(&webbed, None), 0);
        // Strength 2: 2 / 3 = 0 whole: never freed. Strength 300: 100: always.
        let weak = Ailing { webbed: true, strength: 2, ..calm() };
        assert_eq!(count(&weak, Some(Interrupt::Webbed)), DRAWS);
        let strong = Ailing { webbed: true, strength: 300, ..calm() };
        assert_eq!(count(&strong, Some(Interrupt::Freed)), DRAWS);
    }

    #[test]
    fn blindness_takes_a_fifth_of_the_actions_unless_it_fights_blind() {
        let blind = Ailing { blind: true, ..calm() };
        assert!(about(count(&blind, Some(Interrupt::Blinded)), 20));
        let monk = Ailing { fights_blind: true, ..blind };
        assert_eq!(tally(&monk), [(None, DRAWS)]);
    }

    #[test]
    fn insanity_takes_four_fifths_of_the_actions_in_one_of_three_fits() {
        let mad = Ailing { insane: true, ..calm() };
        assert!(about(count(&mad, None), 20));
        for fit in 0..3 {
            // Each fit a third of 80 percent.
            assert!(about(count(&mad, Some(Interrupt::Insane { fit })), 27), "fit {fit}");
        }
        assert_eq!(count(&mad, Some(Interrupt::Berserk)), 0, "no friend to turn on");
    }

    #[test]
    fn insanity_with_a_friend_in_reach_turns_it_on_its_friends_two_fifths_of_the_time() {
        // 80 percent are taken; of those 40 percent turn: 32 percent of all.
        let mad = Ailing { insane: true, friend_in_reach: true, ..calm() };
        assert!(about(count(&mad, Some(Interrupt::Berserk)), 32));
        // Of the rest of the 80 percent, 48, a third each: 16 percent.
        assert!(about(count(&mad, Some(Interrupt::Insane { fit: 0 })), 16));
        // It has turned this round already: only the fits.
        let turned = Ailing { turned_on_friends: true, ..mad };
        assert_eq!(count(&turned, Some(Interrupt::Berserk)), 0);
        assert!(about(count(&turned, Some(Interrupt::Insane { fit: 1 })), 27));
    }

    #[test]
    fn fear_freezes_a_quarter_of_the_actions_unless_it_is_running() {
        let afraid = Ailing { afraid: true, ..calm() };
        assert!(about(count(&afraid, Some(Interrupt::Afraid)), 25));
        assert_eq!(count(&afraid, Some(Interrupt::Panics)), 0, "a member does not run");
        let running = Ailing { running: true, ..afraid };
        assert_eq!(count(&running, Some(Interrupt::Afraid)), 0);
    }

    #[test]
    fn fear_sends_a_monster_that_decides_for_itself_running_two_fifths_of_the_time() {
        // 25 percent frozen; of the 75 left, 40 percent run: 30 percent.
        let afraid = Ailing { afraid: true, runs: true, ..calm() };
        assert!(about(count(&afraid, Some(Interrupt::Afraid)), 25));
        assert!(about(count(&afraid, Some(Interrupt::Panics)), 30));
        // Running already: never frozen, and 40 percent run for its life again.
        let running = Ailing { running: true, ..afraid };
        assert_eq!(count(&running, Some(Interrupt::Afraid)), 0);
        assert!(about(count(&running, Some(Interrupt::Panics)), 40));
    }

    #[test]
    fn turncoat_lets_an_action_against_its_new_foes_stand() {
        let turned = Ailing { turncoat: true, against_foes: true, ..calm() };
        assert_eq!(tally(&turned), [(Some(Interrupt::Turncoat), DRAWS)]);
        let elsewhere = Ailing { against_foes: false, ..turned };
        assert_eq!(tally(&elsewhere), [(None, DRAWS)]);
    }

    #[test]
    fn the_first_that_applies_wins() {
        // Webbed and nauseated: a quarter lose it to nausea, the rest to the web.
        let both = Ailing { nauseated: true, webbed: true, strength: 2, ..calm() };
        assert!(about(count(&both, Some(Interrupt::Nauseated)), 25));
        assert!(about(count(&both, Some(Interrupt::Webbed)), 75));
        // A web, and then nothing else is looked at.
        let webbed_and_mad = Ailing { webbed: true, insane: true, strength: 2, ..calm() };
        assert_eq!(count(&webbed_and_mad, Some(Interrupt::Webbed)), DRAWS);
    }

    #[test]
    fn each_interrupt_has_its_retail_number() {
        let numbered = [
            (Interrupt::Webbed, 0),
            (Interrupt::Afraid, 1),
            (Interrupt::Blinded, 2),
            (Interrupt::Nauseated, 3),
            (Interrupt::Insane { fit: 0 }, 4),
            (Interrupt::Insane { fit: 2 }, 6),
            (Interrupt::Freed, 7),
            (Interrupt::Berserk, 8),
            (Interrupt::Turncoat, 9),
            (Interrupt::Panics, 12),
        ];
        for (interrupt, number) in numbered {
            assert_eq!(interrupt.number(), number, "{interrupt:?}");
        }
    }
}
