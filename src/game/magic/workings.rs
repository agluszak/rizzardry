//! What a utility spell does once it comes off (`docs/rules/casting.md`
//! section 8): the arithmetic of charming and reading a mind, of a lure's
//! hold, of an elemental summoned, of Knock Knock and a disarming, of a
//! sprung trap. Each a function of the facts it is about and nothing else; a
//! rule that rolls takes the random-number generator.

use bevy::prelude::Entity;

use crate::content::characters::Realm;
use crate::content::species::MonsterKind;
use crate::game::difficulty::Difficulty;
use crate::game::hits::defences;
use crate::game::magic::{Outcome, Strength};
use crate::game::rng::{GameRng, weighted};
use crate::game::world::hostility::Stance;

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

/// The one the party is talking with, and what is cast at them (retail
/// `TargetResistsCondition`, `ApplyCharmEffect`).
#[derive(Clone, Copy, Debug)]
pub struct Speaking {
    pub mindread: bool,
    pub outcome: Outcome,
    pub strength: Strength,
    pub difficulty: Difficulty,
    pub stance: Stance,
    pub level: u32,
    /// Their resistance in the Mental realm.
    pub resistance: i32,
    /// Their Senses and Intelligence, by which a backfire is noticed.
    pub senses: u32,
    pub intelligence: u32,
    /// What they have already been charmed by.
    pub charmed: u32,
    /// The times they have refused a mind read already.
    pub refusals: u8,
}

/// What came of speaking to them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spoken {
    pub reply: Reply,
    /// They saved against Charm, which is told as resisted.
    pub resisted: bool,
    /// The times they have refused, now.
    pub refusals: u8,
}

/// Whether the one spoken to saves: the mental save of their level against the
/// cast's strength, scaled for difficulty by their side like any save, held
/// within 5 and 95.
pub fn saves(rng: &mut GameRng, speaking: &Speaking) -> bool {
    let chance = defences::condition_chance(
        speaking.level,
        speaking.resistance,
        speaking.strength.against,
        0,
    );
    defences::save_roll(rng, speaking.difficulty.scale(chance, speaking.stance))
}

