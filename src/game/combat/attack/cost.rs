//! What a swing costs and uses up, and the attacker's next attack
//! (`docs/rules/attack.md` sections 6 and 7).

use enum_map::EnumMap;

use crate::content::items::AttackMode;
use crate::game::characters::Load;
use crate::game::inventory::WeaponSlot;
use crate::game::rng::GameRng;

/// What a swing's stamina weighs.
#[derive(Clone, Copy, Debug)]
pub struct Cost {
    /// The arm tires its wielder: a member's does, a monster's costs nothing.
    pub tires: bool,
    pub mode: AttackMode,
    /// What the weapon weighs; none for a natural weapon.
    pub weight: Option<u16>,
    /// A Berserk order, which doubles it.
    pub berserk: bool,
    pub diseased: bool,
    /// What the attacker carries; one that carries nothing weighed pays nothing.
    pub load: Option<Load>,
    pub slowed: bool,
    pub hasted: bool,
}

/// The draw behind a swing's stamina: it costs `below(n) + add` (retail
/// `CharacterActionFatigueCost`). A punch is `2 + below(3)`, a kick
/// `3 + below(3)`, a weapon without a weight `3 + below(4)`, one of weight `w`
/// `below(band) + 1 + band` with `band = w / 40 + 1`.
pub fn stamina_draw(mode: AttackMode, weight: Option<u16>) -> (u32, u32) {
    match mode {
        AttackMode::Punch => (3, 2),
        AttackMode::Kick => (3, 3),
        _ => match weight {
            None => (4, 3),
            Some(weight) => {
                let band = u32::from(weight) / 40 + 1;
                (band, 1 + band)
            }
        },
    }
}

/// Stamina spent by one swing, hit or miss: the draw, doubled by a Berserk
/// order and again by disease, then scaled by the load (none 0 percent, light
/// 25, medium 50, heavy 100, extreme 200; minus 25 if slowed, otherwise plus 25
/// if hasted) as `cost + percent * cost / 100` (retail `FatigueCharacter`).
pub fn swing_stamina(rng: &mut GameRng, facts: &Cost) -> u32 {
    let Some(load) = facts.load.filter(|_| facts.tires) else { return 0 };
    let (below, add) = stamina_draw(facts.mode, facts.weight);
    let cost = (rng.below(below) + add)
        * if facts.berserk { 2 } else { 1 }
        * if facts.diseased { 2 } else { 1 };
    load.scale_fatigue(cost, facts.slowed, facts.hasted)
}

/// What a swing uses up of what is shot or thrown.
#[derive(Clone, Copy, Debug)]
pub struct Ammunition {
    /// A member's missile weapon whose ammunition is at hand; nothing else
    /// uses anything up.
    pub ranged: bool,
    /// It shoots the missiles in the other hand.
    pub launcher: bool,
    /// What it uses casts its spell, or is a thrown missile that is kept.
    pub kept: bool,
    /// What the weapon counts: charges, else units.
    pub left: Option<u16>,
    /// How many swings of the attack there have been, this one included.
    pub spent: usize,
}

/// What a swing used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Used {
    /// How many of what it shoots or throws are spent.
    pub spends: Option<u16>,
    /// What the weapon counts is used up by the swings of this attack: the
    /// slot has no attacks left for the round and the attack ends at once.
    pub used_up: bool,
}

/// What a swing uses up: a launcher one of the missiles in the other hand every
/// swing, a thrown weapon or a gun one of itself; a weapon that casts its
/// spell, or a thrown missile that is kept, uses nothing.
pub fn uses_up(facts: &Ammunition) -> Used {
    let spends = (facts.ranged && (facts.launcher || !facts.kept)).then_some(1);
    Used {
        spends,
        used_up: spends.is_some()
            && facts.left.is_some_and(|left| usize::from(left) <= facts.spent),
    }
}

/// What the next attack weighs.
#[derive(Clone, Copy, Debug)]
pub struct Hands {
    /// Attacks left in each slot, the one just made counted.
    pub left: EnumMap<WeaponSlot, u32>,
    pub initiative: EnumMap<WeaponSlot, i32>,
    /// The slot the attack was made with.
    pub current: WeaponSlot,
    /// The moment of the round it was made in.
    pub now: u32,
}

/// The next attack: with which slot and at which moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Next {
    pub slot: WeaponSlot,
    pub moment: u32,
}

