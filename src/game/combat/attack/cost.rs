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
