//! What a cast costs and trains (`docs/rules/casting.md` section 7): the
//! stamina that tires the caster and the skills practised. A learned spell is
//! paid for in points, its cost a power; an item with one use.

use crate::content::characters::{Realm, Skill};
use crate::content::items::UseType;
use crate::content::spells::SpellDefinition;
use crate::game::characters::{Calling, Training};
use crate::game::magic::best_book;
use crate::game::rng::GameRng;

/// The stamina throwing an item costs.
const THROWN: u32 = 10;

/// The stamina playing an instrument or gadget costs: so much a spell level,
/// and this besides.
const PLAYED_PER_LEVEL: u32 = 8;
const PLAYED_BESIDES: u32 = 20;

/// The stamina a cast tires its caster by (retail `FatigueCharacter`): a
/// spell cast by its level and power; an item thrown 10; an instrument or
/// gadget played 8 a spell level and 20; any other item 1 or 2, twice that
/// `diseased`. A member scales it by what they carry; a monster's stands.
pub fn stamina(
    rng: &mut GameRng,
    used: Option<UseType>,
    spell: &SpellDefinition,
    power: u32,
    diseased: bool,
) -> u32 {
    match used {
        None => spell.level + power,
        Some(UseType::Thrown) => THROWN,
        Some(UseType::Instrument | UseType::Gadget) => {
            spell.level * PLAYED_PER_LEVEL + PLAYED_BESIDES
        }
        Some(_) => (rng.below(2) + 1) * if diseased { 2 } else { 1 },
    }
}

/// The skills a cast that affected someone practises, and how often (retail
/// `PracticeSpellcasting`). A learned spell: its book and Power Cast by
/// `(level + power + 2) / 4` (Power Cast if they may use it), its realm by
/// `level + power`. An item: the
/// skill it is used with by `level + power / 2`.
pub fn practice(
    (calling, training, power_cast): (&Calling, &Training, bool),
    used: Option<UseType>,
    spell: &SpellDefinition,
    power: u32,
) -> Vec<(Skill, u32)> {
    let Some(used) = used else {
        let index = spell.level + power;
        let mut skills = vec![(best_book((calling, training), spell), (index + 2) >> 2)];
        skills.extend(spell.realm.map(|realm| (Realm::skill(realm), index)));
        if power_cast {
            skills.push((Skill::PowerCast, (index + 2) >> 2));
        }
        return skills;
    };
    used.skill().map(|skill| vec![(skill, spell.level + power / 2)]).unwrap_or_default()
}
