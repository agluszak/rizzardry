//! The power of a cast, and the strength it goes off at
//! (`docs/rules/casting.md` section 3): each a function of the facts it is
//! about and nothing else. A rule that rolls takes the random-number
//! generator; none changes anything.

use crate::content::spells::{MAX_POWER, PowerChoice, SpellDefinition, Targeting};
use crate::game::characters::{Condition, Lasting, Pool, Vitals};
use crate::game::hits::definitions::Restores;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;

/// The power that asks the caster to choose it: as much as the victim needs
/// (retail "Auto Calibrate"), or, outside a fight, as much and as often as it
/// takes (retail "Auto Recast").
pub const AUTO: u32 = MAX_POWER + 1;

/// A failure chance over which the walk up the powers of a restoring spell
/// stops short, in percent.
const SURE_ENOUGH: u32 = 10;

/// The strength a cast goes off at (retail `GetSpellDifficulty`): half the
/// figure of whoever it comes from, the power, and half the spell's weight,
/// three less against all enemies and less the floor of a death spell, never
/// below 0, then raised by half the boost.
pub fn strength(figure: u32, power: u32, boost: u32, spell: &SpellDefinition) -> Strength {
    let against = figure / 2 + power + (spell.cost / 2 + spell.level) / 2;
    let all_enemies = if spell.targeting == Targeting::AllEnemies { 3 } else { 0 };
    let taken = spell.facts.death_floor + all_enemies;
    let against = against.max(taken) - taken;
    Strength { power, against: against + against * (boost / 2) / 100, boost }
}

/// The percent a member's Power Cast raises a cast by: a fourth of the skill
/// and one, while the skill is available to them.
pub fn boost(skill: u32, available: bool) -> u32 {
    if available { (skill >> 2) + 1 } else { 0 }
}

/// The highest power a realm's points pay for, 0 if none: its points left over
/// the spell's cost a power, held to seven; a spell that costs nothing is
/// always seven.
pub fn affordable(available: u32, cost: u32) -> u32 {
    match cost {
        0 => MAX_POWER,
        cost => (available / cost).min(MAX_POWER),
    }
}

/// A learned spell's power: the choice cut down to what the points pay for.
/// A spell whose power is fixed is cast at 1 whatever is asked, if the points
/// pay for that (retail `CastSpellFromSource`, `W8_SPELL_POWER_FIXED`).
pub fn chosen(asked: u32, affordable: u32, class: PowerChoice) -> u32 {
    let asked = if class == PowerChoice::Fixed { 1 } else { asked };
    asked.min(MAX_POWER).min(affordable)
}

/// The power an auto cast takes, held to what is affordable. A member lacking
/// health or stamina is healed by the lowest power whose average roll covers
/// it, stopping one short as soon as the failure chance at a power goes over
/// 10 percent; a monster is healed at 1. A cure takes the cheapest power to
/// outlast the longest of the conditions it lifts, each power delivering the
/// square of itself less the share the failure chance takes; a condition
/// without end is answered with 1. Any other spell takes 1.
pub fn auto(
    spell: &SpellDefinition,
    (restores, outlasts): (Option<Restores>, &[Condition]),
    victim: Option<(&Vitals, &crate::game::characters::Conditions, bool)>,
    affordable: u32,
    failure: [u32; MAX_POWER as usize],
    identify: Option<(u8, u32)>,
) -> u32 {
    if let Some((difficulty, boost)) = identify {
        return identifying(boost, difficulty).min(affordable);
    }
    let Some((victim, conditions, member)) = victim else { return 1 };
    let failure = |power: u32| failure[power as usize - 1];
    let power = if outlasts.is_empty() {
        let Some(restores) = restores else { return 1 };
        let lacks = |pool: Pool| pool.max.max(pool.current) - pool.current;
        let missing = match restores {
            Restores::Stamina => lacks(victim.stamina),
            Restores::HealthThenStamina if lacks(victim.health) == 0 => lacks(victim.stamina),
            _ => lacks(victim.health),
        };
        // The average roll, cut down to a whole number.
        let average = |power: u32| {
            spell.dice.map_or(0, |d| (2 * d.base + d.count * power as i32 * (d.sides + 1)) / 2)
        };
        if !member || missing == 0 {
            1
        } else {
            (1..=MAX_POWER)
                .find_map(|power| {
                    if failure(power) > SURE_ENOUGH {
                        Some((power - 1).max(1))
                    } else {
                        ((missing as i32) < average(power)).then_some(power)
                    }
                })
                // Retail's eight: as much as the points pay for.
                .unwrap_or(AUTO)
        }
    } else {
        let mut turns = None;
        for &(condition, lasting) in conditions.iter() {
            if outlasts.contains(&condition) {
                match lasting {
                    // Nothing outlasts it: the least power.
                    Lasting::Indefinite => return 1,
                    Lasting::Rounds(left) => turns = turns.max(Some(left)),
                }
            }
        }
        match turns {
            None => 1,
            Some(turns) => (1..=affordable.max(1))
                .map(|power| {
                    // What one cast really delivers: the square of the power,
                    // less the share the failure chance takes.
                    let square = power.pow(2);
                    let per = (square - (failure(power) * square / 100).min(square)).max(1);
                    (spell.cost * turns.div_ceil(per) * power, power)
                })
                .min()
                .map_or(1, |(_, power)| power),
        }
    };
    power.min(affordable)
}

