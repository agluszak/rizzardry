//! Choosing whom to strike and with what (`docs/rules/attack.md` section 2),
//! and what an actor with nobody to strike does instead.

use enum_map::EnumMap;
use enumset::EnumSet;

use super::choose_mode;
use crate::content::items::AttackMode;
use crate::game::characters::Condition;
use crate::game::combat::monster::close::MoveBudget;
use crate::game::inventory::WeaponSlot;
use crate::game::party::formation::Row;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// One arm of the attacker: which slot it belongs to and its modes.
#[derive(Clone, Copy, Debug)]
pub struct ArmFacts {
    pub slot: WeaponSlot,
    pub modes: EnumSet<AttackMode>,
}

/// A living body in the fight other than the attacker, as the attacker sees it.
#[derive(Clone, Debug)]
pub struct Foe {
    /// How the attacker stands toward it.
    pub stance: Stance,
    /// It is an NPC's body, whom a member under orders never strikes.
    pub npc: bool,
    /// For each of the attacker's arms, by index, whether it reaches it.
    pub reached_by: Vec<bool>,
    /// A clear line from the attacker's eye to its centre.
    pub seen: bool,
    /// A party member's Stealth skill points; anyone else has none.
    pub stealth: Option<u32>,
    pub worst: Option<Condition>,
    /// It has an arm.
    pub armed: bool,
    /// The side of the attacker's frame it stands on.
    pub side: Row,
    /// The gap between the edges of the two bodies.
    pub gap: f32,
    /// The squared distance between feet.
    pub distance_squared: f32,
}

/// What a body that can walk would do with nobody to strike.
#[derive(Clone, Copy, Debug)]
pub struct Closing {
    /// How it stands toward the party.
    pub stance: Stance,
    /// What it may go toward the party, if it needs to and can.
    pub toward_party: Option<MoveBudget>,
    /// What it may go toward a foe, if it can move at all.
    pub toward_foes: Option<MoveBudget>,
    /// The moment of the round now.
    pub now: u32,
}

/// Everything choosing a target and an arm weighs.
#[derive(Clone, Copy, Debug)]
pub struct Choosing<'a> {
    /// A party member under orders; any other decides for itself.
    pub ordered: bool,
    /// It has turned on its own this round (insanity), and looks for friends
    /// rather than foes.
    pub turned: bool,
    /// A Berserk order, which strikes in the berserk mode.
    pub berserk: bool,
    /// A wary body that may hold back this moment, which is one more equal chance.
    pub waits: bool,
    pub due: WeaponSlot,
    pub attacks_left: EnumMap<WeaponSlot, u32>,
    pub arms: &'a [ArmFacts],
    pub foes: &'a [Foe],
    /// Which rows of the party's formation are occupied.
    pub occupied: EnumMap<Row, bool>,
    /// Present only for a body with a stride that can walk and has not moved.
    pub closing: Option<Closing>,
}

/// Where a body closing in heads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    Party,
    /// The foe at this index.
    Foe(usize),
}

/// What an actor does with its moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chosen {
    /// Strike the foe at `target` with the arm at `arm` in `mode`, from `slot`,
    /// which then has `left` attacks.
    Strike { slot: WeaponSlot, arm: usize, mode: AttackMode, target: usize, left: u32 },
    /// Close in: go as far as the budget allows, and act again at `again`.
    Advance { toward: Toward, budget: MoveBudget, again: u32 },
    /// Hold back warily: done for the round.
    Wait,
    /// Done for the round.
    Stand,
}

/// What choosing came to, and which foes (by index) a wild attacker's eye
/// missed who may learn Stealth from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub chosen: Chosen,
    pub learns_stealth: Vec<usize>,
}

/// What the number of swings of an attack weighs.
#[derive(Clone, Copy, Debug)]
pub struct SwingCount {
    /// The most swings of the arm, and what the weapon adds (it may be
    /// negative).
    pub swings: u32,
    pub extra: i32,
    pub main_slot: bool,
    /// A Samurai's Lightning Strike readied this round, by their Samurai level.
    pub lightning: Option<u8>,
    pub sword: bool,
    pub slowed: bool,
}

