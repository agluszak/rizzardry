//! What a utility spell does once it comes off (`docs/rules/casting.md`
//! section 8): the arithmetic of charming and reading a mind, of a lure's
//! hold, of an elemental summoned, of Knock Knock and a disarming, of a
//! sprung trap.

use bevy::prelude::Entity;

use crate::content::characters::{Attribute, Realm};
use crate::content::species::MonsterKind;
use crate::game::difficulty::Difficulty;
use crate::game::hits::defences;
use crate::game::hits::effects::BodyReadOnlyItem;
use crate::game::magic::{Outcome, Strength};
use crate::game::rng::{GameRng, weighted};

/// The most a lock or trap spell's level may be.
const HIGHEST_LEVEL: u32 = 7;

/// What a charm amounts to for each power, before the boost.
const CHARM_PER_POWER: u32 = 7;

/// The most a mind that has refused is counted to have refused.
const MOST_REFUSALS: u8 = 2;

/// The speaker's answer to a backfire that was noticed, to a mind read that
/// was saved against, and the first of the three refusals of one that was not
/// (retail dialogue strings).
const QUOTE_NOTICED: u16 = 0x16;
const QUOTE_SAVED: u16 = 0x69;
const QUOTE_FIRST_REFUSAL: u16 = 0x67;

/// What a speaker says in answer to Charm or Mindread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Nothing the speaker shows.
    Unnoticed,
    /// A line of their script.
    Quote(u16),
    /// Won over by this much.
    Charmed(u32),
}

/// What became of a Charm or a Mindread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spoken {
    pub reply: Reply,
    /// They saved against a charm.
    pub resisted: bool,
    /// How often they have now refused to have their mind read.
    pub refusals: u8,
}

/// Charm or Mindread cast at the one the party is talking with (retail
/// `TargetResistsCondition`, `ApplyCharmEffect`). A fizzle shows nothing; a
/// backfire is noticed with a chance of half their Senses and Intelligence.
/// A cast that comes off is saved against in the Mental realm, by their
/// level against its strength, scaled for difficulty by their side. A mind
/// read saved against answers with one line, and otherwise with the next of
/// their refusals, which stop at two. A charm not saved against wins them
/// over by seven a power, raised by the boost and cut by their resistance,
/// if that is more than they have been `charmed` by already.
pub fn speak(
    rng: &mut GameRng,
    (mindread, outcome): (bool, Outcome),
    strength: Strength,
    difficulty: Difficulty,
    their: &BodyReadOnlyItem,
    (charmed, refusals): (u32, u8),
) -> Spoken {
    let mut spoken = Spoken { reply: Reply::Unnoticed, resisted: false, refusals };
    let (level, resistance) = (their.level.0, their.resistances.realms[Realm::Mental]);
    match outcome {
        Outcome::Fizzled => {}
        Outcome::Backfired => {
            let wits =
                their.attributes[Attribute::Senses] + their.attributes[Attribute::Intelligence];
            if rng.below(100) < wits / 2 {
                spoken.reply = Reply::Quote(QUOTE_NOTICED);
            }
        }
        Outcome::Cast => {
            let chance = defences::condition_chance(level, resistance, strength.against, 0);
            let saved = defences::save_roll(rng, difficulty.scale(chance, their.stance()));
            if mindread {
                spoken.reply = Reply::Quote(if saved {
                    QUOTE_SAVED
                } else {
                    spoken.refusals = (refusals + 1).min(MOST_REFUSALS);
                    QUOTE_FIRST_REFUSAL + u16::from(refusals)
                });
            } else if saved {
                spoken.resisted = true;
            } else {
                let amount = strength.power * CHARM_PER_POWER;
                let amount = defences::resist(
                    rng,
                    amount + amount * strength.boost / 100,
                    resistance.min(100),
                    level,
                    strength.against,
                );
                if amount > charmed {
                    spoken.reply = Reply::Charmed(amount);
                }
            }
        }
    }
    spoken
}

/// How a monster answers a lure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Response {
    Controlled,
    #[default]
    Resisted,
}

/// How a monster a lure puts to the test answers it (retail
/// `MonsterResistsSpellEffect`): an undead one always resists; the rest save
/// in the Mental realm, their level against the lure's strength, unscaled by
/// difficulty.
pub fn lure_response(rng: &mut GameRng, monster: &BodyReadOnlyItem, against: u32) -> Response {
    let resistance = monster.resistances.realms[Realm::Mental];
    let chance = defences::condition_chance(monster.level.0, resistance, against, 0);
    if monster.resistances.kind == Some(MonsterKind::Undead) || defences::save_roll(rng, chance) {
        Response::Resisted
    } else {
        Response::Controlled
    }
}

/// How near a controlled monster outside a fight comes to its lure, in metres.
pub const LURE_HELD_AT: f32 = 5.0;

/// When a lure ends: its turns of aging from now, in milliseconds.
pub fn lure_ends(now: u64, turns: u32, tick_ms: u64) -> u64 {
    now + u64::from(turns) * tick_ms
}

/// Species numbers: the elementals Summon Elemental may call, four elements to
/// a tier (retail `g_elemental_summon_group_ids`).
const FIRST_ELEMENTAL: u32 = 21;
const ELEMENTS: u32 = 4;
/// Where each element stands among its tier's four: air, earth, water, fire.
const ELEMENT_OF_TIER: [u32; 4] = [3, 2, 0, 1];
/// What each element's realm skill weighs besides its level.
const SKILL_PLUS: u32 = 20;

