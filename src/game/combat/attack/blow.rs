//! The blow that lands (`docs/rules/attack.md` section 5) and Razor Cloak's
//! answer (section 7).

use super::reduce;
use crate::content::items::Dice;
use crate::game::hits::Effect;
use crate::game::hits::definitions::magnitude;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;

/// The damage of dice rolled: raised by the arm's percent, at least 1.
pub fn damage_of(percent: i32, rolled: u32) -> u32 {
    (((100 + percent) * rolled as i32 + 50) / 100).max(1) as u32
}

/// The extra dice a backstab rolls for a draw below 100 (retail
/// `RollBackstabExtraDice`): one if `roll + 15 < chance`, one more if
/// `roll + 75 < chance`, one more if `roll + 95 < chance`.
pub fn backstab_extra(roll: i32, chance: i32) -> u32 {
    [15, 75, 95].iter().take_while(|&&over| roll + over < chance).count() as u32
}

/// The damage Razor Cloak answers a close blow with: the spell's dice rolled
/// `count * power` times, at the power the cloak had before the point was
/// spent, no boost, less the attacker's own reduction.
pub fn riposte(rng: &mut GameRng, dice: Option<Dice>, power: u32, reduction: i32) -> u32 {
    reduce(magnitude(rng, dice, Strength { power, against: 0, boost: 0 }) as i32, reduction)
}

/// What a landed blow did, told apart: the health it took and what else it did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dealt {
    pub damage: u32,
    pub rest: Vec<Effect>,
}

/// Sort what a blow did into the health it took (after Guardian Angel) and the
/// rest.
pub fn dealt(effects: impl IntoIterator<Item = Effect>) -> Dealt {
    let (damage, rest): (Vec<Effect>, Vec<Effect>) =
        effects.into_iter().partition(|effect| matches!(effect, Effect::Damaged { .. }));
    let damage = damage
        .iter()
        .map(|effect| match effect {
            Effect::Damaged { amount, .. } => *amount,
            _ => 0,
        })
        .sum();
    Dealt { damage, rest }
}
