//! What a cast costs and trains (`docs/rules/casting.md` section 7): the
//! points a learned spell is paid for in, the stamina that tires the caster,
//! the skills practised, and whom a member's cast provokes. Each a function of
//! the facts it is about and nothing else; a rule that rolls takes the
//! random-number generator.

use crate::content::characters::Skill;
use crate::game::characters::Load;
use crate::game::rng::GameRng;

/// Stamina of a thrown item's use.
const THROWN: u32 = 10;

/// Stamina an instrument or gadget costs: so many a spell level, and so many
/// besides.
const PLAYED_PER_LEVEL: u32 = 8;
const PLAYED_BESIDES: u32 = 20;

/// The spell points a cast at `power` takes from its realm, whatever the roll.
pub fn points(cost: u32, power: u32) -> u32 {
    cost * power
}

/// What the cast was, as it tires its caster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toll {
    /// A learned spell, or a monster's own: the spell's level and the power.
    Spell { level: u32, power: u32 },
    /// A thrown item.
    Thrown,
    /// An instrument or gadget, however it was played.
    Played { level: u32 },
    /// Any other item.
    Item { diseased: bool },
}

/// What tires the caster.
#[derive(Clone, Copy, Debug)]
pub struct Tiring {
    pub toll: Toll,
    /// A member's cost is scaled by what they carry; a monster's is taken as
    /// it stands.
    pub load: Option<Load>,
    pub slowed: bool,
    pub hasted: bool,
}

/// The stamina a cast costs the caster (retail `SpellCastFatigueCost`,
/// `CharacterActionFatigueCost`, `FatigueCharacter`, `FatigueMonster`): the
/// toll of the cast, then, for a member, `cost + load% * cost / 100`, less 25
/// if slowed, otherwise plus 25 if hasted.
pub fn stamina(rng: &mut GameRng, facts: &Tiring) -> u32 {
    let toll = match facts.toll {
        Toll::Spell { level, power } => level + power,
        Toll::Thrown => THROWN,
        Toll::Played { level } => level * PLAYED_PER_LEVEL + PLAYED_BESIDES,
        Toll::Item { diseased } => {
            let base = rng.below(2) + 1;
            if diseased { base * 2 } else { base }
        }
    };
    match facts.load {
        Some(load) => load.scale_fatigue(toll, facts.slowed, facts.hasted),
        None => toll,
    }
}

/// What a cast trains, if it affected someone (retail `Magic.cpp:2682-2692`,
/// `PC Item.cpp:3792-3796`).
#[derive(Clone, Copy, Debug)]
pub enum Training {
    /// A learned spell: the book skill it was cast by, its realm's skill, and
    /// Power Cast if it is available to the caster.
    Learned { level: u32, power: u32, book: Skill, realm: Option<Skill>, power_cast: bool },
    /// An item's spell: the skill that presents it.
    Item { level: u32, power: u32, skill: Option<Skill> },
    /// An instrument or gadget that could not be played: the skill, once.
    PlayedBadly { skill: Option<Skill> },
}

/// The skills practised and how many uses of each. A learned spell trains its
/// book and Power Cast by `(level + power + 2) / 4` and its realm by
/// `level + power`; an item trains its skill by `level + power / 2`; a bad
/// performance trains its skill once.
pub fn practice(facts: &Training) -> Vec<(Skill, u32)> {
    match *facts {
        Training::Learned { level, power, book, realm, power_cast } => {
            let index = level + power;
            let mut skills = vec![(book, (index + 2) >> 2)];
            if let Some(realm) = realm {
                skills.push((realm, index));
            }
            if power_cast {
                skills.push((Skill::PowerCast, (index + 2) >> 2));
            }
            skills
        }
        Training::Item { level, power, skill } => {
            skill.map(|skill| vec![(skill, level + power / 2)]).unwrap_or_default()
        }
        Training::PlayedBadly { skill } => skill.map(|skill| vec![(skill, 1)]).unwrap_or_default(),
    }
}