/// The elementals Summon Elemental may call.
pub const ELEMENTALS: std::ops::RangeInclusive<u32> = FIRST_ELEMENTAL..=48;

/// The elemental called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summoned {
    /// Its element, as an index of Fire, Water, Air, Earth.
    pub element: usize,
    /// Its species.
    pub species: u32,
    /// It is hostile to the party's side: the summoning backfired.
    pub backfired: bool,
    /// Power Cast lays the element's buff on it as well.
    pub buffed: bool,
}

/// Its element chosen by weight from `skill + 20` each (Fire is 0 under
/// water), its tier by the power (one to seven), friendly unless the summoning
/// backfired; with chance equal to the boost, buffed.
pub fn summon(
    rng: &mut GameRng,
    skills: Option<[u32; 4]>,
    under_water: bool,
    outcome: Outcome,
    strength: Strength,
) -> Summoned {
    let mut weights = skills.map_or([SKILL_PLUS; 4], |skills| skills.map(|s| s + SKILL_PLUS));
    if under_water {
        weights[0] = 0;
    }
    let element = weighted(rng.below(weights.iter().sum()), weights).unwrap_or(3);
    let tier = strength.power.clamp(1, 7) - 1;
    let buffed = strength.boost != 0 && rng.below(100) < strength.boost;
    Summoned {
        element,
        species: FIRST_ELEMENTAL + tier * ELEMENTS + ELEMENT_OF_TIER[element],
        backfired: outcome == Outcome::Backfired,
        buffed,
    }
}

/// The element an index of Fire, Water, Air, Earth is.
pub fn element(index: usize) -> Realm {
    [Realm::Fire, Realm::Water, Realm::Air, Realm::Earth][index]
}

/// The level of a lock or trap spell: the power raised by the boost, held at
/// seven.
pub fn level(strength: Strength) -> u32 {
    (strength.power + strength.power * strength.boost / 100).min(HIGHEST_LEVEL)
}

/// Per mille that Knock Knock moves each tumbler it reaches, by level.
const KNOCK_CHANCE: [u32; 8] = [0, 344, 459, 516, 550, 573, 589, 602];

/// A tumbler Knock Knock reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reached {
    /// Which of the lock's tumblers.
    pub tumbler: usize,
    /// It moves: it is held up by the spell, or dropped by a backfire.
    pub moves: bool,
}

/// The tumblers of a lock of `tumblers` that Knock Knock at `level` reaches:
/// `level + 1` of them chosen at random, each moving with a chance in per
/// mille by level.
pub fn knock(rng: &mut GameRng, tumblers: usize, level: u32) -> Vec<Reached> {
    let mut order: Vec<usize> = (0..tumblers).collect();
    for _ in 0..order.len() {
        let a = rng.below(order.len() as u32) as usize;
        let b = rng.below(order.len() as u32) as usize;
        order.swap(a, b);
    }
    order
        .into_iter()
        .take(level as usize + 1)
        .map(|tumbler| Reached { tumbler, moves: rng.below(1000) < KNOCK_CHANCE[level as usize] })
        .collect()
}

/// Whether a spell at `level` disarms a trap of `difficulty`: a roll below 100
/// plus six a point of difficulty is under five a level and fifty. A backfire
/// never does.
pub fn disarms(rng: &mut GameRng, level: u32, difficulty: u8, outcome: Outcome) -> bool {
    let roll = rng.below(100) + u32::from(difficulty) * 6;
    outcome != Outcome::Backfired && roll < level * 5 + 50
}

/// What a trap's spell does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprung {
    /// The trap is destroyed, with chance 1 in 2.
    pub destroyed: bool,
    pub power: u32,
    pub victims: Vec<Entity>,
}

/// The power of a sprung trap's spell is `min(7, extra + 4)`, `extra` being how
/// far the trap's difficulty (held to 1 to 7) is over its type's grade. It
/// reaches `difficulty - below(difficulty / 2)` living members, and
/// `below(extra - 3)` more if `extra` is over 3; if more are alive, members
/// are removed at random from the party's order, never the one who set it off.
pub fn spring(
    rng: &mut GameRng,
    (difficulty, grade): (u8, u32),
    members: &[Entity],
    selected: Option<Entity>,
) -> Sprung {
    let difficulty = u32::from(difficulty.clamp(1, 7));
    let destroyed = rng.below(2) == 0;
    let mut count = difficulty - rng.below(difficulty / 2);
    let extra = difficulty.max(grade) - grade;
    let power = (extra + 4).min(HIGHEST_LEVEL);
    if extra > 3 {
        count += rng.below(extra - 3);
    }
    let mut victims = members.to_vec();
    while victims.len() > count as usize {
        let index = rng.below(victims.len() as u32) as usize;
        if Some(victims[index]) != selected {
            victims.remove(index);
        }
    }
    Sprung { destroyed, power, victims }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The power raised by the boost, held at seven: 4 + 4 * 50 / 100 = 6; 5 + 5 = 10
    /// is held to 7; no boost leaves the power.
    #[test]
    fn a_lock_or_trap_spells_level_is_the_power_boosted_and_held_at_seven() {
        let at = |power, boost| level(Strength { power, against: 0, boost });
        assert_eq!((at(4, 50), at(5, 100), at(3, 0), at(7, 0)), (6, 7, 3, 7));
    }
}
