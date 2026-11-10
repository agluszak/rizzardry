//! The roll to cast (`docs/rules/casting.md` section 4): how likely a cast is
//! to fail, and what a roll against that becomes. Each a function of the
//! facts it is about and nothing else; the roll takes the random-number
//! generator.

use crate::content::spells::{SpellDefinition, min_caster_level};
use crate::game::difficulty::Difficulty;
use crate::game::magic::Outcome;
use crate::game::rng::GameRng;

/// The skill a spell's difficulty band needs at power 7 (retail
/// `g_combat_effect_slot_spells_and_cast_success`), for bands 0 to 16.
const THRESHOLDS: [u32; 17] =
    [30, 40, 50, 58, 64, 70, 76, 81, 86, 90, 94, 97, 100, 102, 105, 107, 110];

/// The most a skill short of its need fails by, in percent.
const SHORTFALL_FAILS: u32 = 70;

/// A failure chance under which a failure cannot backfire, in percent.
const TOO_SURE_TO_BACKFIRE: u32 = 6;

/// How late in a round an action may come before its failure chance grows,
/// by setting (retail `ScaleByCombatPace`).
fn pace(difficulty: Difficulty) -> u32 {
    match difficulty {
        Difficulty::Novice => 80,
        Difficulty::Normal => 60,
        Difficulty::Expert => 40,
    }
}

/// What a spell's caster is judged by: the book skill counts once, the realm
/// skill four times (retail `GetSpellCastingSkillLevel`).
pub fn casting_skill(book: u32, realm: u32) -> u32 {
    (book + realm * 4) / 5
}

/// Retail `GetSpellFailureChance`: the skill a spell's difficulty band needs
/// at `power`, and up to 70 percent failure for falling short of it.
pub fn spell_failure(skill: u32, spell: &SpellDefinition, power: u32) -> u32 {
    let band = (spell.cost / 2 + spell.level).min(16) as usize;
    let need = THRESHOLDS[band] * power / 7;
    if skill < need { (need - skill) * SHORTFALL_FAILS / need } else { 0 }
}

/// How late the caster's moment is in the round, and how that weighs.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub difficulty: Difficulty,
    /// The caster's moment in the round, 1 to 100; 0 outside a fight.
    pub moment: u32,
}

/// A failure chance raised for an action made late in a round: from the
/// moment of the setting's pace on, `(50 - pace + moment) * chance * 2 / 100`.
pub fn paced(chance: u32, on: Pace) -> u32 {
    let pace = pace(on.difficulty);
    if pace > on.moment { chance } else { (on.moment + 50 - pace) * chance * 2 / 100 }
}

/// A learned spell's chance to fail (retail `ExecuteCharacterSpellCast`).
#[derive(Clone, Copy, Debug)]
pub struct Learned<'a> {
    pub spell: &'a SpellDefinition,
    pub power: u32,
    /// The skill in the book it is cast by, and in its realm.
    pub book: u32,
    pub realm: u32,
    /// The caster's total caster level for the spell.
    pub caster_level: i32,
    pub pace: Pace,
    /// A member casts it on themselves.
    pub on_oneself: bool,
}

/// The percent chance a learned cast fails: the casting skill's shortfall, and
/// the caster levels the spell's level and power call for, scaled by the
/// round's pace, and a spell harder on oneself gaining its figure after that.
/// It is not held to 100: a chance past it still counts for a third of itself
/// in the chance to backfire.
pub fn learned(facts: &Learned) -> u32 {
    let Learned { spell, power, book, realm, caster_level, pace, on_oneself } = *facts;
    let mut chance = spell_failure(casting_skill(book, realm), spell, power);
    let short = min_caster_level(spell.level) - caster_level - 1 + power as i32;
    if short > 0 {
        chance += spell.level * short as u32;
    }
    let harder = if on_oneself { spell.facts.harder_on_oneself } else { 0 };
    paced(chance, pace) + harder
}

/// How an item presents its spell, as its chance to fail weighs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presented {
    Thrown,
    Charged,
    Scroll,
    Instrument,
    Gadget,
    /// Anything else: potions, food, gear.
    Other,
}

/// An item's spell to be cast (retail `GetItemUseDifficulty`).
#[derive(Clone, Copy, Debug)]
pub struct Item<'a> {
    pub spell: &'a SpellDefinition,
    pub power: u32,
    pub presented: Presented,
    /// The level of the skill that presents it; none for an item with no such
    /// skill, which never fails.
    pub skill: Option<u32>,
    /// The levels in the profession that plays it (Bard or Gadgeteer).
    pub player_levels: u32,
    pub identified: bool,
    pub pace: Pace,
}

