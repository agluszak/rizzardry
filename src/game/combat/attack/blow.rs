//! The blow that lands (`docs/rules/attack.md` section 5) and Razor Cloak's
//! answer (section 7).

use super::{Surprise, keeps_distance, reduce, roll_dice};
use crate::content::characters::Skill;
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice};
use crate::content::species::MonsterKind;
use crate::game::characters::scale_by_level;
use crate::game::hits::Effect;
use crate::game::hits::definitions::magnitude;
use crate::game::magic::Strength;
use crate::game::rng::GameRng;

/// What a member's training adds to a blow that got through armour.
#[derive(Clone, Copy, Debug)]
pub struct Training {
    /// The points in the arm's critical skill, if it has one.
    pub critical: Option<u32>,
    /// The profession level of a member with the Knock Out trait, by which its
    /// chance is `5` scaled: 3 at level 0 up to 5 from level 20, whole.
    pub knock_out: Option<u8>,
}

/// What the damage of a blow weighs.
#[derive(Clone, Copy, Debug)]
pub struct Blowing<'a> {
    /// The arm's dice for the mode, a punch's and a kick's their own.
    pub dice: Dice,
    pub percent: i32,
    pub mode: AttackMode,
    pub surprise: Surprise,
    /// The attacker's backstab chance.
    pub backstab: i32,
    /// The victim is on the move, and is defending.
    pub moving: bool,
    pub defending: bool,
    /// The monster kinds the arm slays, and the victim's kind.
    pub slays: &'a [MonsterKind],
    pub kind: Option<MonsterKind>,
    /// The victim's damage reduction in percent.
    pub reduction: i32,
    /// The blow went astray: its damage is limited, and nothing trains.
    pub fumbled: bool,
    pub victim_health: u32,
    pub effects: AttackEffects,
    /// A member's training; a monster has none.
    pub training: Option<Training>,
}

/// What a blow does: its damage, and the weapon's effects with what training
/// added to their chances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Struck {
    pub damage: u32,
    pub effects: AttackEffects,
}

/// The most one blow that went astray takes off its victim: a fifth of its
/// health, at least one (retail `CapAttackDamageByTargetHealth`).
pub fn fumble_cap(damage: u32, health: u32) -> u32 {
    damage.min((health / 5).max(1))
}

/// The extra dice a backstab rolls for a draw below 100 (retail
/// `RollBackstabExtraDice`): one if `roll + 15 < chance`, one more if
/// `roll + 75 < chance`, one more if `roll + 95 < chance`.
pub fn backstab_extra(roll: i32, chance: i32) -> u32 {
    [15, 75, 95].iter().take_while(|&&over| roll + over < chance).count() as u32
}

/// Extra dice a backstab rolls, up to three with a high enough chance.
pub fn backstab_dice(rng: &mut GameRng, chance: i32) -> u32 {
    backstab_extra(rng.below(100) as i32, chance)
}

/// The kill chance training adds to a blow for a draw below 25: a point per 25
/// of `skill` and one more with chance `(skill mod 25) / 25`.
pub fn critical_bonus(skill: u32, roll: u32) -> u32 {
    skill / 25 + u32::from(roll < skill % 25)
}

/// The kill chance training adds to a blow.
pub fn critical_chance(rng: &mut GameRng, skill: u32) -> u32 {
    critical_bonus(skill, rng.below(25))
}

/// How many times the arm's dice are rolled (retail
/// `ResolveCharacterAttackDamage`): 2 for a close blow that is unaware or at a
/// victim on the move, otherwise 1; the backstab's `extra` for a natural close
/// blow; one more for berserk; one less if more than one, the victim is
/// defending and the blow is not unaware; one more for each slay that is the
/// victim's kind.
pub fn dice_to_roll(facts: &Blowing, backstab: u32) -> u32 {
    let close = !keeps_distance(facts.mode);
    let mut count = if (facts.surprise.unaware || facts.moving) && close { 2 } else { 1 };
    if facts.surprise.natural && close {
        count += backstab;
    }
    if facts.mode == AttackMode::Berserk {
        count += 1;
    }
    if count > 1 && facts.defending && !facts.surprise.unaware {
        count -= 1;
    }
    if let Some(kind) = facts.kind {
        count += facts.slays.iter().filter(|&&slain| slain == kind).count() as u32;
    }
    count
}