/// The power an instrument or gadget is played at (retail `UseItem`): the
/// player tries powers from 7 down, asking the item's failure chance at each
/// without the round's pace, and steps down one while it is over 15 percent;
/// at the first power at or under it, starting from a chance of 7 a power,
/// steps further down while the chance is over a roll below 100, taking 7 off
/// the chance each step. 0 is played badly.
pub fn played(rng: &mut GameRng, failure: [u32; MAX_POWER as usize]) -> u32 {
    let mut power = MAX_POWER;
    while power > 0 && failure[power as usize - 1] > 15 {
        power -= 1;
    }
    while power > 0 && power * 7 > rng.below(100) {
        power -= 1;
    }
    power
}

/// What it takes to go on casting a repeated spell (retail
/// `ExecuteCharacterAction`): stamina above the spell's level and points for
/// power 1.
pub fn goes_again(stamina: u32, level: u32, available: u32, cost: u32) -> bool {
    stamina > level && available >= cost
}

/// The least power at which a cast reveals an item this hard (retail
/// `ApplyIdentifyAttempt`), 7 if none does.
pub fn identifying(boost: u32, difficulty: u8) -> u32 {
    (1..=MAX_POWER).find(|&power| reveals(power, boost, difficulty)).unwrap_or(MAX_POWER)
}

