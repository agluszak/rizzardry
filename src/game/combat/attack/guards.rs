//! Who steps in to take a blow meant for another (`docs/rules/attack.md`
//! section 4, "Guards intercepting"; retail `ResolveGuardianInterception`).

use super::Surprise;
use crate::content::species::Reach;
use crate::game::rng::GameRng;

/// How well a body presses a blow past a guard, or guards against one.
#[derive(Clone, Copy, Debug)]
pub struct Press {
    /// Speed + to hit and rating of the main hand (a monster's first attack's
    /// score), before fatigue (retail `GetCharacterProtectionScore`,
    /// `GetMonsterProtectionScore`).
    pub press: i32,
    pub fatigue: i32,
    /// Fatigue tells half as much on it.
    pub tires_little: bool,
}

impl Press {
    /// The press less its fatigue (halved for one who tires little).
    pub fn pressing(&self) -> i32 {
        self.press - if self.tires_little { self.fatigue / 2 } else { self.fatigue }
    }
}

/// A body guarding the victim of a blow.
#[derive(Clone, Copy, Debug)]
pub struct Guard {
    /// Alive, and nothing as bad as Blind.
    pub able: bool,
    /// The reach of its first arm.
    pub reach: Option<Reach>,
    /// The gap between it and its ward.
    pub gap: f32,
    /// Interceptions it has made this round, and how many it may.
    pub taken: u32,
    pub allowed: u32,
    pub press: Press,
}

/// What stepping in weighs.
#[derive(Clone, Copy, Debug)]
pub struct StepIn<'a> {
    /// The victim's guards in the order they took it up.
    pub guards: &'a [Guard],
    /// The blow's attacker.
    pub attacker: Press,
    pub surprise: Surprise,
}

/// The guard that takes the blow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stepping {
    /// By index.
    pub guard: usize,
    /// The stamina it loses to it.
    pub tires: u32,
}

/// The chance a guard steps in, in percent: its press against the attacker's,
/// the attacker's raised by 10 for an unaware blow and 10 for a natural one,
/// `guard - attacker + 50` held to 5 to 95.
pub fn stepping_chance(guard: &Press, attacker: &Press, surprise: Surprise) -> u32 {
    let pressing =
        attacker.pressing() + 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
    (guard.pressing() - pressing + 50).clamp(5, 95) as u32
}