/// The damage of dice rolled: raised by the arm's percent, at least 1.
pub fn damage_of(percent: i32, rolled: u32) -> u32 {
    (((100 + percent) * rolled as i32 + 50) / 100).max(1) as u32
}

/// The damage of a blow that got through armour, and the effects it carries: the
/// dice rolled `dice_to_roll` times (the base added to each), raised by the
/// percent, cut by the victim's reduction, limited if it went astray; for a
/// blow not astray, a member's training raises the Kill chance (by the arm's
/// critical skill) and the Knock Out chance, each held to 100.
pub fn blow(rng: &mut GameRng, facts: &Blowing) -> Struck {
    let backstab = if facts.surprise.natural && !keeps_distance(facts.mode) {
        backstab_dice(rng, facts.backstab)
    } else {
        0
    };
    let rolled = roll_dice(rng, facts.dice, dice_to_roll(facts, backstab));
    let mut damage = reduce(damage_of(facts.percent, rolled) as i32, facts.reduction);
    if facts.fumbled && damage > 0 {
        damage = fumble_cap(damage, facts.victim_health);
    }
    let mut effects = facts.effects;
    if !facts.fumbled
        && let Some(training) = facts.training
    {
        if let Some(skill) = training.critical {
            let kill = &mut effects.chances[AttackEffect::Kill];
            *kill = (*kill + critical_chance(rng, skill)).min(100);
        }
        if let Some(level) = training.knock_out {
            let knock_out = &mut effects.chances[AttackEffect::KnockOut];
            *knock_out = (*knock_out + scale_by_level(level, 5.0) as u32).min(100);
        }
    }
    Struck { damage, effects }
}

/// What Razor Cloak's answer weighs.
#[derive(Clone, Copy, Debug)]
pub struct Riposte {
    /// The spell's dice.
    pub dice: Option<Dice>,
    /// The power the cloak had before the point was spent.
    pub power: u32,
    /// The attacker's own damage reduction in percent.
    pub reduction: i32,
}

/// The damage Razor Cloak answers a close blow with: the spell's dice rolled
/// `count * power` times, no boost, less the attacker's reduction.
pub fn riposte(rng: &mut GameRng, facts: &Riposte) -> u32 {
    let strength = Strength { power: facts.power, against: 0, boost: 0 };
    reduce(magnitude(rng, facts.dice, strength) as i32, facts.reduction)
}