/// What the speaker does: a fizzle shows nothing; a backfire is noticed with
/// chance `(Senses + Intelligence) / 2`; on a cast that comes off they save,
/// and Mindread is answered with the refusal line if they did, else with the
/// next of their refusals, which advances up to the third; Charm, if they do
/// not save, wins them over by `7 * power` raised by the boost and cut by their
/// resistance, if that is more than they were charmed by already.
pub fn speak(rng: &mut GameRng, speaking: &Speaking) -> Spoken {
    let mut spoken =
        Spoken { reply: Reply::Unnoticed, resisted: false, refusals: speaking.refusals };
    match speaking.outcome {
        Outcome::Fizzled => {}
        Outcome::Backfired => {
            if rng.below(100) < (speaking.senses + speaking.intelligence) / 2 {
                spoken.reply = Reply::Quote(QUOTE_NOTICED);
            }
        }
        Outcome::Cast => {
            let saved = saves(rng, speaking);
            if speaking.mindread {
                spoken.reply = Reply::Quote(if saved {
                    QUOTE_SAVED
                } else {
                    spoken.refusals = (speaking.refusals + 1).min(MOST_REFUSALS);
                    QUOTE_FIRST_REFUSAL + u16::from(speaking.refusals)
                });
            } else if saved {
                spoken.resisted = true;
            } else {
                let Strength { power, boost, against } = speaking.strength;
                let amount = power * CHARM_PER_POWER;
                let amount = defences::resist(
                    rng,
                    amount + amount * boost / 100,
                    speaking.resistance.min(100),
                    speaking.level,
                    against,
                );
                if amount > speaking.charmed {
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

/// A monster a lure may take hold of.
#[derive(Clone, Copy, Debug)]
pub struct Lured {
    pub kind: Option<MonsterKind>,
    pub level: u32,
    pub resistance: i32,
    pub against: u32,
}

/// How a monster answers a lure (retail `MonsterResistsSpellEffect`): an
/// undead one always resists; the rest save in the Mental realm, their level
/// against the lure's strength, unscaled by difficulty.
pub fn lure_response(rng: &mut GameRng, monster: &Lured) -> Response {
    let chance = defences::condition_chance(monster.level, monster.resistance, monster.against, 0);
    if monster.kind == Some(MonsterKind::Undead) || defences::save_roll(rng, chance) {
        Response::Resisted
    } else {
        Response::Controlled
    }
}

/// A monster as a lure finds it.
#[derive(Clone, Copy, Debug)]
pub struct Drawn {
    pub hostile: bool,
    /// It has already resisted this lure and does not roll for it again.
    pub resisted: bool,
    pub can_fight: bool,
    pub blind: bool,
    /// How far it is from the spot along its furthest axis, and the far clip.
    pub off: f32,
    pub far: f32,
    /// A clear line from its eye to the spot.
    pub clear: bool,
}

/// Whether a lure puts a monster to the test: hostile, not already resisting,
/// able to fight and not blind, within the far clip of the spot on every axis,
/// with a clear line to it. One that does not qualify is passed over
/// (retail `ApplyMonsterControlToNearbyMonsters`).
pub fn lure_tests(monster: &Drawn) -> bool {
    monster.hostile
        && !monster.resisted
        && monster.can_fight
        && !monster.blind
        && monster.off <= monster.far
        && monster.clear
}

/// How near a controlled monster outside a fight comes to its lure, in metres.
const LURE_HELD_AT: f32 = 5.0;

/// A monster held by a lure, outside a fight.
#[derive(Clone, Copy, Debug)]
pub struct Following {
    pub response: Response,
    pub can_fight: bool,
    pub blind: bool,
    /// Its distance from the lure.
    pub distance: f32,
}

/// Whether it follows its lure at all (it faces it), and whether it walks to
/// it: while it is more than five metres off.
pub fn follows_lure(monster: &Following) -> (bool, bool) {
    let follows = monster.response == Response::Controlled && monster.can_fight && !monster.blind;
    (follows, follows && monster.distance > LURE_HELD_AT)
}

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

/// What summoning weighs.
#[derive(Clone, Copy, Debug)]
pub struct Summoning {
    /// The caster's Fire, Water, Air and Earth Magic; none for a monster
    /// caster, whose elements weigh 20 each.
    pub skills: Option<[u32; 4]>,
    pub under_water: bool,
    pub outcome: Outcome,
    pub strength: Strength,
}

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
pub fn summon(rng: &mut GameRng, facts: &Summoning) -> Summoned {
    let mut weights = facts.skills.map_or([SKILL_PLUS; 4], |skills| skills.map(|s| s + SKILL_PLUS));
    if facts.under_water {
        weights[0] = 0;
    }
    let element = weighted(rng.below(weights.iter().sum()), weights).unwrap_or(3);
    let tier = facts.strength.power.clamp(1, 7) - 1;
    let buffed = facts.strength.boost != 0 && rng.below(100) < facts.strength.boost;
    Summoned {
        element,
        species: FIRST_ELEMENTAL + tier * ELEMENTS + ELEMENT_OF_TIER[element],
        backfired: facts.outcome == Outcome::Backfired,
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

/// A trap going off (retail `g_trap_difficulty`).
#[derive(Clone, Copy, Debug)]
pub struct Springing<'a> {
    pub difficulty: u8,
    /// The grade of the type of trap.
    pub grade: u32,
    /// The living members, in the party's order.
    pub members: &'a [Entity],
    /// The one who set it off, who is never spared.
    pub selected: Option<Entity>,
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
pub fn spring(rng: &mut GameRng, trap: &Springing) -> Sprung {
    let difficulty = u32::from(trap.difficulty.clamp(1, 7));
    let destroyed = rng.below(2) == 0;
    let mut count = difficulty - rng.below(difficulty / 2);
    let extra = difficulty.max(trap.grade) - trap.grade;
    let power = (extra + 4).min(HIGHEST_LEVEL);
    if extra > 3 {
        count += rng.below(extra - 3);
    }
    let mut victims = trap.members.to_vec();
    while victims.len() > count as usize {
        let index = rng.below(victims.len() as u32) as usize;
        if Some(victims[index]) != trap.selected {
            victims.remove(index);
        }
    }
    Sprung { destroyed, power, victims }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speaking(outcome: Outcome) -> Speaking {
        Speaking {
            mindread: false,
            outcome,
            strength: Strength { power: 3, against: 10, boost: 0 },
            difficulty: Difficulty::Normal,
            stance: Stance::Neutral,
            level: 10,
            resistance: 0,
            senses: 40,
            intelligence: 60,
            charmed: 0,
            refusals: 0,
        }
    }

    /// A charm is kept only if it is more than they already have: 21 against 21
    /// wins nothing.
    #[test]
    fn a_charm_is_only_for_more_than_they_have_already() {
        let weak = Speaking {
            level: 1,
            resistance: -1000,
            strength: Strength { power: 3, against: 100, boost: 0 },
            charmed: 21,
            ..speaking(Outcome::Cast)
        };
        let mut rng = GameRng::seeded(2);
        for _ in 0..100 {
            assert!(!matches!(speak(&mut rng, &weak).reply, Reply::Charmed(_)));
        }
    }

    /// A mind read that is saved against answers with line 0x69; else the next
    /// refusal, 0x67 + the refusals so far, which advance to 2 and stop.
    #[test]
    fn a_mind_read_is_answered_by_the_refusal_lines_in_turn() {
        let weak = Speaking {
            mindread: true,
            level: 1,
            resistance: -1000,
            strength: Strength { power: 1, against: 100, boost: 0 },
            ..speaking(Outcome::Cast)
        };
        let mut rng = GameRng::seeded(4);
        let mut refused = |refusals| {
            (0..200)
                .map(|_| speak(&mut rng, &Speaking { refusals, ..weak }))
                .find(|spoken| spoken.refusals != refusals)
                .unwrap()
        };
        assert_eq!(refused(0), Spoken { reply: Reply::Quote(0x67), resisted: false, refusals: 1 });
        assert_eq!(refused(1), Spoken { reply: Reply::Quote(0x68), resisted: false, refusals: 2 });
        // The third refusal is the same line as a save, and they go no further.
        for _ in 0..50 {
            let third = speak(&mut rng, &Speaking { refusals: 2, ..weak });
            assert_eq!(third, Spoken { reply: Reply::Quote(0x69), resisted: false, refusals: 2 });
        }
        let strong = Speaking { level: 100, resistance: 1000, ..weak };
        let saved = (0..100).any(|_| {
            speak(&mut rng, &strong)
                == Spoken { reply: Reply::Quote(0x69), resisted: false, refusals: 0 }
        });
        assert!(saved);
    }

    fn drawn() -> Drawn {
        Drawn {
            hostile: true,
            resisted: false,
            can_fight: true,
            blind: false,
            off: 20.0,
            far: 30.0,
            clear: true,
        }
    }

    #[test]
    fn a_lure_tests_only_the_hostile_able_sighted_near_and_in_sight() {
        assert!(lure_tests(&drawn()));
        assert!(lure_tests(&Drawn { off: 30.0, ..drawn() }), "the far clip itself is in");
        assert!(!lure_tests(&Drawn { off: 30.5, ..drawn() }));
        assert!(!lure_tests(&Drawn { hostile: false, ..drawn() }));
        assert!(!lure_tests(&Drawn { resisted: true, ..drawn() }));
        assert!(!lure_tests(&Drawn { can_fight: false, ..drawn() }));
        assert!(!lure_tests(&Drawn { blind: true, ..drawn() }));
        assert!(!lure_tests(&Drawn { clear: false, ..drawn() }));
    }

    /// Controlled, able and sighted it faces the lure and walks to it while more
    /// than five metres off.
    #[test]
    fn a_controlled_monster_walks_to_a_lure_more_than_five_metres_off() {
        let held = Following {
            response: Response::Controlled,
            can_fight: true,
            blind: false,
            distance: 5.1,
        };
        assert_eq!(follows_lure(&held), (true, true));
        assert_eq!(follows_lure(&Following { distance: 5.0, ..held }), (true, false));
        assert_eq!(
            follows_lure(&Following { response: Response::Resisted, ..held }),
            (false, false)
        );
        assert_eq!(follows_lure(&Following { can_fight: false, ..held }), (false, false));
        assert_eq!(follows_lure(&Following { blind: true, ..held }), (false, false));
    }

    /// The power raised by the boost, held at seven: 4 + 4 * 50 / 100 = 6; 5 + 5 = 10
    /// is held to 7; no boost leaves the power.
    #[test]
    fn a_lock_or_trap_spells_level_is_the_power_boosted_and_held_at_seven() {
        let at = |power, boost| level(Strength { power, against: 0, boost });
        assert_eq!((at(4, 50), at(5, 100), at(3, 0), at(7, 0)), (6, 7, 3, 7));
    }
}
