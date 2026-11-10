//! The rules by which an actor withstands what lands on it: saves,
//! resistance and cures, over the numbers its
//! [`Resistances`](crate::game::actors::Resistances) hold.

use crate::content::species::MonsterKind;
use crate::game::characters::{Condition, Conditions, Lasting};
use crate::game::magic::Strength;
use crate::game::rng::GameRng;

/// Retail's marker for a condition without end (`W8_CONDITION_INDEFINITE`),
/// which resistance does not cut.
pub const INDEFINITE: u32 = 9999;

/// Each condition's base save (retail `g_condition_resist_base`), by retail
/// number (Drained is 1, Dead 18).
pub const CONDITION_SAVE: [i32; 20] =
    [0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 18, 22, 24, 22];
pub const DEATH_SAVE: i32 = CONDITION_SAVE[18];

/// A condition's base save.
pub fn condition_save(condition: Condition) -> i32 {
    CONDITION_SAVE[condition as usize + 1]
}

/// The conditions a monster of `kind` shrugs off (retail
/// `g_condition_immunities`).
pub fn immune(kind: MonsterKind) -> &'static [Condition] {
    use Condition::*;
    match kind {
        MonsterKind::Undead => {
            &[Diseased, Poisoned, Unconscious, Irritated, Afraid, Insane, Asleep, Nauseated]
        }
        MonsterKind::Android => &[Diseased, Poisoned, Nauseated, Irritated, Afraid, Asleep],
        MonsterKind::Plant => &[Insane, Afraid, Nauseated],
        _ => &[],
    }
}

/// `(level − strength)·3 + base + resistance`, the chance a victim saves.
pub fn condition_chance(level: u32, resistance: i32, against: u32, base: i32) -> i32 {
    (level as i32 - against as i32) * 3 + base + resistance
}

/// A saving throw at `chance` percent, held within 5..95 (retail
/// `TargetResistsCondition`).
pub fn save_roll(rng: &mut GameRng, chance: i32) -> bool {
    (rng.below(100) as i32) < chance.clamp(5, 95)
}

/// Retail `ReduceMagnitudeByResistance`: resistance and the victim's level
/// over the attack's strength take a random share off; what is without end
/// (`Magic Effects.cpp:430-432`) is left whole.
pub fn resist(rng: &mut GameRng, amount: u32, resistance: i32, level: u32, against: u32) -> u32 {
    if amount == INDEFINITE {
        return amount;
    }
    let mut c = resistance + (level as i32 - against as i32) * 3;
    if c <= 0 {
        return amount;
    }
    c += 1;
    let half = c / 2;
    let share =
        (rng.below((c - half) as u32) as i32 + half + rng.below(half as u32) as i32).min(100);
    amount - amount * share as u32 / 100
}

/// The rounds a cure takes off a timed condition: `power²`, boosted.
pub fn cure_rounds(strength: Strength) -> u32 {
    let per = strength.power.pow(2);
    per + per * strength.boost / 100
}

/// Retail `TryCureConditionOnTargets`: a timed condition loses `power²`
/// rounds (and goes if that is all of them); a lasting one goes with chance
/// `power·15`%. A forced cure lifts them all.
pub fn cure(
    rng: &mut GameRng,
    conditions: &mut Conditions,
    cured: &[Condition],
    strength: Strength,
    forced: bool,
) -> Vec<Condition> {
    let per = cure_rounds(strength);
    let mut lifted = Vec::new();
    for &condition in cured {
        let Some(lasting) = conditions.lasting_mut(condition) else { continue };
        let gone = match lasting {
            _ if forced => true,
            Lasting::Rounds(left) if *left <= per => true,
            Lasting::Rounds(left) => {
                *left -= per;
                false
            }
            Lasting::Indefinite => rng.below(100) < strength.power * 15,
        };
        if gone {
            conditions.lift(condition);
            lifted.push(condition);
        }
    }
    lifted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With resistance 1000 against a level-50 victim and strength 0 the cut
    /// is the whole of any ordinary amount (the share is held to 100), yet a
    /// condition without end is left whole (`Magic Effects.cpp:430-432`).
    #[test]
    fn resistance_cuts_a_duration_but_not_what_has_no_end() {
        let mut rng = GameRng::seeded(5);
        for _ in 0..50 {
            assert_eq!(resist(&mut rng, INDEFINITE, 1000, 50, 0), INDEFINITE);
            assert!(resist(&mut rng, 40, 1000, 50, 0) < 40);
        }
    }
}