/// Whether a cast turns the groups of its victims hostile as it starts, before
/// the roll and whatever the roll (retail `MakeTargetGroupHostile`): a
/// member's cast aimed at enemies does; one tried again after a fizzle does
/// not provoke anew.
pub fn provokes(member: bool, aimed_at_enemies: bool, retry: bool) -> bool {
    member && aimed_at_enemies && !retry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_learned_spell_takes_its_cost_a_power_whatever_the_roll() {
        assert_eq!(points(3, 4), 12);
        assert_eq!(points(5, 1), 5);
        assert_eq!(points(0, 7), 0);
    }

    fn tires(toll: Toll, load: Option<Load>) -> Tiring {
        Tiring { toll, load, slowed: false, hasted: false }
    }

    fn spent(facts: &Tiring, seed: u64) -> u32 {
        stamina(&mut GameRng::seeded(seed), facts)
    }

    /// Level 3 and power 4: 7 for a monster as it stands; for a member who
    /// carries nothing, 7 + 0 * 7 / 100; light 25 percent: 7 + 25 * 7 / 100 = 8;
    /// medium 50: 7 + 3 = 10; heavy 100: 14; extreme 200: 21.
    #[test]
    fn a_spell_tires_by_its_level_and_power_and_a_member_by_what_they_carry_too() {
        let spell = Toll::Spell { level: 3, power: 4 };
        assert_eq!(spent(&tires(spell, None), 0), 7);
        let by = |load| spent(&tires(spell, Some(load)), 0);
        assert_eq!(
            [by(Load::None), by(Load::Light), by(Load::Medium), by(Load::Heavy), by(Load::Extreme)],
            [7, 8, 10, 14, 21]
        );
    }

    /// A slowed member pays 25 less percent: heavy 100 - 25 = 75: 7 + 75 * 7 / 100
    /// = 7 + 5 = 12; hasted, 25 more: none 25: 7 + 25 * 7 / 100 = 8. Slowed wins
    /// over hasted.
    #[test]
    fn a_member_slowed_pays_less_and_hasted_more() {
        let spell = Toll::Spell { level: 3, power: 4 };
        let slowed = Tiring { slowed: true, ..tires(spell, Some(Load::Heavy)) };
        assert_eq!(spent(&slowed, 0), 12);
        let hasted = Tiring { hasted: true, ..tires(spell, Some(Load::None)) };
        assert_eq!(spent(&hasted, 0), 8);
        let both = Tiring { slowed: true, hasted: true, ..tires(spell, Some(Load::None)) };
        assert_eq!(spent(&both, 0), 7 - 25 * 7 / 100);
    }

    #[test]
    fn a_thrown_item_costs_ten_and_an_instrument_eight_a_level_and_twenty() {
        assert_eq!(spent(&tires(Toll::Thrown, Some(Load::None)), 0), 10);
        // Level 3: 8 * 3 + 20 = 44; heavy doubles it.
        let played = Toll::Played { level: 3 };
        assert_eq!(spent(&tires(played, Some(Load::None)), 0), 44);
        assert_eq!(spent(&tires(played, Some(Load::Heavy)), 0), 88);
    }

    /// Any other item costs below(2) + 1, so 1 or 2, and twice that diseased:
    /// 2 or 4.
    #[test]
    fn any_other_item_costs_one_or_two_and_twice_that_when_diseased() {
        let well = tires(Toll::Item { diseased: false }, Some(Load::None));
        let ill = tires(Toll::Item { diseased: true }, Some(Load::None));
        let well: Vec<u32> = (0..40).map(|seed| spent(&well, seed)).collect();
        let ill: Vec<u32> = (0..40).map(|seed| spent(&ill, seed)).collect();
        assert!(well.iter().all(|cost| (1..=2).contains(cost)));
        assert!(well.contains(&1) && well.contains(&2));
        assert!(ill.iter().all(|cost| [2, 4].contains(cost)));
        assert!(ill.contains(&2) && ill.contains(&4));
    }

    /// Level 3 at power 4: index 7; the book and Power Cast (7 + 2) >> 2 = 2
    /// uses, the realm 7.
    #[test]
    fn a_learned_spell_trains_its_book_its_realm_and_power_cast() {
        let learned = |power_cast| Training::Learned {
            level: 3,
            power: 4,
            book: Skill::Divinity,
            realm: Some(Skill::DivineMagic),
            power_cast,
        };
        assert_eq!(
            practice(&learned(true)),
            [(Skill::Divinity, 2), (Skill::DivineMagic, 7), (Skill::PowerCast, 2)]
        );
        assert_eq!(practice(&learned(false)), [(Skill::Divinity, 2), (Skill::DivineMagic, 7)]);
        // A spell with no realm trains no realm skill; level 1 power 1: (2 + 2) >> 2 = 1.
        let no_realm = Training::Learned {
            level: 1,
            power: 1,
            book: Skill::Wizardry,
            realm: None,
            power_cast: false,
        };
        assert_eq!(practice(&no_realm), [(Skill::Wizardry, 1)]);
    }

    /// An item trains the skill that presents it by level + power / 2: 4 + 5 / 2 = 6.
    #[test]
    fn an_item_trains_its_skill_by_the_spell_level_and_half_the_power() {
        let item = |skill| Training::Item { level: 4, power: 5, skill };
        assert_eq!(practice(&item(Some(Skill::Artifacts))), [(Skill::Artifacts, 6)]);
        assert!(practice(&item(None)).is_empty());
    }

    #[test]
    fn an_instrument_played_badly_trains_its_skill_once() {
        assert_eq!(
            practice(&Training::PlayedBadly { skill: Some(Skill::Music) }),
            [(Skill::Music, 1)]
        );
        assert!(practice(&Training::PlayedBadly { skill: None }).is_empty());
    }

    #[test]
    fn only_a_members_cast_at_enemies_provokes_and_not_a_retry() {
        assert!(provokes(true, true, false));
        assert!(!provokes(false, true, false), "a monster provokes no one");
        assert!(!provokes(true, false, false), "a cast not aimed at enemies");
        assert!(!provokes(true, true, true), "tried again after a fizzle");
    }
}
