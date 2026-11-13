//! What a swing costs and uses up, and the attacker's next attack
//! (`docs/rules/attack.md` sections 6 and 7).

use bevy::prelude::{Entity, Query};
use enum_map::EnumMap;

use super::{Attack, FighterReadOnlyItem};
use crate::content::items::AttackMode;
use crate::game::actors::Arm;
use crate::game::characters::Condition;
use crate::game::inventory::WeaponSlot;
use crate::game::items::{Items, Missile, Weapon};
use crate::game::rng::GameRng;

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
/// An arm that does not tire, and one who carries nothing weighed, pay
/// nothing.
pub fn swing_stamina(rng: &mut GameRng, attack: &Attack, me: &FighterReadOnlyItem) -> u32 {
    let Some(burden) = me.body.burden.filter(|_| attack.arm.tires) else { return 0 };
    let (below, add) = stamina_draw(attack.mode, attack.arm.weight);
    let cost = (rng.below(below) + add)
        * if attack.berserk { 2 } else { 1 }
        * if me.has(Condition::Diseased) { 2 } else { 1 };
    burden.load.scale_fatigue(cost, me.has(Condition::Slowed), me.body.attacks.hasted > 0.0)
}

/// What a swing used of what is shot or thrown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Used {
    /// The item one of which is spent.
    pub spends: Option<Entity>,
    /// What the weapon counts is used up by the swings of this attack: the
    /// slot has no attacks left for the round and the attack ends at once.
    pub used_up: bool,
}

/// Whether `swings` swings have used up all a weapon counts.
pub fn runs_out(left: Option<u16>, swings: usize) -> bool {
    left.is_some_and(|left| usize::from(left) <= swings)
}

/// What the latest of `swings` swings of a missile weapon uses up: a launcher
/// one of the missiles in the other hand every swing, a thrown weapon or a gun
/// one of itself; a weapon that casts its spell, or a thrown missile that is
/// kept, uses nothing, and neither does a close weapon.
pub fn uses_up(
    arm: &Arm,
    actor: Entity,
    swings: usize,
    held: &Items,
    strikers: &Query<(Option<&Weapon>, Option<&Missile>)>,
) -> Used {
    let nothing = Used { spends: None, used_up: false };
    if !arm.ranged || arm.weapon.is_none() {
        return nothing;
    }
    let slot = if arm.loaded.is_some() { arm.slot.other() } else { arm.slot };
    let Some(item) = held.worn(actor, slot.equip()) else { return nothing };
    let kept = strikers.get(item).is_ok_and(|(weapon, missile)| {
        weapon.is_some_and(|weapon| weapon.casts) || missile.is_some_and(|missile| missile.kept)
    });
    if kept && arm.loaded.is_none() {
        return nothing;
    }
    Used { spends: Some(item), used_up: runs_out(held.charges(item).or(held.count(item)), swings) }
}

/// The next attack: with which slot and at which moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Next {
    pub slot: WeaponSlot,
    pub moment: u32,
}

/// When the attacker's next attack follows, if it has one (retail
/// `ComputeCharacterActionPhase`), by the attacks `left` in each slot (the
/// one just made counted) and each slot's initiative: with the other slot
/// while it has attacks, unless it has fewer than the `current` slot and less
/// initiative; at `now + (100 - now) / (attacks left + 1)` when the next slot
/// is the current one or the other has no more attacks than it, otherwise at
/// `now`; changing hands adds the current slot's initiative less the new
/// slot's; held between `now` and 100.
pub fn next_attack(
    left: EnumMap<WeaponSlot, u32>,
    initiative: EnumMap<WeaponSlot, i32>,
    current: WeaponSlot,
    now: u32,
) -> Option<Next> {
    let other = current.other();
    let (here, there) = (left[current], left[other]);
    let slot = if there != 0 && !(there < here && initiative[other] < initiative[current]) {
        other
    } else {
        current
    };
    let phase = if left[slot] == 0 {
        return None;
    } else if slot == current || there <= here {
        now + (100 - now) / (left[slot] + 1)
    } else {
        now
    };
    let switch = if slot != current { initiative[current] - initiative[slot] } else { 0 };
    Some(Next { slot, moment: (phase as i32 + switch).clamp(now as i32, 100) as u32 })
}

#[cfg(test)]
mod tests {
    use super::super::arena::{arena, arm, duel, party, swing};
    use super::*;
    use crate::game::actors::Attacks;
    use crate::game::characters::{Lasting, Load};
    use crate::game::items::Burden;
    use bevy::prelude::World;