/// When the attacker's next attack follows, if it has one (retail
/// `ComputeCharacterActionPhase`): with the other slot while it has attacks,
/// unless it has fewer than the current slot and less initiative; at
/// `now + (100 - now) / (attacks left + 1)` when the next slot is the current
/// one or the other has no more attacks than it, otherwise at `now`; changing
/// hands adds the current slot's initiative less the new slot's; held between
/// `now` and 100.
pub fn next_attack(hands: &Hands) -> Option<Next> {
    let current = hands.current;
    let other = current.other();
    let (here, there) = (hands.left[current], hands.left[other]);
    let slot =
        if there != 0 && !(there < here && hands.initiative[other] < hands.initiative[current]) {
            other
        } else {
            current
        };
    let left = hands.left[slot];
    let now = hands.now;
    let phase = if left == 0 {
        return None;
    } else if slot == current || there <= here {
        now + (100 - now) / (left + 1)
    } else {
        now
    };
    let switch =
        if slot != current { hands.initiative[current] - hands.initiative[slot] } else { 0 };
    Some(Next { slot, moment: (phase as i32 + switch).clamp(now as i32, 100) as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(mode: AttackMode, weight: Option<u16>) -> Cost {
        Cost {
            tires: true,
            mode,
            weight,
            berserk: false,
            diseased: false,
            load: Some(Load::None),
            slowed: false,
            hasted: false,
        }
    }

    fn spent(facts: &Cost, seed: u64) -> u32 {
        swing_stamina(&mut GameRng::seeded(seed), facts)
    }

    #[test]
    fn slowed_takes_a_quarter_off_hasted_adds_a_quarter_and_slowed_wins() {
        let eight = Cost { berserk: true, diseased: true, ..cost(AttackMode::Swing, Some(0)) };
        assert_eq!(spent(&Cost { slowed: true, ..eight }, 0), 6); // 8 - 25 * 8 / 100
        assert_eq!(spent(&Cost { hasted: true, ..eight }, 0), 10);
        assert_eq!(spent(&Cost { slowed: true, hasted: true, ..eight }, 0), 6);
        // With a heavy load: 100 - 25 = 75 percent, 8 + 6 = 14.
        let heavy = Cost { load: Some(Load::Heavy), ..eight };
        assert_eq!(spent(&Cost { slowed: true, ..heavy }, 0), 14);
        assert_eq!(spent(&Cost { hasted: true, ..heavy }, 0), 18);
    }

    fn ammunition() -> Ammunition {
        Ammunition { ranged: true, launcher: false, kept: false, left: Some(3), spent: 1 }
    }

    #[test]
    fn a_launcher_uses_a_missile_of_the_other_hand_even_if_those_are_kept() {
        let launcher = Ammunition { launcher: true, kept: true, ..ammunition() };
        assert_eq!(uses_up(&launcher).spends, Some(1));
    }

    #[test]
    fn when_the_swings_have_used_up_all_it_counts_the_attack_ends() {
        // Three left, one spent: not yet; two spent: not yet; three: used up.
        for (spent, used_up) in [(1, false), (2, false), (3, true), (4, true)] {
            let facts = Ammunition { spent, ..ammunition() };
            assert_eq!(uses_up(&facts), Used { spends: Some(1), used_up }, "{spent} spent");
        }
        // Nothing it counts: never used up.
        let uncounted = Ammunition { left: None, spent: 9, ..ammunition() };
        assert_eq!(uses_up(&uncounted), Used { spends: Some(1), used_up: false });
    }

    fn hands(
        (primary, secondary): (u32, u32),
        initiative: (i32, i32),
        current: WeaponSlot,
        now: u32,
    ) -> Hands {
        Hands {
            left: EnumMap::from_fn(|slot| match slot {
                WeaponSlot::Primary => primary,
                WeaponSlot::Secondary => secondary,
            }),
            initiative: EnumMap::from_fn(|slot| match slot {
                WeaponSlot::Primary => initiative.0,
                WeaponSlot::Secondary => initiative.1,
            }),
            current,
            now,
        }
    }

    #[test]
    fn it_changes_hands_when_the_other_has_attacks_and_not_fewer_with_less_initiative() {
        use WeaponSlot::*;
        // Equal attacks (1 and 1): the other, at 40 + (100 - 40) / (1 + 1) = 70;
        // changing hands adds 10 - 4 = 6 for the initiative: 76.
        let next = next_attack(&hands((1, 1), (10, 4), Primary, 40));
        assert_eq!(next, Some(Next { slot: Secondary, moment: 76 }));
        // Fewer attacks in the other (1 against 2) but not less initiative
        // (4 against 4): still the other, 40 + 60 / 2 = 70, no initiative to add.
        let next = next_attack(&hands((2, 1), (4, 4), Primary, 40));
        assert_eq!(next, Some(Next { slot: Secondary, moment: 70 }));
        // Fewer attacks and less initiative: the current hand again,
        // 40 + 60 / (2 + 1) = 60.
        let next = next_attack(&hands((2, 1), (10, 4), Primary, 40));
        assert_eq!(next, Some(Next { slot: Primary, moment: 60 }));
        // The other has attacks, the current has none: the other, and since it
        // has more than the current the moment is now.
        let next = next_attack(&hands((0, 2), (0, 0), Primary, 40));
        assert_eq!(next, Some(Next { slot: Secondary, moment: 40 }));
    }

    #[test]
    fn the_moment_is_held_between_now_and_100() {
        use WeaponSlot::*;
        // Moment now (40) plus an initiative of 0 - 20 = -20 is 20: held to 40.
        let low = next_attack(&hands((1, 3), (0, 20), Primary, 40));
        assert_eq!(low, Some(Next { slot: Secondary, moment: 40 }));
        // 40 + 80 = 120: held to 100.
        let high = next_attack(&hands((1, 3), (80, 0), Primary, 40));
        assert_eq!(high, Some(Next { slot: Secondary, moment: 100 }));
        // The same hand at moment 100: 100 + 0 / 2 = 100.
        let late = next_attack(&hands((1, 0), (0, 0), Primary, 100));
        assert_eq!(late, Some(Next { slot: Primary, moment: 100 }));
    }
}