/// Whether a cast at `power` reveals an item: three a power, raised by the
/// boost percent, is at least the item's identify difficulty.
pub fn reveals(power: u32, boost: u32, difficulty: u8) -> bool {
    let score = power * 3;
    score + score * boost / 100 >= u32::from(difficulty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Realm;
    use crate::content::items::Dice;
    use crate::content::spells::SpellId;
    use crate::game::characters::Conditions;
    use crate::game::hits::definitions::Restores;

    fn spell(cost: u32, level: u32, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id: SpellId::ENERGY_BLAST,
            realm: Some(Realm::Fire),
            cost,
            level,
            targeting,
            ..Default::default()
        }
    }

    /// Figure 20, power 3, cost 6, level 4: 20/2 + 3 + (6/2 + 4)/2 = 10 + 3 + 3 = 16.
    #[test]
    fn the_strength_of_a_cast_grows_with_its_figure_power_and_weight() {
        let blast = spell(6, 4, Targeting::Enemy);
        assert_eq!(strength(20, 3, 0, &blast), Strength { power: 3, against: 16, boost: 0 });
        // An odd figure is cut down: 21/2 = 10.
        assert_eq!(strength(21, 3, 0, &blast).against, 16);
        // Cost 7: (7/2 + 4)/2 = (3 + 4)/2 = 3 again; cost 8: (4 + 4)/2 = 4.
        assert_eq!(strength(20, 3, 0, &spell(7, 4, Targeting::Enemy)).against, 16);
        assert_eq!(strength(20, 3, 0, &spell(8, 4, Targeting::Enemy)).against, 17);
    }

    /// A death spell takes its floor off, as the all-enemies three; 16 - 6 =
    /// 10 and 10 - 3 - 6 = 1 for both; never below 0: a figure of 0 at power 1,
    /// cost 2, level 1 is 0 + 1 + (1 + 1)/2 = 2, and 2 - 6 is held at 0.
    #[test]
    fn a_death_spell_takes_its_floor_off_but_never_below_nothing() {
        let mut death = spell(6, 4, Targeting::Enemy);
        death.facts.death_floor = 6;
        assert_eq!(strength(20, 3, 0, &death).against, 10);
        death.targeting = Targeting::AllEnemies;
        assert_eq!(strength(20, 3, 0, &death).against, 7);
        let mut small = spell(2, 1, Targeting::Enemy);
        small.facts.death_floor = 6;
        assert_eq!(strength(0, 1, 0, &small).against, 0);
    }

    #[test]
    fn a_chosen_power_is_cut_down_to_what_the_points_pay_for() {
        let chosen = |asked, affordable| super::chosen(asked, affordable, PowerChoice::Chosen);
        assert_eq!(chosen(5, 3), 3);
        assert_eq!(chosen(2, 6), 2);
        assert_eq!(chosen(7, 7), 7);
        // The auto choice is resolved before it is chosen; it is held to seven.
        assert_eq!(chosen(AUTO, 9), 7);
        assert_eq!(chosen(5, 0), 0);
        // A spell of fixed power goes off at 1 whatever was asked, or not at
        // all when the points do not pay for one.
        let fixed = |asked, affordable| super::chosen(asked, affordable, PowerChoice::Fixed);
        assert_eq!((fixed(5, 7), fixed(1, 7), fixed(5, 0)), (1, 1, 0));
    }

    fn dice(count: i32, sides: i32) -> Option<Dice> {
        Some(Dice { base: 0, count, sides })
    }

    /// Someone an auto power is worked out for.
    #[derive(Clone, Copy)]
    struct Victim<'a> {
        member: bool,
        health: Pool,
        stamina: Pool,
        conditions: &'a Conditions,
    }

    /// An auto cast of a spell that costs 3 a power.
    struct Auto<'a> {
        affordable: u32,
        victim: Option<Victim<'a>>,
        dice: Option<Dice>,
        restores: Option<Restores>,
        outlasts: &'a [Condition],
        failure: [u32; 7],
        identify: Option<(u8, u32)>,
    }

    fn auto(asked: &Auto) -> u32 {
        let spell = SpellDefinition { cost: 3, dice: asked.dice, ..Default::default() };
        let victim = asked.victim.map(|victim| {
            let vitals =
                Vitals { health: victim.health, stamina: victim.stamina, ..Default::default() };
            (vitals, victim.conditions, victim.member)
        });
        super::auto(
            &spell,
            (asked.restores, asked.outlasts),
            victim.as_ref().map(|(vitals, conditions, member)| (vitals, *conditions, *member)),
            asked.affordable,
            asked.failure,
            asked.identify,
        )
    }

    fn victim(member: bool, health: (u32, u32), stamina: (u32, u32), c: &Conditions) -> Victim<'_> {
        Victim {
            member,
            health: Pool { current: health.0, max: health.1 },
            stamina: Pool { current: stamina.0, max: stamina.1 },
            conditions: c,
        }
    }

    fn healing<'a>(
        dice: Option<Dice>,
        restores: Restores,
        victim: Victim<'a>,
        affordable: u32,
        failure: [u32; 7],
    ) -> Auto<'a> {
        Auto {
            affordable,
            victim: Some(victim),
            dice,
            restores: Some(restores),
            outlasts: &[],
            failure,
            identify: None,
        }
    }

    /// 1d4 averages (0 + 1 * p * 5) / 2 = 2 at power 1 (2.5 cut down), 5 at
    /// power 2: a victim lacking 2 is not covered at 1 (2 < 2 is false) but is
    /// at 2. A monster is healed at 1, and nobody lacking nothing at more.
    #[test]
    fn a_heal_is_chosen_by_the_average_roll_cut_to_a_whole_number() {
        let none = Conditions::default();
        let hurt = victim(true, (8, 10), (10, 10), &none);
        let sure = [0; 7];
        assert_eq!(auto(&healing(dice(1, 4), Restores::Health, hurt, 7, sure)), 2);
        let monster = victim(false, (8, 10), (10, 10), &none);
        assert_eq!(auto(&healing(dice(1, 4), Restores::Health, monster, 7, sure)), 1);
        let whole = victim(true, (10, 10), (10, 10), &none);
        assert_eq!(auto(&healing(dice(1, 4), Restores::Health, whole, 7, sure)), 1);
    }

    /// A failure chance over 10 from power 3 stops the walk at 2.
    #[test]
    fn a_walk_up_the_powers_stops_one_short_of_a_failure_over_ten_percent() {
        let none = Conditions::default();
        let dead_tired = victim(true, (0, 10), (10, 10), &none);
        let unsure = [0, 0, 20, 20, 20, 20, 20];
        assert_eq!(auto(&healing(dice(1, 1), Restores::Health, dead_tired, 7, unsure)), 2);
        // Exactly 10 does not stop it.
        let borderline = [0, 0, 10, 10, 10, 10, 10];
        assert_eq!(auto(&healing(dice(1, 1), Restores::Health, dead_tired, 7, borderline)), 7);
        // Failing from power 1 is held to 1: (1 - 1).max(1).
        let hopeless = [11; 7];
        assert_eq!(auto(&healing(dice(1, 1), Restores::Health, dead_tired, 7, hopeless)), 1);
    }

    /// Stamina measures a lack of stamina; Restoration measures health, and
    /// stamina only once health is full. 1d1 averages p: a lack of 6 wants 7, of 2 wants 3.
    #[test]
    fn restoring_measures_the_pool_it_mends() {
        let none = Conditions::default();
        let sure = [0; 7];
        let tired = victim(true, (10, 10), (4, 10), &none);
        assert_eq!(auto(&healing(dice(1, 1), Restores::Stamina, tired, 7, sure)), 7);
        let hurt_and_tired = victim(true, (8, 10), (4, 10), &none);
        // Health lacks 2: power 3; stamina is not looked at.
        assert_eq!(
            auto(&healing(dice(1, 1), Restores::HealthThenStamina, hurt_and_tired, 7, sure)),
            3
        );
        // Health full, stamina lacks 6: power 7.
        let rested = victim(true, (10, 10), (4, 10), &none);
        assert_eq!(auto(&healing(dice(1, 1), Restores::HealthThenStamina, rested, 7, sure)), 7);
        // Heal Wounds does not look at stamina: it lacks nothing.
        assert_eq!(auto(&healing(dice(1, 1), Restores::Health, rested, 7, sure)), 1);
    }

    fn curing<'a>(
        victim: Victim<'a>,
        outlasts: &'a [Condition],
        affordable: u32,
        failure: [u32; 7],
    ) -> Auto<'a> {
        Auto {
            affordable,
            victim: Some(victim),
            dice: None,
            restores: None,
            outlasts,
            failure,
            identify: None,
        }
    }

    /// Cost 3, a poison of 20 rounds, no failure: power p delivers p * p a
    /// cast, so it takes ceil(20 / p^2) casts of 3 * p points: 20 * 3 = 60,
    /// 5 * 6 = 30, 3 * 9 = 27, 2 * 12 = 24, 1 * 15 = 15, 1 * 18 = 18, 1 * 21 =
    /// 21 for powers 1 to 7. Power 5 is cheapest.
    #[test]
    fn a_cure_takes_the_cheapest_power_to_outlast_the_poison() {
        let poisoned = Conditions::from([(Condition::Poisoned, Lasting::Rounds(20))]);
        let v = victim(true, (10, 10), (10, 10), &poisoned);
        assert_eq!(auto(&curing(v, &[Condition::Poisoned], 7, [0; 7])), 5);
        // Held to what the points pay for: 4 costs 24 of 60, 30, 27, 24.
        assert_eq!(auto(&curing(v, &[Condition::Poisoned], 4, [0; 7])), 4);
        // A failure chance of 50 percent halves what a cast delivers:
        // p * p - 50 * p * p / 100 = 1 - 0, 4 - 2, 9 - 4, 16 - 8, 25 - 12, 36 - 18, 49 - 24
        // = 1, 2, 5, 8, 13, 18, 25. Casts 20, 10, 4, 3, 2, 2, 1; points
        // 60, 60, 36, 36, 30, 36, 21: power 7.
        assert_eq!(auto(&curing(v, &[Condition::Poisoned], 7, [50; 7])), 7);
    }

    /// 90 points over a cost of 30 is 3 powers; 100 over 14 is 7 (cut from 7.14);
    /// 1000 over 3 is held to 7; nothing over a cost, or less than one cost, none;
    /// a spell that costs nothing is always 7.
    #[test]
    fn the_points_pay_for_so_many_powers_up_to_seven() {
        assert_eq!(affordable(90, 30), 3);
        assert_eq!(affordable(100, 14), 7);
        assert_eq!(affordable(1000, 3), 7);
        assert_eq!(affordable(0, 3), 0);
        assert_eq!(affordable(2, 3), 0);
        assert_eq!(affordable(3, 3), 1);
        assert_eq!(affordable(0, 0), 7);
    }
}