/// Whether a blow lands as a hit: it did more than 0 damage.
pub fn lands(damage: u32) -> bool {
    damage > 0
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

/// The skills a blow that did damage and was not astray practises: a close arm
/// Critical Strike and Power Strike; a ranged one Eagle Eye, and Critical
/// Strike besides for a thrower with Thrown Criticals.
pub fn practised_by_blow(ranged: bool, thrown_criticals: bool) -> &'static [Skill] {
    match (ranged, thrown_criticals) {
        (false, _) => &[Skill::CriticalStrike, Skill::PowerStrike],
        (true, true) => &[Skill::EagleEye, Skill::CriticalStrike],
        (true, false) => &[Skill::EagleEye],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::Entity;

    /// Dice that roll 1 every time: a blow's damage is how many it rolls.
    const ONE: Dice = Dice { count: 1, sides: 1, base: 0 };

    fn blowing() -> Blowing<'static> {
        Blowing {
            dice: ONE,
            percent: 0,
            mode: AttackMode::Swing,
            surprise: Surprise::default(),
            backstab: 0,
            moving: false,
            defending: false,
            slays: &[],
            kind: None,
            reduction: 0,
            fumbled: false,
            victim_health: 100,
            effects: AttackEffects::default(),
            training: None,
        }
    }

    fn count(facts: &Blowing) -> u32 {
        dice_to_roll(facts, 0)
    }

    fn damage(facts: &Blowing) -> u32 {
        blow(&mut GameRng::seeded(0), facts).damage
    }

    const UNAWARE: Surprise = Surprise { unaware: true, natural: false };
    const NATURAL: Surprise = Surprise { unaware: false, natural: true };

    #[test]
    fn a_plain_blow_rolls_the_dice_once() {
        assert_eq!(count(&blowing()), 1);
        assert_eq!(damage(&blowing()), 1);
    }

    #[test]
    fn a_close_blow_unaware_or_at_a_victim_on_the_move_rolls_two() {
        assert_eq!(count(&Blowing { surprise: UNAWARE, ..blowing() }), 2);
        assert_eq!(count(&Blowing { moving: true, ..blowing() }), 2);
        // Not both: 2, not 3.
        assert_eq!(count(&Blowing { surprise: UNAWARE, moving: true, ..blowing() }), 2);
        // Not throws, lashes and shots.
        for mode in [AttackMode::Throw, AttackMode::Lash, AttackMode::Shoot] {
            assert_eq!(count(&Blowing { surprise: UNAWARE, moving: true, mode, ..blowing() }), 1);
        }
    }

    #[test]
    fn a_natural_close_blow_adds_the_backstab_dice() {
        let natural = Blowing { surprise: NATURAL, ..blowing() };
        assert_eq!(dice_to_roll(&natural, 0), 1);
        assert_eq!(dice_to_roll(&natural, 2), 3);
        // Unaware as well: 2 + 3.
        let both = Blowing { surprise: Surprise { unaware: true, natural: true }, ..blowing() };
        assert_eq!(dice_to_roll(&both, 3), 5);
        // Not on a shot, and not if not natural.
        let shot = Blowing { surprise: NATURAL, mode: AttackMode::Shoot, ..blowing() };
        assert_eq!(dice_to_roll(&shot, 3), 1);
        assert_eq!(dice_to_roll(&blowing(), 3), 1);
    }

    #[test]
    fn berserk_rolls_one_more() {
        assert_eq!(count(&Blowing { mode: AttackMode::Berserk, ..blowing() }), 2);
    }

    #[test]
    fn a_defending_victim_takes_a_die_off_unless_the_blow_was_unaware() {
        // On the move and defending: 2 - 1.
        assert_eq!(count(&Blowing { moving: true, defending: true, ..blowing() }), 1);
        // Berserk and defending: 1 + 1 - 1.
        assert_eq!(count(&Blowing { mode: AttackMode::Berserk, defending: true, ..blowing() }), 1);
        // Unaware: kept.
        assert_eq!(count(&Blowing { surprise: UNAWARE, defending: true, ..blowing() }), 2);
        // One die only: nothing to take.
        assert_eq!(count(&Blowing { defending: true, ..blowing() }), 1);
    }

    #[test]
    fn each_slay_that_is_the_victims_kind_adds_a_die() {
        let slays = [MonsterKind::Undead, MonsterKind::Demon, MonsterKind::Undead];
        let facts = |kind| Blowing { slays: &slays, kind, ..blowing() };
        assert_eq!(count(&facts(Some(MonsterKind::Undead))), 3);
        assert_eq!(count(&facts(Some(MonsterKind::Demon))), 2);
        assert_eq!(count(&facts(Some(MonsterKind::Plant))), 1);
        // A member has no kind.
        assert_eq!(count(&facts(None)), 1);
    }

    #[test]
    fn the_damage_sums_whole_rolls_with_the_base_in_each() {
        // 1d1+2 rolls 3, twice: 6.
        let facts =
            Blowing { dice: Dice { count: 1, sides: 1, base: 2 }, moving: true, ..blowing() };
        assert_eq!(damage(&facts), 6);
    }

    #[test]
    fn the_damage_is_raised_by_the_percent_and_is_at_least_one() {
        // max(1, ((100 + p) * rolled + 50) / 100).
        let rolled = Dice { count: 7, sides: 1, base: 0 };
        let with = |percent| Blowing { dice: rolled, percent, ..blowing() };
        assert_eq!(damage(&with(0)), 7); // 750 / 100
        assert_eq!(damage(&with(50)), 11); // (150 * 7 + 50) / 100 = 1100 / 100
        assert_eq!(damage(&with(-50)), 4); // (50 * 7 + 50) / 100 = 400 / 100
        assert_eq!(damage(&with(-100)), 1); // 50 / 100 = 0, at least 1
        assert_eq!(damage_of(0, 0), 1);
    }

    #[test]
    fn the_victims_reduction_cuts_the_damage_and_may_leave_none() {
        let rolled = Dice { count: 50, sides: 1, base: 0 };
        let with = |reduction| Blowing { dice: rolled, reduction, ..blowing() };
        // (100 - r) * 50 + 50 over 100.
        assert_eq!(damage(&with(50)), 25);
        assert_eq!(damage(&with(100)), 0);
        // One point cut by half: (50 * 1 + 50) / 100 = 1; by 51 percent, 0.
        assert_eq!(damage(&Blowing { reduction: 51, ..blowing() }), 0);
        assert_eq!(damage(&Blowing { reduction: 50, ..blowing() }), 1);
    }

    #[test]
    fn a_blow_gone_astray_is_limited_to_a_fifth_of_the_victims_health_at_least_one() {
        let rolled = Dice { count: 50, sides: 1, base: 0 };
        let astray = Blowing { dice: rolled, fumbled: true, victim_health: 20, ..blowing() };
        assert_eq!(damage(&astray), 4);
        assert_eq!(damage(&Blowing { fumbled: false, ..astray }), 50);
        // A fifth of 3 is 0: one.
        assert_eq!(damage(&Blowing { victim_health: 3, ..astray }), 1);
        // Less than the limit is unchanged.
        let small = Blowing { dice: Dice { count: 2, sides: 1, base: 0 }, ..astray };
        assert_eq!(damage(&small), 2);
        assert_eq!(fumble_cap(30, 100), 20);
        assert_eq!(fumble_cap(30, 4), 1);
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
    fn a_natural_blow_rolls_its_backstab_dice_into_the_damage() {
        // A chance beyond any draw: all three extra, 1 + 3 dice.
        let sure = Blowing { surprise: NATURAL, backstab: 1000, ..blowing() };
        assert_eq!(damage(&sure), 4);
        // None: just the one.
        assert_eq!(damage(&Blowing { backstab: 0, ..sure }), 1);
        // Not natural: none whatever the chance.
        assert_eq!(damage(&Blowing { surprise: Surprise::default(), ..sure }), 1);
    }

    #[test]
    fn training_adds_a_point_per_25_of_the_critical_skill_and_a_chance_at_one_more() {
        // Multiple of 25: certain. 50 / 25 = 2.
        assert_eq!(critical_bonus(50, 0), 2);
        assert_eq!(critical_bonus(25, 24), 1);
        // 60: 2, and 1 more when the draw below 25 is below 60 mod 25 = 10.
        assert_eq!(critical_bonus(60, 9), 3);
        assert_eq!(critical_bonus(60, 10), 2);
        assert_eq!(critical_bonus(24, 23), 1);
        assert_eq!(critical_bonus(24, 24), 0);
        assert_eq!(critical_bonus(0, 0), 0);
        for seed in 0..50 {
            assert!((2..=3).contains(&critical_chance(&mut GameRng::seeded(seed), 60)));
            assert_eq!(critical_chance(&mut GameRng::seeded(seed), 50), 2);
        }
    }

    fn with_chances(kill: u32, knock_out: u32) -> AttackEffects {
        let mut effects = AttackEffects::default();
        effects.chances[AttackEffect::Kill] = kill;
        effects.chances[AttackEffect::KnockOut] = knock_out;
        effects
    }

    fn effects_of(facts: &Blowing) -> (u32, u32) {
        let chances = blow(&mut GameRng::seeded(0), facts).effects.chances;
        (chances[AttackEffect::Kill], chances[AttackEffect::KnockOut])
    }

    #[test]
    fn a_members_training_raises_the_kill_and_knock_out_chances_to_at_most_100() {
        let trained = Blowing {
            effects: with_chances(10, 20),
            training: Some(Training { critical: Some(50), knock_out: Some(10) }),
            ..blowing()
        };
        // Kill 10 + 50 / 25; Knock Out 20 + 5 * (10 * 2 + 60) / 100 = 20 + 4.
        assert_eq!(effects_of(&trained), (12, 24));
        let nearly = Blowing { effects: with_chances(99, 98), ..trained };
        assert_eq!(effects_of(&nearly), (100, 100));
        // No critical skill, or no trait: that chance is as the weapon has it.
        let skill_less = Training { critical: None, knock_out: Some(10) };
        assert_eq!(effects_of(&Blowing { training: Some(skill_less), ..trained }), (10, 24));
        let untrained = Training { critical: Some(50), knock_out: None };
        assert_eq!(effects_of(&Blowing { training: Some(untrained), ..trained }), (12, 20));
        // A monster has no training.
        assert_eq!(effects_of(&Blowing { training: None, ..trained }), (10, 20));
    }

    #[test]
    fn the_knock_out_trait_adds_5_scaled_by_profession_level() {
        // 5 * (level * 2 + 60) / 100, whole: level 0 is 3, level 10 is 4, from 20 it is 5.
        for (level, bonus) in [(0, 3), (10, 4), (20, 5), (21, 5), (40, 5)] {
            let facts = Blowing {
                training: Some(Training { critical: None, knock_out: Some(level) }),
                ..blowing()
            };
            assert_eq!(effects_of(&facts), (0, bonus), "level {level}");
        }
    }

    #[test]
    fn only_damage_lands_a_blow() {
        assert!(lands(1));
        assert!(!lands(0));
    }

    #[test]
    fn a_blow_gone_astray_is_not_trained() {
        let astray = Blowing {
            effects: with_chances(10, 20),
            training: Some(Training { critical: Some(50), knock_out: Some(10) }),
            fumbled: true,
            ..blowing()
        };
        assert_eq!(effects_of(&astray), (10, 20));
    }

    #[test]
    fn razor_cloak_rolls_the_spells_dice_count_times_power_less_the_attackers_reduction() {
        // 1d1 at power 3 is three dice of 1.
        let cloak = Riposte { dice: Some(ONE), power: 3, reduction: 0 };
        assert_eq!(riposte(&mut GameRng::seeded(0), &cloak), 3);
        // (50 * 3 + 50) / 100 = 2.
        assert_eq!(riposte(&mut GameRng::seeded(0), &Riposte { reduction: 50, ..cloak }), 2);
        assert_eq!(riposte(&mut GameRng::seeded(0), &Riposte { reduction: 100, ..cloak }), 0);
        assert_eq!(riposte(&mut GameRng::seeded(0), &Riposte { dice: None, ..cloak }), 0);
        // The base is not multiplied: 0d1+5 rolled count 0 times power 3 = 0 dice, +5.
        let flat = Riposte { dice: Some(Dice { count: 0, sides: 1, base: 5 }), ..cloak };
        assert_eq!(riposte(&mut GameRng::seeded(0), &flat), 5);
    }

    #[test]
    fn what_a_blow_did_is_told_as_the_health_it_took_and_the_rest() {
        let victim = Entity::PLACEHOLDER;
        let effects = [
            Effect::Damaged { victim, amount: 4 },
            Effect::Nothing { victim },
            Effect::Damaged { victim, amount: 3 },
        ];
        assert_eq!(dealt(effects), Dealt { damage: 7, rest: vec![Effect::Nothing { victim }] });
        assert_eq!(dealt([]), Dealt { damage: 0, rest: Vec::new() });
    }

    #[test]
    fn a_blow_that_did_damage_practises_by_the_arm() {
        assert_eq!(practised_by_blow(false, false), [Skill::CriticalStrike, Skill::PowerStrike]);
        assert_eq!(practised_by_blow(false, true), [Skill::CriticalStrike, Skill::PowerStrike]);
        assert_eq!(practised_by_blow(true, false), [Skill::EagleEye]);
        assert_eq!(practised_by_blow(true, true), [Skill::EagleEye, Skill::CriticalStrike]);
    }
}
