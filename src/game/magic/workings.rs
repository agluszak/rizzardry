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

    fn id(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

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

    /// A save of (10 - 10)*3 + 0 + 90 = 90 percent. A hostile speaker's is
    /// scaled by 3*20/100 on Novice (54) and by 7*20/100 on Expert (126, held
    /// to the most a save may be, 95); a neutral one's, or any on Normal, is
    /// not (90).
    #[test]
    fn a_save_against_charm_is_scaled_for_difficulty_by_the_speakers_side() {
        let mut rng = GameRng::seeded(11);
        let mut rate = |difficulty, stance| {
            let speaking =
                Speaking { difficulty, stance, resistance: 90, ..speaking(Outcome::Cast) };
            (0..4000).filter(|_| saves(&mut rng, &speaking)).count() / 40
        };
        let near = |percent: usize, expected: usize| percent.abs_diff(expected) <= 4;
        assert!(near(rate(Difficulty::Novice, Stance::Hostile), 54));
        assert!(near(rate(Difficulty::Expert, Stance::Hostile), 95));
        assert!(near(rate(Difficulty::Normal, Stance::Hostile), 90));
        assert!(near(rate(Difficulty::Novice, Stance::Neutral), 90));
    }

    /// A speaker who can never save (a level 1 against a strength of 100: held to
    /// the least, 5) is charmed by 7 a power: 21 at power 3; raised by a boost of
    /// 50 percent, 31; and cut by nothing at a resistance of -100.
    #[test]
    fn charm_wins_over_by_seven_a_power_raised_by_the_boost() {
        let weak = Speaking {
            level: 1,
            resistance: -1000,
            strength: Strength { power: 3, against: 100, boost: 0 },
            ..speaking(Outcome::Cast)
        };
        let mut rng = GameRng::seeded(1);
        let amounts: Vec<Reply> = (0..200).map(|_| speak(&mut rng, &weak).reply).collect();
        assert!(amounts.contains(&Reply::Charmed(21)), "21 at power 3");
        // Almost every cast wins them over: 95 percent fail to save.
        let charmed = amounts.iter().filter(|reply| matches!(reply, Reply::Charmed(_))).count();
        assert!(charmed > 170, "{charmed}");
        let boosted = Speaking { strength: Strength { boost: 50, ..weak.strength }, ..weak };
        assert!((0..200).any(|_| speak(&mut rng, &boosted).reply == Reply::Charmed(31)));
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

    /// The speaker who saves is told as resisted.
    #[test]
    fn a_speaker_who_saves_resists_a_charm() {
        let strong = Speaking {
            level: 100,
            resistance: 1000,
            strength: Strength { power: 1, against: 1, boost: 0 },
            ..speaking(Outcome::Cast)
        };
        let mut rng = GameRng::seeded(3);
        let resisted = (0..200).filter(|_| speak(&mut rng, &strong).resisted).count();
        assert!(resisted > 170, "{resisted}");
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

    /// A fizzle shows nothing; a backfire is noticed with chance (40 + 60) / 2 = 50.
    #[test]
    fn a_backfire_is_noticed_by_the_senses_and_intelligence_of_the_speaker() {
        let mut rng = GameRng::seeded(5);
        assert_eq!(speak(&mut rng, &speaking(Outcome::Fizzled)).reply, Reply::Unnoticed);
        let noticed = (0..2000)
            .filter(|_| speak(&mut rng, &speaking(Outcome::Backfired)).reply == Reply::Quote(0x16))
            .count();
        assert!((900..=1100).contains(&noticed), "{noticed}");
    }

    /// Level 1 against a strength of 100, in a realm of resistance -100:
    /// (1 - 100)*3 + 0 - 100 = -397, held to the least save, 5%.
    #[test]
    fn an_undead_monster_always_resists_a_lure_and_the_rest_save_by_the_rule() {
        let mut rng = GameRng::seeded(7);
        let weak = |kind| Lured { kind, level: 1, resistance: -100, against: 100 };
        let answers = |kind, rng: &mut GameRng| {
            (0..400).filter(|_| lure_response(rng, &weak(kind)) == Response::Resisted).count()
        };
        assert_eq!(answers(Some(MonsterKind::Undead), &mut rng), 400);
        // Five in a hundred save: far from all, far from none.
        let saved = answers(Some(MonsterKind::Beast), &mut rng);
        assert!((5..=50).contains(&saved), "{saved}");
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

    #[test]
    fn a_lure_stands_for_its_turns() {
        assert_eq!(lure_ends(1_000, 4, 250), 2_000);
        assert_eq!(lure_ends(1_000, 0, 250), 1_000);
    }

    fn summoning(power: u32, boost: u32) -> Summoning {
        Summoning {
            skills: Some([100, 0, 0, 0]),
            under_water: false,
            outcome: Outcome::Cast,
            strength: Strength { power, against: 0, boost },
        }
    }

    /// Skills of 100, 0, 0, 0 weigh 120, 20, 20, 20: fire most often (120 of 180),
    /// each element sometimes. Tier of power p: 21 + (p - 1) * 4 + (air 0, earth 1,
    /// water 2, fire 3): power 1 fire 24, water 23, air 21, earth 22; power 7 fire
    /// 48.
    #[test]
    fn an_elemental_is_of_an_element_by_the_skills_and_of_a_tier_by_the_power() {
        let mut rng = GameRng::seeded(6);
        let calls: Vec<Summoned> = (0..600).map(|_| summon(&mut rng, &summoning(1, 0))).collect();
        let fire = calls.iter().filter(|call| call.element == 0).count();
        assert!((340..=460).contains(&fire), "{fire} of 600");
        for call in &calls {
            let species = [24, 23, 21, 22][call.element];
            assert_eq!(call.species, species);
            assert!(!call.backfired && !call.buffed);
        }
        let top: Vec<u32> = (0..100).map(|_| summon(&mut rng, &summoning(7, 0)).species).collect();
        assert!(top.iter().all(|species| [48, 47, 45, 46].contains(species)));
        // A power past seven is held to it.
        assert!((0..20).all(|_| summon(&mut rng, &summoning(9, 0)).species >= 45));
    }

    #[test]
    fn under_water_there_is_no_fire_and_a_monster_weighs_the_elements_alike() {
        let mut rng = GameRng::seeded(7);
        let wet = Summoning { under_water: true, ..summoning(1, 0) };
        assert!((0..300).all(|_| summon(&mut rng, &wet).element != 0));
        let monster = Summoning { skills: None, ..summoning(1, 0) };
        let counts = (0..800).fold([0; 4], |mut counts, _| {
            counts[summon(&mut rng, &monster).element] += 1;
            counts
        });
        assert!(counts.iter().all(|&count| (140..=260).contains(&count)), "{counts:?}");
    }

    /// A backfired summoning is hostile; Power Cast buffs with chance equal to the
    /// boost: none at 0, always at 100.
    #[test]
    fn a_summoning_that_backfired_is_hostile_and_power_cast_may_buff_it() {
        let mut rng = GameRng::seeded(8);
        let backfired = Summoning { outcome: Outcome::Backfired, ..summoning(1, 0) };
        assert!(summon(&mut rng, &backfired).backfired);
        assert!((0..100).all(|_| summon(&mut rng, &summoning(1, 100)).buffed));
        assert!((0..100).all(|_| !summon(&mut rng, &summoning(1, 0)).buffed));
        let half = (0..2000).filter(|_| summon(&mut rng, &summoning(1, 50)).buffed).count();
        assert!((900..=1100).contains(&half), "{half}");
        assert_eq!(
            [element(0), element(1), element(2), element(3)],
            [Realm::Fire, Realm::Water, Realm::Air, Realm::Earth]
        );
    }

    /// The power raised by the boost, held at seven: 4 + 4 * 50 / 100 = 6; 5 + 5 = 10
    /// is held to 7; no boost leaves the power.
    #[test]
    fn a_lock_or_trap_spells_level_is_the_power_boosted_and_held_at_seven() {
        let at = |power, boost| level(Strength { power, against: 0, boost });
        assert_eq!((at(4, 50), at(5, 100), at(3, 0), at(7, 0)), (6, 7, 3, 7));
    }

    /// Level 3 reaches 4 of the lock's tumblers, each moving 516 times in a
    /// thousand; a lock of 3 has only 3 to reach. Level 1 moves 344 in a thousand.
    #[test]
    fn knock_knock_reaches_a_level_and_one_of_the_tumblers_and_moves_some_of_them() {
        let mut rng = GameRng::seeded(9);
        let reached = knock(&mut rng, 6, 3);
        assert_eq!(reached.len(), 4);
        let mut which: Vec<usize> = reached.iter().map(|tumbler| tumbler.tumbler).collect();
        which.sort_unstable();
        which.dedup();
        assert_eq!(which.len(), 4, "each tumbler once");
        assert!(which.iter().all(|&tumbler| tumbler < 6));
        assert_eq!(knock(&mut rng, 3, 3).len(), 3);
        let moved = |level: u32, rng: &mut GameRng| {
            (0..1000)
                .map(|_| knock(rng, 8, level).iter().filter(|r| r.moves).count())
                .sum::<usize>()
        };
        // 1000 locks, 2 tumblers at level 1: 2000 tries at 344 per mille.
        let first = moved(1, &mut rng);
        assert!((600..=780).contains(&first), "{first}");
        // Level 7 reaches 8: 8000 tries at 602 per mille.
        let last = moved(7, &mut rng);
        assert!((4_600..=5_000).contains(&last), "{last}");
    }

    /// A roll below 100 plus 6 a point of difficulty against 5 a level and 50:
    /// at level 7 and difficulty 5 the bar is 85 against 30 + roll: a roll
    /// under 55 passes, 55 percent of the time. A backfire never does.
    #[test]
    fn a_trap_is_disarmed_by_a_roll_and_its_difficulty_against_the_level() {
        let mut rng = GameRng::seeded(10);
        let passed = (0..4000).filter(|_| disarms(&mut rng, 7, 5, Outcome::Cast)).count();
        assert!((2_000..=2_400).contains(&passed), "{passed}");
        assert!((0..500).all(|_| !disarms(&mut rng, 7, 0, Outcome::Backfired)));
        // Difficulty 0 at level 7: 85, so a roll below 85 passes: 85 percent.
        let easy = (0..4000).filter(|_| disarms(&mut rng, 7, 0, Outcome::Cast)).count();
        assert!((3_250..=3_550).contains(&easy), "{easy}");
    }

    fn springing<'a>(difficulty: u8, grade: u32, members: &'a [Entity]) -> Springing<'a> {
        Springing { difficulty, grade, members, selected: Some(members[0]) }
    }

    /// Difficulty 3 of grade 3: extra 0, power 4; difficulty 7 of grade 2: extra 5,
    /// power 7 (5 + 4 = 9, held to 7); difficulty 9 is held to 7; difficulty 0 to 1.
    #[test]
    fn a_trap_spell_has_a_power_by_how_far_the_difficulty_is_over_its_grade() {
        let members: Vec<Entity> = (1..=6).map(id).collect();
        let mut rng = GameRng::seeded(11);
        let power = |difficulty, grade, rng: &mut GameRng| {
            spring(rng, &springing(difficulty, grade, &members)).power
        };
        assert_eq!(power(3, 3, &mut rng), 4);
        assert_eq!(power(7, 2, &mut rng), 7);
        assert_eq!(power(9, 2, &mut rng), 7);
        assert_eq!(power(1, 5, &mut rng), 4, "under its grade is no extra");
        assert_eq!(power(0, 5, &mut rng), 4);
        assert_eq!(power(5, 2, &mut rng), 7, "extra 3 is 7");
        assert_eq!(power(4, 2, &mut rng), 6, "extra 2 is 6");
    }

    /// Difficulty 4 reaches 4 - below(2) = 3 or 4 members; difficulty 1 exactly 1;
    /// the one who set it off is never spared; the destroyed half the time.
    #[test]
    fn a_trap_spell_reaches_a_few_members_never_sparing_the_one_who_set_it_off() {
        let members: Vec<Entity> = (1..=6).map(id).collect();
        let mut rng = GameRng::seeded(12);
        let mut destroyed = 0;
        for _ in 0..400 {
            let sprung = spring(&mut rng, &springing(4, 4, &members));
            assert!((3..=4).contains(&sprung.victims.len()), "{sprung:?}");
            assert!(sprung.victims.contains(&members[0]));
            destroyed += usize::from(sprung.destroyed);
            let one = spring(&mut rng, &springing(1, 4, &members));
            assert_eq!(one.victims, [members[0]]);
        }
        assert!((140..=260).contains(&destroyed), "{destroyed}");
        // More than there are members is all of them: extra 5 at difficulty 7.
        let few: Vec<Entity> = (1..=2).map(id).collect();
        let sprung = spring(&mut rng, &springing(7, 2, &few));
        assert_eq!(sprung.victims, few);
    }

    /// Extra over 3 adds below(extra - 3) members: difficulty 7 of grade 0 (extra 7)
    /// adds below(4); 7 - below(3) is 5 to 7, plus 0 to 3, so it reaches 5 to 10
    /// of ten members, and sometimes more than 7.
    #[test]
    fn a_trap_far_over_its_grade_reaches_more_members() {
        let members: Vec<Entity> = (1..=10).map(id).collect();
        let mut rng = GameRng::seeded(13);
        let counts: Vec<usize> =
            (0..600).map(|_| spring(&mut rng, &springing(7, 0, &members)).victims.len()).collect();
        assert!(counts.iter().all(|count| (5..=10).contains(count)));
        assert!(counts.iter().any(|&count| count > 7));
    }
}
