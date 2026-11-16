//! The roll to cast (`docs/rules/casting.md` section 4): how likely a cast is
//! to fail, and what a roll against that becomes. Each a function of the
//! facts it is about and nothing else; the roll takes the random-number
//! generator.

use crate::content::items::UseType;
use crate::content::spells::{SpellDefinition, min_caster_level};
use crate::game::characters::{Profession, Trained};
use crate::game::difficulty::Difficulty;
use crate::game::items::Casts;
use crate::game::magic::{Outcome, best_book, total_caster_level};
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

/// The failure chance of a learned spell at `power` (retail
/// `GetSpellFailureChance`): what the caster's skill, the book once and the
/// realm four times, falls short by; `spell level` more for each caster level
/// they lack for it at this power; raised for an action late in the round;
/// and what the spell adds when cast on oneself.
pub fn learned(
    character: Trained,
    skills: &crate::game::actors::Skills,
    spell: &SpellDefinition,
    power: u32,
    pace: Pace,
    on_oneself: bool,
) -> u32 {
    let book = skills[best_book((character.calling, character.training), spell)];
    let realm = spell.realm.map_or(0, |realm| skills[realm.skill()]);
    let mut chance = spell_failure(casting_skill(book, realm), spell, power);
    let caster_level = total_caster_level(character.calling, spell.books);
    let short = min_caster_level(spell.level) - caster_level - 1 + power as i32;
    if short > 0 {
        chance += spell.level * short as u32;
    }
    let harder = if on_oneself { spell.facts.harder_on_oneself } else { 0 };
    paced(chance, pace) + harder
}

/// The failure chance of an item's spell at `power`, and whether the use is
/// quiet (retail `GetItemSpellFailureChance`): by the skill the item is used
/// with, none for one used with no skill; a thrown item four fifths of it, a
/// charged item or scroll nine tenths; an instrument or gadget adds a third
/// of what the skill lacks of fifteen a spell level, and the spell's level
/// for each level its player lacks as a Bard or Gadgeteer; raised late in the
/// round; thirty more unidentified; a thrown item's use is quiet and halved.
pub fn item(
    character: Trained,
    skills: &crate::game::actors::Skills,
    (casts, identified): (&Casts, bool),
    spell: &SpellDefinition,
    power: u32,
    pace: Pace,
) -> (u32, bool) {
    let Some(skill) = casts.by.skill() else { return (0, false) };
    let level = skills[skill];
    let mut chance = spell_failure(level, spell, power);
    match casts.by {
        UseType::Thrown => chance = chance * 8 / 10,
        UseType::Charged | UseType::Scroll => chance = chance * 9 / 10,
        UseType::Instrument | UseType::Gadget => {
            let need = spell.level * 15;
            if level < need {
                chance += (need - level) / 3;
            }
            let player = if casts.by == UseType::Instrument {
                Profession::Bard
            } else {
                Profession::Gadgeteer
            };
            let levels = i32::from(character.calling.levels[player]);
            let lacks = min_caster_level(spell.level) - levels - 1 + power as i32;
            if lacks > 0 {
                chance += spell.level * lacks as u32;
            }
        }
        _ => {}
    }
    let mut chance = paced(chance, pace);
    if !identified {
        chance += 30;
    }
    let quiet = casts.by == UseType::Thrown;
    if quiet {
        chance /= 2;
    }
    (chance, quiet)
}

/// What a roll to cast against a failure `chance` in percent becomes. Fire
/// under water is `quenched`: it fails every time, half the time if the use
/// is quiet, and never backfires. A failure backfires when the roll is under
/// a third of the chance, the whole of it for a quiet use; one of a spell
/// that cannot backfire, or (not quiet) of a chance under 6, only fizzles.
pub fn outcome(
    rng: &mut GameRng,
    chance: u32,
    quiet: bool,
    quenched: bool,
    can_backfire: bool,
) -> Outcome {
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
    }

    /// Normal's pace is 60. Before it nothing changes; from it the factor is
    /// (moment + 50 - 60) * 2 / 100: at 90, 80 * 231 * 2 / 100 = 369. Novice
    /// (80): at 100, 70 * 231 * 2 / 100 = 323. Expert (40) from 40.
    #[test]
    fn a_chance_grows_for_an_action_late_in_the_round() {
        let at = |difficulty, moment| Pace { difficulty, moment };
        assert_eq!(paced(231, at(Difficulty::Normal, 59)), 231);
        assert_eq!(paced(231, at(Difficulty::Normal, 60)), 231);
        assert_eq!(paced(231, at(Difficulty::Normal, 90)), 369);
        assert_eq!(paced(231, at(Difficulty::Novice, 100)), 323);
        assert_eq!(paced(100, at(Difficulty::Expert, 39)), 100);
        assert_eq!(paced(100, at(Difficulty::Expert, 100)), 220);
    }

    /// A chance past a hundred counts for a third of itself in the chance to
    /// backfire: 300 is 100, so every failure backfires; a spell that cannot
    /// backfire only fizzles.
    #[test]
    fn a_failure_chance_past_a_hundred_always_backfires_unless_the_spell_cannot() {
        let mut rng = GameRng::seeded(2);
        for _ in 0..100 {
            assert_eq!(outcome(&mut rng, 300, false, false, true), Outcome::Backfired);
            assert_eq!(outcome(&mut rng, 100, false, false, false), Outcome::Fizzled);
        }
    }
}