/// The swings of an attack, and whether a readied Lightning Strike is used up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swings {
    pub count: u32,
    pub lightning_spent: bool,
}

/// How many swings an attack makes: `below(swings) + 1`, the weapon's extra
/// swings added, at least 1; with a sword in the main slot, a Samurai's
/// readied Lightning Strike, and not slowed, 3 more if `below(11)` plus their
/// level is under 20, otherwise 4 (retail `BeginCombatExecution`). The readied
/// Strike is used up by the first attack of the main slot, sword or not.
pub fn swing_count(rng: &mut GameRng, facts: &SwingCount) -> Swings {
    let mut count = (rng.below(facts.swings) as i32 + 1 + facts.extra).max(1) as u32;
    let readied = facts.lightning.filter(|_| facts.main_slot);
    if let Some(level) = readied
        && facts.sword
        && !facts.slowed
    {
        count += if rng.below(11) + u32::from(level) < 20 { 3 } else { 4 };
    }
    Swings { count, lightning_spent: readied.is_some() }
}

impl Choosing<'_> {
    /// The stance it looks for: foes, or friends when it has turned on its own.
    fn seeks(&self) -> Stance {
        if self.turned { Stance::Friendly } else { Stance::Hostile }
    }
}

/// Whether a body is "in reach of the party": armed, on the rear side, or on an
/// unoccupied side while the center rank is occupied.
pub fn in_reach_of_party(armed: bool, side: Row, occupied: &EnumMap<Row, bool>) -> bool {
    armed && (side == Row::Rear || (!occupied[side] && occupied[Row::Center]))
}

/// Whether a wild attacker's eye misses a party member this time: with chance
/// `Stealth * 75 / 100` percent.
pub fn eye_misses(rng: &mut GameRng, stealth: u32) -> bool {
    rng.below(100) < stealth * 75 / 100
}

/// A way to strike: an arm, one of its modes, a foe.
#[derive(Clone, Copy)]
struct Way {
    arm: usize,
    mode: AttackMode,
    target: usize,
}

/// Choose whom to strike and how (section 2), or what to do instead.
pub fn choose(rng: &mut GameRng, facts: &Choosing) -> Choice {
    // Those a wild attacker's eye misses this time.
    let lost: Vec<bool> = facts
        .foes
        .iter()
        .map(|foe| !facts.ordered && foe.stealth.is_some_and(|stealth| eye_misses(rng, stealth)))
        .collect();
    let mut evaded = Vec::new();
    let mut found = None;
    for slot in [facts.due, facts.due.other()] {
        if facts.attacks_left[slot] == 0 {
            continue;
        }
        let mut options = Vec::new();
        for (arm, held) in facts.arms.iter().enumerate().filter(|(_, arm)| arm.slot == slot) {
            for (target, foe) in facts.foes.iter().enumerate() {
                if foe.stance != facts.seeks()
                    || (facts.ordered && foe.npc)
                    || !foe.reached_by[arm]
                    || (!facts.ordered && !foe.seen)
                {
                    continue;
                }
                if lost[target] {
                    if !evaded.contains(&target) {
                        evaded.push(target);
                    }
                    continue;
                }
                options.extend(held.modes.iter().map(|mode| Way { arm, mode, target }));
            }
        }
        if !options.is_empty() {
            found = Some((slot, options));
            break;
        }
    }
    let learns_stealth: Vec<usize> = evaded.into_iter().filter(|_| rng.below(2) == 0).collect();
    let Some((slot, options)) = found else {
        return Choice { chosen: close_in(facts), learns_stealth };
    };
    let picked = if facts.ordered {
        // The foe in the lowest condition (a sound one before a hurt one), then
        // one in reach of the party, then the nearest; ties keep the fight's
        // order (retail `CompareMonsterTargetCandidates`).
        let key = |option: &Way| {
            let foe = &facts.foes[option.target];
            (foe.worst, !in_reach_of_party(foe.armed, foe.side, &facts.occupied), foe.gap)
        };
        let best = options
            .iter()
            .min_by(|a, b| {
                let (a, b) = (key(a), key(b));
                a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2))
            })
            .copied()
            .unwrap();
        let mode = if facts.berserk {
            AttackMode::Berserk
        } else {
            choose_mode(rng, facts.arms[best.arm].modes)
        };
        Some(Way { mode, ..best })
    } else {
        // Every way to strike has an equal chance, and so has waiting (retail
        // `ChooseMonsterTarget`, `BuildMonsterActionQueue`).
        let waits = usize::from(facts.waits);
        let pick = rng.below((options.len() + waits) as u32) as usize;
        (pick >= waits).then(|| options[pick - waits])
    };
    let chosen = match picked {
        Some(Way { arm, mode, target }) => {
            Chosen::Strike { slot, arm, mode, target, left: facts.attacks_left[slot] - 1 }
        }
        None => Chosen::Wait,
    };
    Choice { chosen, learns_stealth }
}

