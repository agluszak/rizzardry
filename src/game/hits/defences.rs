//! The rules by which an actor withstands what lands on it: saves,
//! resistance and cures, over the numbers its
//! [`Resistances`](crate::game::actors::Resistances) hold.

use super::definitions::{EffectKind, EffectSpec};
use crate::content::species::MonsterKind;
use crate::game::actors::Resistances;
use crate::game::characters::{Condition, Conditions, Lasting};
use crate::game::difficulty::Difficulty;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

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

/// What a cure did to what ails a victim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cured {
    /// The conditions it lifted.
    pub lifted: Vec<Condition>,
    /// The timed conditions it shortened without lifting, and what they have left.
    pub shortened: Vec<(Condition, u32)>,
}

/// Retail `TryCureConditionOnTargets`: for each condition the spell names that
/// the victim bears, a timed one loses `power²` rounds (raised by the boost)
/// and goes if that is all of them; one without end goes with chance
/// `power·15`%. An outright cure lifts them all.
pub fn cure(
    rng: &mut GameRng,
    conditions: &Conditions,
    cured: &[Condition],
    strength: Strength,
    outright: bool,
) -> Cured {
    let per = cure_rounds(strength);
    let mut done = Cured::default();
    for &condition in cured {
        let Some(lasting) = conditions.lasting(condition) else { continue };
        let gone = match lasting {
            _ if outright => true,
            Lasting::Rounds(left) if left <= per => true,
            Lasting::Rounds(left) => {
                done.shortened.push((condition, left - per));
                false
            }
            Lasting::Indefinite => rng.below(100) < strength.power * 15,
        };
        if gone {
            done.lifted.push(condition);
        }
    }
    done
}

/// Whether an effect needs a living body to act on: harm, drain, afflict, slay,
/// tire, heal, refresh and replenish do nothing to the dead.
pub fn needs_the_living(kind: &EffectKind) -> bool {
    use EffectKind as K;
    matches!(
        kind,
        K::Harm
            | K::Sap
            | K::Afflict { .. }
            | K::Slay
            | K::Tire
            | K::DrainPool
            | K::DrainBudget
            | K::DrainRealm(_)
            | K::Heal
            | K::Refresh
            | K::Replenish
    )
}

/// The victim of an effect, as its defences find it.
#[derive(Clone, Copy, Debug)]
pub struct Victim<'a> {
    pub dead: bool,
    pub level: u32,
    /// The side it is on, by which its saves are scaled for difficulty.
    pub stance: Stance,
    pub resistances: &'a Resistances,
}

/// What defending against one effect weighs (retail `ApplyEffectConditions`).
#[derive(Clone, Copy, Debug)]
pub struct Defending<'a> {
    pub row: &'a EffectSpec,
    /// The magnitude the effect rolled, before its save.
    pub rolled: u32,
    pub strength: Strength,
    pub difficulty: Difficulty,
    pub victim: Victim<'a>,
}

/// How an effect was stopped, or the magnitude it lands with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Defended {
    Through(u32),
    /// Nothing to act on: the body was dead, or the effect did not happen.
    Missed,
    Resisted {
        /// The victim saved against it, which a member practises Iron Will for.
        saved: bool,
    },
}