/// Which guard steps in for a swing that has reached: those able, with the
/// ward within reach of their first arm and interceptions left each step in
/// with their chance, and one of those at random takes the blow, tired
/// `below(3) + 2` by it.
pub fn steps_in(rng: &mut GameRng, facts: &StepIn) -> Option<Stepping> {
    let stepping: Vec<usize> = facts
        .guards
        .iter()
        .enumerate()
        .filter(|(_, guard)| {
            guard.able
                && guard.reach.is_some_and(|reach| guard.gap <= reach.metres())
                && guard.taken < guard.allowed
                && rng.below(100) < stepping_chance(&guard.press, &facts.attacker, facts.surprise)
        })
        .map(|(index, _)| index)
        .collect();
    if stepping.is_empty() {
        return None;
    }
    let guard = stepping[rng.below(stepping.len() as u32) as usize];
    Some(Stepping { guard, tires: rng.below(3) + 2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(press: i32) -> Press {
        Press { press, fatigue: 0, tires_little: false }
    }

    /// A guard as likely as can be to step in: its press far above.
    fn guard() -> Guard {
        Guard {
            able: true,
            reach: Some(Reach::Touch),
            gap: 1.0,
            taken: 0,
            allowed: 2,
            press: press(1000),
        }
    }

    fn step_in<'a>(guards: &'a [Guard]) -> StepIn<'a> {
        StepIn { guards, attacker: press(10), surprise: Surprise::default() }
    }

    /// Who steps in over many draws.
    fn who(facts: &StepIn) -> Vec<Option<usize>> {
        (0..300)
            .map(|seed| steps_in(&mut GameRng::seeded(seed), facts).map(|stepping| stepping.guard))
            .collect()
    }

    #[test]
    fn a_press_is_tired_by_fatigue_halved_for_one_who_tires_little() {
        assert_eq!(Press { press: 90, fatigue: 20, tires_little: false }.pressing(), 70);
        assert_eq!(Press { press: 90, fatigue: 20, tires_little: true }.pressing(), 80);
        // 5 / 2 = 2.
        assert_eq!(Press { press: 90, fatigue: 5, tires_little: true }.pressing(), 88);
    }

    #[test]
    fn the_chance_is_the_guards_press_less_the_attackers_plus_50_held_to_5_to_95() {
        let chance =
            |guard, attacker, surprise| stepping_chance(&press(guard), &press(attacker), surprise);
        let none = Surprise::default();
        assert_eq!(chance(100, 100, none), 50);
        assert_eq!(chance(120, 100, none), 70);
        assert_eq!(chance(80, 100, none), 30);
        // Held: 200 - 100 + 50 = 150 -> 95; 0 - 100 + 50 = -50 -> 5.
        assert_eq!(chance(200, 100, none), 95);
        assert_eq!(chance(0, 100, none), 5);
        // The edge: 145 - 100 + 50 = 95 exactly, and above it still 95.
        assert_eq!(chance(145, 100, none), 95);
        assert_eq!(chance(146, 100, none), 95);
    }

    #[test]
    fn the_attackers_press_is_raised_by_ten_for_each_way_the_blow_surprises() {
        let guard = press(100);
        let attacker = press(100);
        let by =
            |unaware, natural| stepping_chance(&guard, &attacker, Surprise { unaware, natural });
        assert_eq!(by(true, false), 40); // 100 - 110 + 50
        assert_eq!(by(false, true), 40);
        assert_eq!(by(true, true), 30); // 100 - 120 + 50
    }

    #[test]
    fn fatigue_counts_for_both_presses() {
        let tired = Press { press: 100, fatigue: 20, tires_little: false };
        // 80 - 100 + 50 = 30; the attacker tired: 80 - 80 + 50 = 50.
        assert_eq!(stepping_chance(&tired, &press(100), Surprise::default()), 30);
        assert_eq!(stepping_chance(&tired, &tired, Surprise::default()), 50);
    }

    #[test]
    fn no_guards_means_the_blow_stays_with_its_ward() {
        assert_eq!(who(&step_in(&[])).iter().flatten().count(), 0);
    }

    #[test]
    fn a_guard_steps_in_with_its_chance_and_not_always() {
        // Guard 1000 against attacker 10: held to 95 percent.
        let guards = [guard()];
        let stepped = who(&step_in(&guards));
        let yes = stepped.iter().flatten().count();
        assert!((250..300).contains(&yes), "{yes} of 300 at 95 percent");
        // At the least, 5: it seldom does.
        let weak = [Guard { press: press(-1000), ..guard() }];
        let yes = who(&step_in(&weak)).iter().flatten().count();
        assert!(yes < 50, "{yes} of 300 at 5 percent");
    }

    #[test]
    fn a_guard_must_be_able_in_reach_of_its_ward_and_have_interceptions_left() {
        let blind = Guard { able: false, ..guard() };
        // Its first arm's reach in metres at least the gap: touch is 2.
        let far = Guard { gap: 2.5, ..guard() };
        let near = Guard { gap: 2.0, ..guard() };
        let long = Guard { gap: 20.0, reach: Some(Reach::Long), ..guard() };
        let unarmed = Guard { reach: None, ..guard() };
        let spent = Guard { taken: 2, ..guard() };
        for out in [blind, far, unarmed, spent] {
            assert_eq!(who(&step_in(&[out])).iter().flatten().count(), 0, "{out:?}");
        }
        for ready in [near, long, Guard { taken: 1, ..guard() }] {
            assert!(who(&step_in(&[ready])).iter().flatten().count() > 200, "{ready:?}");
        }
    }

    #[test]
    fn one_of_those_stepping_in_takes_the_blow_and_only_one_who_could() {
        let guards =
            [Guard { able: false, ..guard() }, guard(), Guard { taken: 2, ..guard() }, guard()];
        let chosen = who(&step_in(&guards));
        for index in chosen.iter().flatten() {
            assert!([1, 3].contains(index), "{index}");
        }
        // Each of the two takes it in some draws.
        assert!(chosen.contains(&Some(1)) && chosen.contains(&Some(3)));
    }

    #[test]
    fn the_guard_is_tired_by_two_to_four() {
        let guards = [guard()];
        for seed in 0..100 {
            if let Some(stepping) = steps_in(&mut GameRng::seeded(seed), &step_in(&guards)) {
                assert!((2..=4).contains(&stepping.tires));
            }
        }
    }

    #[test]
    fn a_stronger_press_steps_in_more_often() {
        let weaker = [Guard { press: press(50), ..guard() }];
        let stronger = [Guard { press: press(80), ..guard() }];
        // Against 60: 50 - 60 + 50 = 40 percent; 80 - 60 + 50 = 70 percent.
        let facts = |guards| StepIn { attacker: press(60), ..step_in(guards) };
        let weak = who(&facts(&weaker)).iter().flatten().count();
        let strong = who(&facts(&stronger)).iter().flatten().count();
        assert!(weak < strong, "{weak} < {strong}");
        assert!((90..150).contains(&weak), "{weak} of 300 at 40 percent");
        assert!((180..240).contains(&strong), "{strong} of 300 at 70 percent");
    }
}