    #[test]
    fn slowed_takes_a_quarter_off_hasted_adds_a_quarter_and_slowed_wins() {
        let mut world = arena();
        let (_, members) = party(&mut world, 1);
        let member = members[0];
        // A weapon of no weight costs 2, doubled by a Berserk order and again
        // by disease: 8.
        let mut light = arm(80);
        (light.tires, light.weight) = (true, Some(0));
        let spent = |world: &mut World, load| {
            world.entity_mut(member).insert(Burden { weight: 0, load });
            duel(world, (member, member), |me, _| {
                let fury = Attack { berserk: true, ..swing(&light) };
                swing_stamina(&mut GameRng::seeded(0), &fury, me)
            })
        };
        let afflict = |world: &mut World, condition| {
            world
                .get_mut::<crate::game::characters::Conditions>(member)
                .unwrap()
                .afflict(condition, Lasting::Rounds(9));
        };
        afflict(&mut world, Condition::Diseased);
        assert_eq!(spent(&mut world, Load::None), 8);
        world.get_mut::<Attacks>(member).unwrap().hasted = 0.1;
        assert_eq!(spent(&mut world, Load::None), 10);
        // With a heavy load: 100 + 25 percent, 8 + 10.
        assert_eq!(spent(&mut world, Load::Heavy), 18);
        // Slowed wins over hasted: 8 - 25 * 8 / 100; heavy, 75 percent, 8 + 6.
        afflict(&mut world, Condition::Slowed);
        assert_eq!(spent(&mut world, Load::None), 6);
        assert_eq!(spent(&mut world, Load::Heavy), 14);
    }

    #[test]
    fn when_the_swings_have_used_up_all_it_counts_the_attack_ends() {
        // Three left: one or two swings do not use it up, three do.
        assert_eq!(
            [1, 2, 3, 4].map(|swings| runs_out(Some(3), swings)),
            [false, false, true, true]
        );
        // Nothing it counts: never used up.
        assert!(!runs_out(None, 9));
    }

    fn next(
        (primary, secondary): (u32, u32),
        initiative: (i32, i32),
        current: WeaponSlot,
        now: u32,
    ) -> Option<Next> {
        fn by_slot<T: Copy>(main: T, off: T) -> EnumMap<WeaponSlot, T> {
            EnumMap::from_fn(|slot| if slot == WeaponSlot::Primary { main } else { off })
        }
        next_attack(by_slot(primary, secondary), by_slot(initiative.0, initiative.1), current, now)
    }

    #[test]
    fn it_changes_hands_when_the_other_has_attacks_and_not_fewer_with_less_initiative() {
        use WeaponSlot::*;
        // Equal attacks (1 and 1): the other, at 40 + (100 - 40) / (1 + 1) = 70;
        // changing hands adds 10 - 4 = 6 for the initiative: 76.
        assert_eq!(next((1, 1), (10, 4), Primary, 40), Some(Next { slot: Secondary, moment: 76 }));
        // Fewer attacks in the other (1 against 2) but not less initiative
        // (4 against 4): still the other, 40 + 60 / 2 = 70, no initiative to add.
        assert_eq!(next((2, 1), (4, 4), Primary, 40), Some(Next { slot: Secondary, moment: 70 }));
        // Fewer attacks and less initiative: the current hand again,
        // 40 + 60 / (2 + 1) = 60.
        assert_eq!(next((2, 1), (10, 4), Primary, 40), Some(Next { slot: Primary, moment: 60 }));
        // The other has attacks, the current has none: the other, and since it
        // has more than the current the moment is now.
        assert_eq!(next((0, 2), (0, 0), Primary, 40), Some(Next { slot: Secondary, moment: 40 }));
    }

    #[test]
    fn the_moment_is_held_between_now_and_100() {
        use WeaponSlot::*;
        // Moment now (40) plus an initiative of 0 - 20 = -20 is 20: held to 40.
        assert_eq!(next((1, 3), (0, 20), Primary, 40), Some(Next { slot: Secondary, moment: 40 }));
        // 40 + 80 = 120: held to 100.
        assert_eq!(next((1, 3), (80, 0), Primary, 40), Some(Next { slot: Secondary, moment: 100 }));
        // The same hand at moment 100: 100 + 0 / 2 = 100.
        assert_eq!(next((1, 0), (0, 0), Primary, 100), Some(Next { slot: Primary, moment: 100 }));
    }
}