/// Validate and resist one effect before it is applied, in the order of the
/// page: the dead, its chance, immunity, the save, resistance, wards. `half`
/// halves the strength it weighs against and the rounds rolled. The magnitude
/// it lands with is cut by resistance; one cut to nothing is resisted.
pub fn defend(rng: &mut GameRng, facts: &Defending) -> Defended {
    use EffectKind as K;
    let Defending { row, rolled, strength, difficulty, victim } = *facts;
    let mut magnitude = if row.half { rolled / 2 } else { rolled };
    if needs_the_living(&row.kind) && victim.dead {
        return Defended::Missed;
    }
    if row.chance.is_some_and(|chance| rng.below(100) >= chance) {
        return Defended::Missed;
    }
    let against = if row.half { strength.against / 2 } else { strength.against };
    let affliction = match row.kind {
        K::Afflict { condition, .. } => Some(condition),
        _ => None,
    };
    let mut resisted = match affliction {
        Some(condition) => victim.resistances.immune.contains(&condition),
        None => matches!(row.kind, K::Slay) && victim.resistances.death_immune,
    };
    let mut saved = false;
    if !resisted && let (Some(save), Some(realm)) = (row.save, row.resist) {
        let resistance = victim.resistances.realms[realm];
        let chance = condition_chance(victim.level, resistance, against, save);
        resisted = save_roll(rng, difficulty.scale(chance, victim.stance));
        saved = resisted;
    }
    if !resisted
        && row.roll.is_some()
        && let Some(realm) = row.resist
    {
        let resistance = victim.resistances.realms[realm];
        magnitude = resist(rng, magnitude, resistance, victim.level, against);
        resisted = magnitude == 0;
    }
    // What it shrugs off whenever a save is allowed at all.
    if !resisted
        && row.save.is_some()
        && let Some(condition) = affliction
    {
        resisted = victim.resistances.wards.contains(&condition);
    }
    if resisted { Defended::Resisted { saved } } else { Defended::Through(magnitude) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Realm;
    use crate::game::hits::definitions::Roll;

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

    fn row(kind: EffectKind) -> EffectSpec {
        EffectSpec { kind, roll: Some(Roll::Fixed(0)), ..Default::default() }
    }

    fn poison() -> EffectKind {
        EffectKind::Afflict { condition: Condition::Poisoned, poison: 0, indefinite: false }
    }

    fn victim(resistances: &Resistances) -> Victim<'_> {
        Victim { dead: false, level: 10, stance: Stance::Hostile, resistances }
    }

    fn defending<'a>(row: &'a EffectSpec, rolled: u32, victim: Victim<'a>) -> Defending<'a> {
        Defending {
            row,
            rolled,
            strength: Strength { power: 3, against: 0, boost: 0 },
            difficulty: Difficulty::Normal,
            victim,
        }
    }

    fn defended(facts: &Defending, seed: u64) -> Defended {
        defend(&mut GameRng::seeded(seed), facts)
    }

    #[test]
    fn the_dead_are_missed_by_what_needs_the_living_only() {
        let none = Resistances::default();
        let dead = Victim { dead: true, ..victim(&none) };
        for kind in [EffectKind::Harm, EffectKind::Heal, EffectKind::Slay, poison()] {
            let harm = row(kind.clone());
            assert_eq!(defended(&defending(&harm, 5, dead), 0), Defended::Missed, "{kind:?}");
        }
        for kind in [EffectKind::Raise, EffectKind::Cure { conditions: vec![], outright: false }] {
            let raise = row(kind);
            assert_eq!(defended(&defending(&raise, 5, dead), 0), Defended::Through(5));
        }
    }

    /// A condition its kind cannot suffer is resisted without a save (not
    /// "saved"), and so is a death it cannot suffer.
    #[test]
    fn immunity_resists_before_any_save() {
        let immune = Resistances { immune: &[Condition::Poisoned], ..Default::default() };
        let saving = EffectSpec { save: Some(0), resist: Some(Realm::Water), ..row(poison()) };
        for seed in 0..50 {
            assert_eq!(
                defended(&defending(&saving, 5, victim(&immune)), seed),
                Defended::Resisted { saved: false }
            );
        }
        let deathless = Resistances { death_immune: true, ..Default::default() };
        let slay = row(EffectKind::Slay);
        assert_eq!(
            defended(&defending(&slay, 5, victim(&deathless)), 0),
            Defended::Resisted { saved: false }
        );
        let mortal = Resistances::default();
        assert_eq!(defended(&defending(&slay, 5, victim(&mortal)), 0), Defended::Through(5));
    }

    /// A row with a roll in a realm, not saved against, has its magnitude cut by
    /// resistance: 1000 takes it all (resisted, not saved); 10 at the level
    /// takes 5 to 14 percent; a permanent magnitude is not cut; no resistance
    /// leaves it whole.
    #[test]
    fn resistance_cuts_the_magnitude_and_a_magnitude_cut_to_nothing_is_resisted() {
        let at = |resistance| Resistances {
            realms: [(Realm::Fire, resistance)].into_iter().collect(),
            ..Default::default()
        };
        let harm = EffectSpec { resist: Some(Realm::Fire), ..row(EffectKind::Harm) };
        let strength = Strength { power: 3, against: 10, boost: 0 };
        let against = |harm: &EffectSpec, rolled, resistance: &Resistances, seed| {
            let facts = Defending { strength, ..defending(harm, rolled, victim(resistance)) };
            defended(&facts, seed)
        };
        assert_eq!(against(&harm, 100, &at(1000), 0), Defended::Resisted { saved: false });
        assert_eq!(against(&harm, 100, &at(0), 0), Defended::Through(100));
        for seed in 0..100 {
            let Defended::Through(left) = against(&harm, 100, &at(10), seed) else { panic!() };
            assert!((86..=95).contains(&left), "{left}");
        }
        let forever = EffectSpec { roll: Some(Roll::Fixed(INDEFINITE)), ..harm.clone() };
        assert_eq!(against(&forever, INDEFINITE, &at(1000), 0), Defended::Through(INDEFINITE));
        // A row without a roll is not cut: only rolled magnitudes are.
        let rollless = EffectSpec { roll: None, ..harm };
        assert_eq!(against(&rollless, 100, &at(1000), 0), Defended::Through(100));
    }

    /// A row that allows a save and afflicts a warded condition is resisted
    /// even if no realm gave a save; one that allows no save is not.
    #[test]
    fn a_ward_resists_an_affliction_that_allows_a_save() {
        let warded = Resistances { wards: vec![Condition::Poisoned], ..Default::default() };
        let allows = EffectSpec { save: Some(6), ..row(poison()) };
        assert_eq!(
            defended(&defending(&allows, 5, victim(&warded)), 0),
            Defended::Resisted { saved: false }
        );
        let disallows = row(poison());
        assert_eq!(defended(&defending(&disallows, 5, victim(&warded)), 0), Defended::Through(5));
        let unwarded = Resistances::default();
        assert_eq!(defended(&defending(&allows, 5, victim(&unwarded)), 0), Defended::Through(5));
    }

    fn rounds(left: u32) -> Conditions {
        Conditions::from([(Condition::Poisoned, Lasting::Rounds(left))])
    }

    fn cured(conditions: &Conditions, power: u32, boost: u32, outright: bool) -> Cured {
        let strength = Strength { power, against: 0, boost };
        cure(&mut GameRng::seeded(1), conditions, &[Condition::Poisoned], strength, outright)
    }

    /// Power 2 takes 2 * 2 = 4 rounds off: 20 left 16; 4 or fewer is lifted. A
    /// boost of 50 percent makes it 4 + 4 * 50 / 100 = 6.
    #[test]
    fn a_cure_takes_power_squared_rounds_boosted_off_a_timed_condition() {
        assert_eq!(
            cured(&rounds(20), 2, 0, false),
            Cured { lifted: vec![], shortened: vec![(Condition::Poisoned, 16)] }
        );
        assert_eq!(cured(&rounds(4), 2, 0, false).lifted, [Condition::Poisoned]);
        assert_eq!(cured(&rounds(5), 2, 0, false).shortened, [(Condition::Poisoned, 1)]);
        assert_eq!(cure_rounds(Strength { power: 2, against: 0, boost: 50 }), 6);
        assert_eq!(cured(&rounds(6), 2, 50, false).lifted, [Condition::Poisoned]);
        assert_eq!(cured(&rounds(7), 2, 50, false).shortened, [(Condition::Poisoned, 1)]);
    }
}