/// What a body with nobody to strike does: one that can walk closes in on the
/// nearest foe it could strike if it has turned on its own, or on the party if
/// it is hostile to it, and acts again at twice the moment now, no later than
/// 100 (retail `SetUpMonsterTurn`); anything else stands.
fn close_in(facts: &Choosing) -> Chosen {
    let Some(closing) = facts.closing else { return Chosen::Stand };
    let again = (closing.now * 2).min(100);
    let going = match closing.stance {
        Stance::Friendly => facts
            .foes
            .iter()
            .enumerate()
            .filter(|(_, foe)| foe.stance == facts.seeks())
            .min_by(|a, b| a.1.distance_squared.total_cmp(&b.1.distance_squared))
            .zip(closing.toward_foes)
            .map(|((foe, _), budget)| (Toward::Foe(foe), budget)),
        Stance::Hostile => closing.toward_party.map(|budget| (Toward::Party, budget)),
        Stance::Neutral => None,
    };
    going.map_or(Chosen::Stand, |(toward, budget)| Chosen::Advance { toward, budget, again })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A foe the attacker stands hostile to and reaches with arm 0, in view.
    fn foe() -> Foe {
        Foe {
            stance: Stance::Hostile,
            npc: false,
            reached_by: vec![true],
            seen: true,
            stealth: None,
            worst: None,
            armed: true,
            side: Row::Front,
            gap: 1.0,
            distance_squared: 1.0,
        }
    }

    const ARMS: [ArmFacts; 1] =
        [ArmFacts { slot: WeaponSlot::Primary, modes: enumset::enum_set!(AttackMode::Swing) }];

    fn facts<'a>(arms: &'a [ArmFacts], foes: &'a [Foe]) -> Choosing<'a> {
        Choosing {
            ordered: true,
            turned: false,
            berserk: false,
            waits: false,
            due: WeaponSlot::Primary,
            attacks_left: EnumMap::from_fn(|_| 2),
            arms,
            foes,
            occupied: EnumMap::default(),
            closing: None,
        }
    }

    fn target(choice: &Choice) -> usize {
        let Chosen::Strike { target, .. } = choice.chosen else { panic!("{choice:?}") };
        target
    }

    fn pick(facts: &Choosing) -> Choice {
        choose(&mut GameRng::seeded(0), facts)
    }

    fn swings(facts: SwingCount) -> Swings {
        swing_count(&mut GameRng::seeded(0), &facts)
    }

    fn count_of(swings: u32, extra: i32) -> SwingCount {
        SwingCount { swings, extra, main_slot: true, lightning: None, sword: true, slowed: false }
    }

    #[test]
    fn a_lightning_strike_needs_a_sword_the_main_slot_and_no_slowing_but_is_used_up_in_the_main_slot()
     {
        let readied = SwingCount { lightning: Some(5), ..count_of(1, 0) };
        // Without a sword or slowed: no swings added, still used up.
        assert_eq!(
            swings(SwingCount { sword: false, ..readied }),
            Swings { count: 1, lightning_spent: true }
        );
        assert_eq!(
            swings(SwingCount { slowed: true, ..readied }),
            Swings { count: 1, lightning_spent: true }
        );
        // The off hand's attack neither uses it nor gains.
        assert_eq!(
            swings(SwingCount { main_slot: false, ..readied }),
            Swings { count: 1, lightning_spent: false }
        );
        assert_eq!(swings(count_of(1, 0)), Swings { count: 1, lightning_spent: false });
    }

    #[test]
    fn a_body_looks_for_foes_or_for_friends_when_it_has_turned_on_its_own() {
        let mut friend = foe();
        friend.stance = Stance::Friendly;
        let foes = [friend, foe()];
        assert_eq!(target(&pick(&facts(&ARMS, &foes))), 1);
        let turned = Choosing { turned: true, ..facts(&ARMS, &foes) };
        assert_eq!(target(&pick(&turned)), 0);
        // A neutral is nobody's.
        let mut neutral = foe();
        neutral.stance = Stance::Neutral;
        assert_eq!(pick(&facts(&ARMS, &[neutral])).chosen, Chosen::Stand);
    }

    #[test]
    fn a_member_under_orders_strikes_the_soundest_foe_first() {
        // No condition counts lowest, then the milder: Poisoned comes before
        // Asleep in the order of conditions.
        let mut poisoned = foe();
        poisoned.worst = Some(Condition::Poisoned);
        let mut asleep = foe();
        asleep.worst = Some(Condition::Asleep);
        let sound = foe();
        assert_eq!(target(&pick(&facts(&ARMS, &[asleep.clone(), poisoned.clone(), sound]))), 2);
        assert_eq!(target(&pick(&facts(&ARMS, &[asleep, poisoned]))), 1);
    }

    #[test]
    fn then_one_in_reach_of_the_party_and_then_the_nearer() {
        let mut occupied = EnumMap::default();
        occupied[Row::Center] = true;
        occupied[Row::Front] = true;
        // The front is held, so a foe there is not in reach of the party.
        let mut front = foe();
        front.gap = 1.0;
        // Rear: in reach of the party whatever the center.
        let mut rear = foe();
        rear.side = Row::Rear;
        rear.gap = 9.0;
        // A side nobody holds while the center rank is occupied.
        let mut open_left = foe();
        open_left.side = Row::Left;
        open_left.gap = 5.0;
        // Armed is required: an unarmed foe on the rear is not.
        let mut unarmed = foe();
        unarmed.side = Row::Rear;
        unarmed.armed = false;
        unarmed.gap = 0.5;
        let foes = [front.clone(), unarmed.clone(), rear.clone(), open_left.clone()];
        let at = Choosing { occupied, ..facts(&ARMS, &foes) };
        // In reach: rear (9.0) and open left (5.0); the nearer, 5.0, is index 3.
        assert_eq!(target(&pick(&at)), 3);
        // Occupy the left side too and it no longer counts: the rear does.
        let mut held = occupied;
        held[Row::Left] = true;
        assert_eq!(target(&pick(&Choosing { occupied: held, ..facts(&ARMS, &foes) })), 2);
        // With the center empty only the rear counts.
        let none = Choosing { occupied: EnumMap::default(), ..facts(&ARMS, &foes) };
        assert_eq!(target(&pick(&none)), 2);
        // Nobody in reach of the party: the smallest gap, here the unarmed 0.5.
        let foes = [front, unarmed, open_left];
        let at = Choosing { occupied: EnumMap::default(), ..facts(&ARMS, &foes) };
        assert_eq!(target(&pick(&at)), 1);
    }

    #[test]
    fn a_member_under_orders_never_strikes_an_npc_but_needs_no_sight() {
        let mut npc = foe();
        npc.npc = true;
        npc.seen = false;
        let mut hidden = foe();
        hidden.seen = false;
        assert_eq!(target(&pick(&facts(&ARMS, &[npc.clone(), hidden.clone()]))), 1);
        assert_eq!(pick(&facts(&ARMS, &[npc])).chosen, Chosen::Stand);
    }

    #[test]
    fn an_actor_deciding_for_itself_needs_to_see_what_it_strikes() {
        let mut hidden = foe();
        hidden.seen = false;
        let wild = |foes: &[Foe]| {
            let facts = Choosing { ordered: false, ..facts(&ARMS, foes) };
            pick(&facts)
        };
        assert_eq!(wild(&[hidden.clone()]).chosen, Chosen::Stand);
        assert_eq!(target(&wild(&[hidden, foe()])), 1);
    }

    #[test]
    fn the_slot_due_is_used_if_it_has_a_candidate_and_attacks_left() {
        let arms = [
            ArmFacts { slot: WeaponSlot::Primary, modes: AttackMode::Swing.into() },
            ArmFacts { slot: WeaponSlot::Secondary, modes: AttackMode::Punch.into() },
        ];
        let mut both = foe();
        both.reached_by = vec![true, true];
        let foes = [both];
        let mut at = facts(&arms, &foes);
        at.due = WeaponSlot::Secondary;
        at.attacks_left = EnumMap::from_fn(|slot| if slot == WeaponSlot::Primary { 3 } else { 2 });
        // The slot due, with the attacks left in it one fewer.
        assert!(matches!(
            pick(&at).chosen,
            Chosen::Strike { slot: WeaponSlot::Secondary, left: 1, mode: AttackMode::Punch, .. }
        ));
        // Out of attacks in it: the other, the same way.
        at.attacks_left[WeaponSlot::Secondary] = 0;
        assert!(matches!(
            pick(&at).chosen,
            Chosen::Strike { slot: WeaponSlot::Primary, left: 2, .. }
        ));
    }

    fn budget() -> MoveBudget {
        MoveBudget { metres: 6.0, charge: false }
    }

    fn closing(stance: Stance) -> Closing {
        Closing {
            stance,
            toward_party: Some(budget()),
            toward_foes: Some(MoveBudget { metres: 9.0, charge: true }),
            now: 30,
        }
    }

    fn lonely(closing: Option<Closing>, foes: &[Foe]) -> Chosen {
        let mut facts = facts(&ARMS, foes);
        facts.ordered = false;
        facts.closing = closing;
        pick(&facts).chosen
    }

    #[test]
    fn a_hostile_body_with_nobody_to_strike_closes_in_on_the_party() {
        // Twice the moment now, 30, is 60.
        assert_eq!(
            lonely(Some(closing(Stance::Hostile)), &[]),
            Chosen::Advance { toward: Toward::Party, budget: budget(), again: 60 }
        );
        // No later than 100.
        let late = Closing { now: 70, ..closing(Stance::Hostile) };
        assert!(matches!(lonely(Some(late), &[]), Chosen::Advance { again: 100, .. }));
        // Already in reach of the party (nothing to go toward): it stands.
        let near = Closing { toward_party: None, ..closing(Stance::Hostile) };
        assert_eq!(lonely(Some(near), &[]), Chosen::Stand);
    }

    #[test]
    fn a_turncoat_goes_toward_the_nearest_foe_it_could_strike() {
        let (mut near, mut far, mut friend) = (foe(), foe(), foe());
        // None of them within reach of its arm.
        near.reached_by = vec![false];
        far.reached_by = vec![false];
        near.distance_squared = 4.0;
        far.distance_squared = 100.0;
        friend.stance = Stance::Friendly;
        friend.distance_squared = 1.0;
        // It seeks foes: the friend, nearer still, is not one.
        let chosen = lonely(Some(closing(Stance::Friendly)), &[far, friend, near]);
        assert_eq!(
            chosen,
            Chosen::Advance {
                toward: Toward::Foe(2),
                budget: MoveBudget { metres: 9.0, charge: true },
                again: 60
            }
        );
        // It cannot move: it stands. Nobody to go to: it stands.
        let mut out = foe();
        out.reached_by = vec![false];
        let still = Closing { toward_foes: None, ..closing(Stance::Friendly) };
        assert_eq!(lonely(Some(still), &[out]), Chosen::Stand);
        assert_eq!(lonely(Some(closing(Stance::Friendly)), &[]), Chosen::Stand);
    }
}
