//! What a swing costs and uses up, and the attacker's next attack
//! (`docs/rules/attack.md` sections 6 and 7).

use enum_map::EnumMap;

use crate::content::items::AttackMode;
use crate::game::inventory::WeaponSlot;

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

/// Whether `swings` swings have used up all a weapon counts.
pub fn runs_out(left: Option<u16>, swings: usize) -> bool {
    left.is_some_and(|left| usize::from(left) <= swings)
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
