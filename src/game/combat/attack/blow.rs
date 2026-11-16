//! The blow that lands (`docs/rules/attack.md` section 5) and Razor Cloak's
//! answer (section 7).

use super::{Attack, FighterReadOnlyItem, Setting, keeps_distance, reduce, roll_dice};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::game::characters::scale_by_level;
use crate::game::hits::Effect;
use crate::game::hits::definitions::magnitude;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;

/// What a blow does: its damage, and the weapon's effects with what training
/// added to their chances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Struck {
    pub damage: u32,
    pub effects: AttackEffects,
}

/// How many times the arm's dice are rolled (retail
/// `ResolveCharacterAttackDamage`): 2 for a close blow that is unaware or at a
/// victim on the move, otherwise 1; the backstab's extra dice for a natural
/// close blow; one more for berserk; one less if more than one, the victim is
/// defending and the blow is not unaware; one more for each kind the arm
/// slays that the victim is.
fn dice_to_roll(
    rng: &mut GameRng,
    attack: &Attack,
    me: &FighterReadOnlyItem,
    struck: &FighterReadOnlyItem,
    setting: Setting,
) -> u32 {
    let close = !keeps_distance(attack.mode);
    let surprise = attack.surprise;
    let mut count = if (surprise.unaware || struck.moving(setting)) && close { 2 } else { 1 };
    if surprise.natural && close {
        count += backstab_extra(rng.below(100) as i32, me.body.attacks.backstab);
    }
    if attack.mode == AttackMode::Berserk {
        count += 1;
    }
    if count > 1 && struck.defending() && !surprise.unaware {
        count -= 1;
    }
    if let Some(kind) = struck.body.resistances.kind {
        count += attack.arm.slays.iter().filter(|&&slain| slain == kind).count() as u32;
    }
    count
}

/// The damage of dice rolled: raised by the arm's percent, at least 1.
pub fn damage_of(percent: i32, rolled: u32) -> u32 {
    (((100 + percent) * rolled as i32 + 50) / 100).max(1) as u32
}

/// The damage of a blow that got through armour, and the effects it carries:
/// the arm's dice rolled as often as the blow rolls them (the base added to
/// each), raised by the arm's percent and cut by the victim's reduction. One
/// gone astray takes at most a fifth of its victim's health, at least one
/// (retail `CapAttackDamageByTargetHealth`), and nothing trains it; otherwise
/// the attacker's training raises the Kill chance, by a point per 25 in the
/// arm's critical skill and one more with chance `(skill mod 25) / 25`, and
/// the Knock Out chance, by 5 scaled by the level of one with the trait (3 at
/// level 0 up to 5 from level 20), each held to 100.
pub fn blow(
    rng: &mut GameRng,
    attack: &Attack,
    me: &FighterReadOnlyItem,
    struck: &FighterReadOnlyItem,
    setting: Setting,
    fumbled: bool,
) -> Struck {
    let arm = attack.arm;
    let count = dice_to_roll(rng, attack, me, struck, setting);
    let rolled = roll_dice(rng, arm.rolls(attack.mode), count);
    let mut damage = reduce(damage_of(arm.percent, rolled) as i32, struck.body.reduction.0);
    let mut effects = arm.effects;
    if fumbled {
        if damage > 0 {
            damage = damage.min((struck.body.vitals.health.max / 5).max(1));
        }
        return Struck { damage, effects };
    }
    if let Some(skill) = arm.critical {
        let kill = &mut effects.chances[AttackEffect::Kill];
        *kill = (*kill + skill / 25 + u32::from(rng.below(25) < skill % 25)).min(100);
    }
    if let Some(level) = me.body.attacks.knock_out {
        let knock_out = &mut effects.chances[AttackEffect::KnockOut];
        *knock_out = (*knock_out + scale_by_level(level, 5.0) as u32).min(100);
    }
    Struck { damage, effects }
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
