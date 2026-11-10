//! A monster's conduct at its moment in a fight (`docs/rules/monsters.md`
//! section 4): run, withdraw, hold back, or fight on.

use crate::content::species::Reach;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// A monster's choice at its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conduct {
    /// It runs for its life, and keeps running in later rounds while the
    /// party stays off.
    Flee,
    /// It moves off from the party.
    Withdraw,
    /// It does nothing this round.
    Hold,
    /// It attacks, or closes in to.
    Fight,
}

/// A health or stamina as a share of its most, in percent (whole).
pub fn share(current: u32, most: u32) -> u32 {
    current * 100 / most.max(1)
}

/// What conduct weighs.
#[derive(Clone, Copy, Debug)]
pub struct Conducting {
    /// Its health as a share of its most, in percent (whole).
    pub health: u32,
    /// The chance (%) its kind runs for its life at its turn.
    pub flee_chance: u8,
    /// Its kind is craven: with no flee chance of its own it always runs.
    pub craven: bool,
    /// Its kind keeps its place hidden rather than withdraw while neutral.
    pub camouflaged: bool,
    pub npc: bool,
    /// The species retail keeps out of every quarrel.
    pub bystander: bool,
    /// Its side toward the party now.
    pub stance: Stance,
    /// It ran in an earlier round.
    pub fleeing: bool,
    /// How far the party is.
    pub party_gap: f32,
}

/// The conduct chosen, and whether the monster is running from now on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conducted {
    pub conduct: Conduct,
    pub fleeing: bool,
}

/// The chance (%) it runs this turn, and whether a flight already under way
/// goes on (retail `MonsterAdvanceChance`, `MonsterAI.cpp:1034-1073`): a badly
/// hurt NPC not hostile always runs; a running one keeps running while the
/// party is beyond short reach; others by their own chance, or always when
/// craven.
fn flee_chance(facts: &Conducting) -> (u32, bool) {
    if facts.npc && facts.stance != Stance::Hostile && facts.health <= 33 {
        return (100, facts.fleeing);
    }
    if facts.fleeing {
        return if facts.party_gap > Reach::Short.metres() { (100, true) } else { (0, false) };
    }
    match facts.flee_chance {
        0 if facts.craven => (100, false),
        chance => (u32::from(chance), false),
    }
}

/// Decide a monster's conduct this turn: a bystander only ever holds; the rest
/// run at their chance, then a neutral one holds if camouflaged and otherwise
/// withdraws, and any other fights.
pub fn conduct(rng: &mut GameRng, facts: &Conducting) -> Conducted {
    if facts.bystander {
        return Conducted { conduct: Conduct::Hold, fleeing: facts.fleeing };
    }
    let (chance, going_on) = flee_chance(facts);
    if rng.below(100) < chance {
        return Conducted { conduct: Conduct::Flee, fleeing: true };
    }
    let conduct = match facts.stance {
        Stance::Neutral if facts.camouflaged => Conduct::Hold,
        Stance::Neutral => Conduct::Withdraw,
        _ => Conduct::Fight,
    };
    Conducted { conduct, fleeing: going_on }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady hostile monster in full health, the party 10 m off.
    fn steady() -> Conducting {
        Conducting {
            health: 100,
            flee_chance: 0,
            craven: false,
            camouflaged: false,
            npc: false,
            bystander: false,
            stance: Stance::Hostile,
            fleeing: false,
            party_gap: 10.0,
        }
    }

    /// The conduct over many draws: every distinct result.
    fn over_draws(facts: &Conducting) -> Vec<Conducted> {
        let mut seen: Vec<Conducted> = Vec::new();
        let mut rng = GameRng::seeded(11);
        for _ in 0..400 {
            let got = conduct(&mut rng, facts);
            if !seen.contains(&got) {
                seen.push(got);
            }
        }
        seen
    }

    fn fights() -> Conducted {
        Conducted { conduct: Conduct::Fight, fleeing: false }
    }

    fn flees() -> Conducted {
        Conducted { conduct: Conduct::Flee, fleeing: true }
    }

    #[test]
    fn a_craven_kind_with_no_chance_of_its_own_always_runs() {
        // Page 4, 2: chance 0 and craven: 100, so below(100) < 100 always.
        assert_eq!(over_draws(&Conducting { craven: true, ..steady() }), [flees()]);
        // With a chance of its own, that is the chance: 100 always, 0 never.
        let own = Conducting { craven: true, flee_chance: 100, ..steady() };
        assert_eq!(over_draws(&own), [flees()]);
    }

    #[test]
    fn a_running_monster_goes_on_running_while_the_party_is_beyond_short_reach() {
        // Page 4, 2: 100 while the party is farther than 4 m (so at 4.5 m).
        let running = Conducting { fleeing: true, party_gap: 4.5, ..steady() };
        assert_eq!(over_draws(&running), [flees()]);
        // Exactly 4 m is within short reach: it is cornered and stops running.
        let cornered = Conducting { fleeing: true, party_gap: 4.0, ..steady() };
        assert_eq!(over_draws(&cornered), [fights()]);
    }

    #[test]
    fn a_badly_hurt_npc_not_hostile_always_runs() {
        // Page 4, 2: an NPC not hostile at 33 percent of its health or less.
        let hurt = Conducting { npc: true, stance: Stance::Neutral, health: 33, ..steady() };
        assert_eq!(over_draws(&hurt), [flees()]);
        let friendly = Conducting { stance: Stance::Friendly, ..hurt };
        assert_eq!(over_draws(&friendly), [flees()]);
        // 34 percent is not hurt enough: a neutral NPC withdraws.
        let well = Conducting { health: 34, ..hurt };
        assert_eq!(over_draws(&well), [Conducted { conduct: Conduct::Withdraw, fleeing: false }]);
        // A hostile NPC never takes this chance.
        assert_eq!(over_draws(&Conducting { stance: Stance::Hostile, ..hurt }), [fights()]);
        // Nor does a monster that is no NPC.
        assert_eq!(
            over_draws(&Conducting { npc: false, ..hurt }),
            [Conducted { conduct: Conduct::Withdraw, fleeing: false }]
        );
    }

    #[test]
    fn a_neutral_monster_withdraws_or_holds_if_its_kind_keeps_hidden() {
        let neutral = Conducting { stance: Stance::Neutral, ..steady() };
        assert_eq!(
            over_draws(&neutral),
            [Conducted { conduct: Conduct::Withdraw, fleeing: false }]
        );
        let hidden = Conducting { camouflaged: true, ..neutral };
        assert_eq!(over_draws(&hidden), [Conducted { conduct: Conduct::Hold, fleeing: false }]);
    }
}
