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
    let mut damage = reduce(damage_of(arm.percent, rolled) as i32, struck.body.armour.reduction);
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

#[cfg(test)]
mod tests {
    use super::super::arena::{NORMAL, arena, arm, duel, monster, swing};
    use super::*;
    use crate::game::actors::Armour;
    use crate::game::characters::Vitals;
    use crate::game::combat::approach::{CombatMove, MoveGoal};
    use crate::game::combat::attack::Surprise;
    use crate::game::combat::turn::{Action, Ordered};
    use crate::game::world::hostility::Stance;
    use bevy::prelude::{Entity, Vec3, World};

    /// Two monsters facing each other.
    fn pair() -> (World, (Entity, Entity)) {
        let mut world = arena();
        let pair = (
            monster(&mut world, Vec3::ZERO, Stance::Hostile),
            monster(&mut world, Vec3::NEG_Z, Stance::Friendly),
        );
        (world, pair)
    }

    /// The damage of a blow of the arena's arm, which rolls 1 a die: how
    /// many dice it rolled.
    fn damage(world: &mut World, pair: (Entity, Entity), attack: &Attack) -> u32 {
        duel(world, pair, |me, struck| {
            blow(&mut GameRng::seeded(0), attack, me, struck, NORMAL, false).damage
        })
    }

    const UNAWARE: Surprise = Surprise { unaware: true, natural: false };

    fn walk(world: &mut World, actor: Entity) {
        world.entity_mut(actor).insert(CombatMove {
            budget: 1.0,
            moved: 0.0,
            goal: MoveGoal::Party,
        });
    }

    #[test]
    fn a_close_blow_unaware_or_at_a_victim_on_the_move_rolls_two() {
        let (mut world, pair) = pair();
        let arm = arm(80);
        assert_eq!(damage(&mut world, pair, &swing(&arm)), 1);
        let unaware = Attack { surprise: UNAWARE, ..swing(&arm) };
        assert_eq!(damage(&mut world, pair, &unaware), 2);
        walk(&mut world, pair.1);
        assert_eq!(damage(&mut world, pair, &swing(&arm)), 2);
        // Not both: 2, not 3.
        assert_eq!(damage(&mut world, pair, &unaware), 2);
        // Not throws, lashes and shots.
        for mode in [AttackMode::Throw, AttackMode::Lash, AttackMode::Shoot] {
            assert_eq!(damage(&mut world, pair, &Attack { mode, ..unaware }), 1);
        }
    }

    #[test]
    fn a_defending_victim_takes_a_die_off_unless_the_blow_was_unaware() {
        let (mut world, pair) = pair();
        let arm = arm(80);
        world.entity_mut(pair.1).insert(Ordered(Action::Defend));
        // One die only: nothing to take.
        assert_eq!(damage(&mut world, pair, &swing(&arm)), 1);
        // Berserk and defending: 1 + 1 - 1.
        let berserk = Attack { mode: AttackMode::Berserk, ..swing(&arm) };
        assert_eq!(damage(&mut world, pair, &berserk), 1);
        // Unaware: kept.
        assert_eq!(damage(&mut world, pair, &Attack { surprise: UNAWARE, ..swing(&arm) }), 2);
        // On the move and defending: 2 - 1.
        walk(&mut world, pair.1);
        assert_eq!(damage(&mut world, pair, &swing(&arm)), 1);
    }

    #[test]
    fn the_damage_is_raised_by_the_percent_and_is_at_least_one() {
        // max(1, ((100 + p) * rolled + 50) / 100).
        assert_eq!(damage_of(0, 7), 7); // 750 / 100
        assert_eq!(damage_of(50, 7), 11); // (150 * 7 + 50) / 100 = 1100 / 100
        assert_eq!(damage_of(-50, 7), 4); // (50 * 7 + 50) / 100 = 400 / 100
        assert_eq!(damage_of(-100, 7), 1); // 50 / 100 = 0, at least 1
        assert_eq!(damage_of(0, 0), 1);
    }

    #[test]
    fn a_blow_gone_astray_is_limited_to_a_fifth_of_the_victims_health_at_least_one() {
        let (mut world, pair) = pair();
        let mut heavy = arm(80);
        heavy.dice = Dice { count: 50, sides: 1, base: 0 };
        let astray = |world: &mut World, fumbled| {
            duel(world, pair, |me, struck| {
                blow(&mut GameRng::seeded(0), &swing(&heavy), me, struck, NORMAL, fumbled).damage
            })
        };
        *world.get_mut::<Vitals>(pair.1).unwrap() = Vitals::full(20, 100);
        assert_eq!(astray(&mut world, true), 4);
        assert_eq!(astray(&mut world, false), 50);
        // A fifth of 3 is 0: one.
        *world.get_mut::<Vitals>(pair.1).unwrap() = Vitals::full(3, 100);
        assert_eq!(astray(&mut world, true), 1);
        // What its armour cut to nothing stays nothing.
        world.get_mut::<Armour>(pair.1).unwrap().reduction = 100;
        assert_eq!(astray(&mut world, true), 0);
    }

    #[test]
    fn a_backstab_rolls_a_die_more_for_each_threshold_its_chance_clears() {
        // Thresholds 15, 75 and 95 over the draw. Chance 100: draw 85 clears
        // none (100 is not below 100), 84 clears the first (99).
        assert_eq!(backstab_extra(85, 100), 0);
        assert_eq!(backstab_extra(84, 100), 1);
        // chance 100: draw 24 clears 15 (39) and 75 (99) but not 95 (119).
        assert_eq!(backstab_extra(24, 100), 2);
        // Draw 4: 4 + 95 = 99 < 100 clears all three.
        assert_eq!(backstab_extra(4, 100), 3);
        assert_eq!(backstab_extra(5, 100), 2);
        // Chance 60: draw 44 clears 15 only (59 < 60); 45 clears none.
        assert_eq!(backstab_extra(44, 60), 1);
        assert_eq!(backstab_extra(45, 60), 0);
        assert_eq!(backstab_extra(0, 0), 0);
    }

    #[test]
    fn razor_cloak_rolls_the_spells_dice_count_times_power_less_the_attackers_reduction() {
        let rng = &mut GameRng::seeded(0);
        // 1d1 at power 3 is three dice of 1.
        let one = Some(Dice { count: 1, sides: 1, base: 0 });
        assert_eq!(riposte(rng, one, 3, 0), 3);
        // (50 * 3 + 50) / 100 = 2.
        assert_eq!(riposte(rng, one, 3, 50), 2);
        assert_eq!(riposte(rng, one, 3, 100), 0);
        assert_eq!(riposte(rng, None, 3, 0), 0);
        // The base is not multiplied: 0d1+5 rolled count 0 times power 3 = 0 dice, +5.
        assert_eq!(riposte(rng, Some(Dice { count: 0, sides: 1, base: 5 }), 3, 0), 5);
    }
}