/// The percent chance an item's spell fails to be cast, and whether the use is
/// quiet, every failure of which can backfire (retail `CastItemSpell`):
/// the spell's chance at the skill's level, four fifths of it for a thrown
/// item and nine tenths for a charged one or a scroll, an instrument or gadget
/// gaining a third of its shortfall from fifteen a spell level and the spell
/// level a time what the player lacks of the caster level, then the round's
/// pace, thirty more if unidentified, and half of it for a quiet use.
pub fn item(facts: &Item) -> (u32, bool) {
    let Item { spell, power, presented, skill, player_levels, identified, pace } = *facts;
    let Some(level) = skill else { return (0, false) };
    let mut chance = spell_failure(level, spell, power);
    match presented {
        Presented::Thrown => chance = chance * 8 / 10,
        Presented::Charged | Presented::Scroll => chance = chance * 9 / 10,
        Presented::Instrument | Presented::Gadget => {
            let need = spell.level * 15;
            if level < need {
                chance += (need - level) / 3;
            }
            let lacks = min_caster_level(spell.level) - player_levels as i32 - 1 + power as i32;
            if lacks > 0 {
                chance += spell.level * lacks as u32;
            }
        }
        Presented::Other => {}
    }
    let mut chance = paced(chance, pace);
    if !identified {
        chance += 30;
    }
    let quiet = presented == Presented::Thrown;
    if quiet {
        chance /= 2;
    }
    (chance, quiet)
}

/// What a roll weighs (retail `CastSpellFromSource`).
#[derive(Clone, Copy, Debug)]
pub struct Rolling {
    pub chance: u32,
    /// A thrown item's use.
    pub quiet: bool,
    /// A Fire spell with the party under water.
    pub quenched: bool,
    pub can_backfire: bool,
}

