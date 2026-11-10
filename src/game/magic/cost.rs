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

    fn tires(toll: Toll, load: Option<Load>) -> Tiring {
        Tiring { toll, load, slowed: false, hasted: false }
    }

    fn spent(facts: &Tiring, seed: u64) -> u32 {
        stamina(&mut GameRng::seeded(seed), facts)
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
}