/// What a roll becomes: with no chance the cast comes off; else it fails if a
/// roll below 100 is under the chance, and backfires if the spell can and the
/// roll is under a third of the chance (all of it for a quiet use, none under
/// 6 percent). A Fire spell under water has chance 100 (50 for a quiet use)
/// and never backfires.
pub fn outcome(rng: &mut GameRng, facts: &Rolling) -> Outcome {
    let Rolling { chance, quiet, quenched, can_backfire } = *facts;
    let chance = if quenched { if quiet { 50 } else { 100 } } else { chance };
    if chance == 0 {
        return Outcome::Cast;
    }
    let rolled = rng.below(100);
    if rolled >= chance {
        return Outcome::Cast;
    }
    let backfire = if quenched || !can_backfire || (!quiet && chance < TOO_SURE_TO_BACKFIRE) {
        0
    } else if quiet {
        chance
    } else {
        chance / 3
    };
    if rolled < backfire { Outcome::Backfired } else { Outcome::Fizzled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Realm;
    use crate::content::spells::SpellId;

    fn spell(cost: u32, level: u32) -> SpellDefinition {
        SpellDefinition {
            id: SpellId::HEAL_ALL,
            realm: Some(Realm::Divine),
            cost,
            level,
            ..Default::default()
        }
    }

    const NORMAL: Pace = Pace { difficulty: Difficulty::Normal, moment: 0 };

    /// Cost 6, level 4: band 6/2 + 4 = 7, threshold 81. At power 7 it needs
    /// 81 * 7 / 7 = 81; a skill of 40 fails (81 - 40) * 70 / 81 = 2870 / 81 = 35;
    /// at power 4 it needs 81 * 4 / 7 = 46: (46 - 40) * 70 / 46 = 9.
    #[test]
    fn a_skill_short_of_what_the_spell_needs_fails_up_to_seventy_percent() {
        let spell = spell(6, 4);
        assert_eq!(spell_failure(40, &spell, 7), 35);
        assert_eq!(spell_failure(40, &spell, 4), 9);
        // No skill at all fails by 70; the need met or passed does not fail.
        assert_eq!(spell_failure(0, &spell, 7), 70);
        assert_eq!(spell_failure(81, &spell, 7), 0);
        assert_eq!(spell_failure(200, &spell, 7), 0);
    }

    /// Normal's pace is 60. Before it nothing changes; from it the factor is
    /// (moment + 50 - 60) * 2 / 100: at 60, 50 * 2 = 100 -> 1 times; at 90,
    /// 80 * 231 * 2 / 100 = 369. Novice (80): at 100, 70 * 231 * 2 / 100 = 323.
    /// Expert (40): at 40, (40 + 50 - 40) * 100 * 2 / 100 = 100; at 39 unchanged.
    #[test]
    fn a_chance_grows_for_an_action_late_in_the_round() {
        let at = |difficulty, moment| Pace { difficulty, moment };
        assert_eq!(paced(231, at(Difficulty::Normal, 0)), 231);
        assert_eq!(paced(231, at(Difficulty::Normal, 59)), 231);
        assert_eq!(paced(231, at(Difficulty::Normal, 60)), 231);
        assert_eq!(paced(231, at(Difficulty::Normal, 90)), 369);
        assert_eq!(paced(231, at(Difficulty::Novice, 100)), 323);
        assert_eq!(paced(100, at(Difficulty::Expert, 39)), 100);
        assert_eq!(paced(100, at(Difficulty::Expert, 40)), 100);
        assert_eq!(paced(100, at(Difficulty::Expert, 100)), 220);
    }

    fn untrained<'a>(spell: &'a SpellDefinition, power: u32, pace: Pace) -> Learned<'a> {
        Learned { spell, power, book: 0, realm: 0, caster_level: 1, pace, on_oneself: false }
    }

    /// Cost 14, level 7: band 14 asks 105 at power 7: 70 percent for a skill of
    /// 0. A level 7 spell asks caster level 18; one of 1 is 18 - 1 - 1 + 7 = 23
    /// short, 7 * 23 = 161 more. 231 in all, not held to 100.
    #[test]
    fn a_failure_chance_adds_the_caster_levels_it_lacks_and_is_not_held_to_a_hundred() {
        let big = spell(14, 7);
        assert_eq!(learned(&untrained(&big, 7, NORMAL)), 231);
        // At power 1: need 105 / 7 = 15, 70; short 18 - 1 - 1 + 1 = 17, 7 * 17 = 119.
        assert_eq!(learned(&untrained(&big, 1, NORMAL)), 189);
        // A caster of level 25 lacks 18 - 25 - 1 + 7 = -1: nothing; only the skill.
        let high = Learned { caster_level: 25, ..untrained(&big, 7, NORMAL) };
        assert_eq!(learned(&high), 70);
        // Exactly nothing short (caster 18 at power 1 is 18 - 18 - 1 + 1 = 0) adds nothing.
        let zero = Learned { caster_level: 18, ..untrained(&big, 1, NORMAL) };
        assert_eq!(learned(&zero), 70);
    }

    fn charged(spell: &SpellDefinition, presented: Presented, pace: Pace) -> Item<'_> {
        Item {
            spell,
            power: 7,
            presented,
            skill: Some(0),
            player_levels: 0,
            identified: true,
            pace,
        }
    }

    /// 63 for a charged item at moment 100 on Normal: (100 + 50 - 60) * 63 * 2 /
    /// 100 = 113; thirty more unidentified (143). A thrown item unidentified
    /// halves the thirty with the rest: (56 -> 100: 90 * 56 * 2 / 100 = 100, + 30)
    /// / 2 = 65.
    #[test]
    fn an_items_chance_grows_late_in_a_round_and_when_unidentified() {
        let spell = spell(14, 7);
        let late = Pace { difficulty: Difficulty::Normal, moment: 100 };
        assert_eq!(item(&charged(&spell, Presented::Charged, late)).0, 113);
        let unknown = Item { identified: false, ..charged(&spell, Presented::Charged, late) };
        assert_eq!(item(&unknown).0, 143);
        let thrown = Item { identified: false, ..charged(&spell, Presented::Thrown, late) };
        assert_eq!(item(&thrown), (65, true));
    }

    /// An instrument at skill 30 for a level 7 spell: 70 - 0... the skill 30 of
    /// 105 needed fails (105 - 30) * 70 / 105 = 50; then (15 * 7 - 30) / 3 =
    /// 25 for the shortfall; a Bard of 10 lacks 18 - 10 - 1 + 7 = 14, 7 * 14 =
    /// 98: 50 + 25 + 98 = 173. A Bard of 25 lacks none (18 - 25 - 1 + 7 = -1):
    /// 75. A skill of 105 or more adds no shortfall: skill 105 fails 0 + 0.
    #[test]
    fn an_instrument_adds_its_shortfall_and_what_its_player_lacks() {
        let spell = spell(14, 7);
        let play = |skill, levels| {
            item(&Item {
                skill: Some(skill),
                player_levels: levels,
                ..charged(&spell, Presented::Instrument, NORMAL)
            })
        };
        assert_eq!(play(30, 10), (173, false));
        assert_eq!(play(30, 25), (75, false));
        assert_eq!(play(105, 25), (0, false));
        // A gadget is played the same way.
        let gadget = Item { skill: Some(30), ..charged(&spell, Presented::Gadget, NORMAL) };
        assert_eq!(item(&gadget), (50 + 25 + 7 * (18 - 1 + 7), false));
    }

    fn rolled(rng: &mut GameRng, facts: Rolling) -> Outcome {
        outcome(rng, &facts)
    }

    fn rolling(chance: u32) -> Rolling {
        Rolling { chance, quiet: false, quenched: false, can_backfire: true }
    }

    /// A chance past a hundred counts for a third of itself in the chance to
    /// backfire: 300 is 100, so every failure backfires (held to 100 first, it
    /// was 33).
    #[test]
    fn a_failure_chance_past_a_hundred_always_backfires() {
        let mut rng = GameRng::seeded(2);
        for _ in 0..100 {
            assert_eq!(rolled(&mut rng, rolling(300)), Outcome::Backfired);
        }
    }

    #[test]
    fn a_spell_that_cannot_backfire_only_fizzles() {
        let mut rng = GameRng::seeded(1);
        for _ in 0..200 {
            let facts = Rolling { can_backfire: false, ..rolling(100) };
            assert_eq!(rolled(&mut rng, facts), Outcome::Fizzled);
        }
    }
}
