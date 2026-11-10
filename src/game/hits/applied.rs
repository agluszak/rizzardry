//! Which effects an effect row makes happen to a victim, and the arithmetic of
//! each kind (`docs/rules/casting.md` section 6): whom a row is for, whether it
//! waits on the row before, how long an affliction lasts, what a raising gives
//! back, what heat burns. Each a function of the facts it is about and nothing
//! else; a rule that rolls takes the random-number generator.

use super::definitions::{Blow, EffectKind, EffectSpec, Whom};
use crate::content::items::{AttackEffects, Dice};
use crate::content::species::MonsterKind;
use crate::game::characters::{Condition, Lasting};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// How a row was settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Landed,
    Resisted,
    /// Nothing to act on: the body was dead, gone, or not of the kind.
    Missed,
}

/// The rows of a definition that apply, each with the weapon's chance and
/// poison when it stands for a weapon effect: those for the party as a whole
/// when `party`, else those for bodies; a row only for a monster's blows only
/// from one, a weapon's row only if the blow's weapon has a chance for it.
pub fn rows<'a>(
    effects: &'a [EffectSpec],
    blow: Option<Blow>,
    party: bool,
) -> impl Iterator<Item = EffectSpec> + 'a {
    effects.iter().filter_map(move |spec| {
        if (spec.on == Whom::Party) != party
            || (spec.by_monsters && !blow.is_some_and(|b| b.by_monster))
        {
            return None;
        }
        let Some(effect) = spec.weapon else { return Some(spec.clone()) };
        let weapon: AttackEffects = blow?.effects;
        let chance = weapon.chances[effect];
        let kind = match spec.kind {
            EffectKind::Afflict { condition: Condition::Poisoned, indefinite, .. } => {
                EffectKind::Afflict {
                    condition: Condition::Poisoned,
                    poison: weapon.strength,
                    indefinite,
                }
            }
            ref kind => kind.clone(),
        };
        (chance > 0).then_some(EffectSpec { chance: Some(chance), kind, ..spec.clone() })
    })
}

/// Whether a row is for this victim: not a row for members on a monster, nor a
/// row for monsters on a member, nor a row for given kinds of monster on a body
/// not of the kind.
pub fn is_for(row: &EffectSpec, member: bool, kind: Option<MonsterKind>) -> bool {
    !((row.on == Whom::Members && !member)
        || (row.on == Whom::Monsters && kind.is_none())
        || row.kinds.as_ref().is_some_and(|kinds| !kind.is_some_and(|k| kinds.contains(&k))))
}

/// Whether a row waits on the row before it, which did not come out so.
pub fn waits(row: &EffectSpec, previous: Option<Outcome>) -> bool {
    (row.landed && previous != Some(Outcome::Landed))
        || (row.resisted && previous != Some(Outcome::Resisted))
}

/// Whether a row for the party happens, at its chance.
pub fn happens(rng: &mut GameRng, chance: Option<u32>) -> bool {
    !chance.is_some_and(|chance| rng.below(100) >= chance)
}

/// How long an affliction of `magnitude` lasts: until lifted if the row says
/// so, whatever the magnitude; a magnitude of 1 becomes 2.
pub fn lasting(indefinite: bool, magnitude: u32) -> Lasting {
    match magnitude {
        _ if indefinite => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds),
    }
}

/// How an affliction lays on a victim on `stance`: Turncoat on a neutral
/// monster does nothing.
pub fn afflicts(
    condition: Condition,
    stance: Stance,
    indefinite: bool,
    magnitude: u32,
) -> Option<Lasting> {
    if condition == Condition::Turncoat && stance == Stance::Neutral {
        return None;
    }
    Some(lasting(indefinite, magnitude))
}

/// What harm gives back to a leech: the magnitude the victim's health could
/// bear, before a Guardian Angel takes any (retail `DrainTargetsLife`,
/// `Magic Effects.cpp:1626-1650`).
pub fn bearable(magnitude: u32, health: u32) -> u32 {
    magnitude.min(health)
}

/// What a monster's budget loses of a drain.
pub fn budget_lost(budget: u32, magnitude: u32) -> u32 {
    budget.min(magnitude)
}

/// The health and stamina a dead member lives again with: `(Piety +
/// Vitality) / 4` percent of each maximum, not under 1 each.
pub fn raised(health_max: u32, stamina_max: u32, piety: u32, vitality: u32) -> (u32, u32) {
    let share = (piety + vitality) >> 2;
    ((health_max * share / 100).max(1), (stamina_max * share / 100).max(1))
}

/// The turns a purifying takes off a cloud: the magnitude and the purifier's
/// level.
pub fn purified(magnitude: u32, level: u32) -> u32 {
    magnitude + level
}

/// What a lost-gold row takes of the purse: that percent, at most 100.
pub fn gold_lost(gold: u32, magnitude: u32) -> u32 {
    gold * magnitude.min(100) / 100
}

/// How many of `units` heat burns, each on its own at `chance` percent.
pub fn burnt(rng: &mut GameRng, units: u32, chance: u32) -> u16 {
    (0..units).filter(|_| rng.below(100) < chance).count() as u16
}

/// What a Guardian Angel absorbs when it is laid: its dice rolled, times the
/// power, raised by the boost; nothing for another spell.
pub fn absorbs(
    rng: &mut GameRng,
    absorbing: bool,
    dice: Option<Dice>,
    power: u32,
    boost: u32,
) -> u32 {
    if !absorbing {
        return 0;
    }
    let rolled = dice.map_or(0, |dice| rng.roll(dice)) * power;
    rolled + rolled * boost / 100
}

/// Whether an enchantment changed its bearer: it was laid, or it first spent
/// itself on a slowing and was wholly spent, which changed something all the
/// same.
pub fn enchanted(laid: bool, answers_slowing: bool, turns: u32, magnitude: u32) -> bool {
    laid || (answers_slowing && turns == 0 && magnitude > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::AttackEffect;

    fn on(whom: Whom) -> EffectSpec {
        EffectSpec { on: whom, ..Default::default() }
    }

    /// A row for members is skipped on a monster, a row for monsters on a member;
    /// a row for given kinds on a body not of the kind.
    #[test]
    fn a_row_is_for_those_it_names() {
        let beast = Some(MonsterKind::Beast);
        assert!(is_for(&on(Whom::Everyone), true, None));
        assert!(is_for(&on(Whom::Everyone), false, beast));
        assert!(is_for(&on(Whom::Members), true, None));
        assert!(!is_for(&on(Whom::Members), false, beast));
        assert!(is_for(&on(Whom::Monsters), false, beast));
        assert!(!is_for(&on(Whom::Monsters), true, None));
        let undead = EffectSpec {
            kinds: Some(vec![MonsterKind::Undead, MonsterKind::Demon]),
            ..Default::default()
        };
        assert!(is_for(&undead, false, Some(MonsterKind::Demon)));
        assert!(!is_for(&undead, false, beast));
        assert!(!is_for(&undead, true, None), "a member is of no kind");
    }

    /// A row that waits on one landed needs it landed; on one resisted needs it
    /// resisted; the first row has nothing before it.
    #[test]
    fn a_row_may_wait_on_the_row_before_it() {
        let plain = EffectSpec::default();
        let landed = EffectSpec { landed: true, ..plain.clone() };
        let resisted = EffectSpec { resisted: true, ..plain.clone() };
        for previous in
            [None, Some(Outcome::Landed), Some(Outcome::Resisted), Some(Outcome::Missed)]
        {
            assert!(!waits(&plain, previous));
        }
        assert!(waits(&landed, None) && waits(&landed, Some(Outcome::Resisted)));
        assert!(waits(&landed, Some(Outcome::Missed)));
        assert!(!waits(&landed, Some(Outcome::Landed)));
        assert!(waits(&resisted, None) && waits(&resisted, Some(Outcome::Landed)));
        assert!(!waits(&resisted, Some(Outcome::Resisted)));
    }

    /// Party rows are the party's only for the party, and the rest only for
    /// bodies; a monster-only row waits for a monster's blow; a weapon's row
    /// takes the weapon's chance and, for poison, its strength, and is dropped
    /// without a chance.
    #[test]
    fn a_blows_rows_take_the_weapons_chances() {
        let party =
            EffectSpec { kind: EffectKind::LoseGold, on: Whom::Party, ..Default::default() };
        let poison = EffectSpec {
            kind: EffectKind::Afflict {
                condition: Condition::Poisoned,
                poison: 0,
                indefinite: false,
            },
            weapon: Some(AttackEffect::Poison),
            ..Default::default()
        };
        let mut chances = AttackEffects::default();
        chances.chances[AttackEffect::Poison] = 40;
        chances.strength = 7;
        let blow = Blow { damage: 5, effects: chances, by_monster: false };
        let effects = [party.clone(), poison.clone()];
        assert_eq!(rows(&effects, None, true).collect::<Vec<_>>(), std::slice::from_ref(&party));
        let bodies: Vec<EffectSpec> = rows(&effects, Some(blow), false).collect();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].chance, Some(40));
        assert_eq!(
            bodies[0].kind,
            EffectKind::Afflict { condition: Condition::Poisoned, poison: 7, indefinite: false }
        );
        // No blow, or no chance: no row.
        assert_eq!(rows(&effects, None, false).count(), 0);
        let none = Blow { effects: AttackEffects::default(), ..blow };
        assert_eq!(rows(&effects, Some(none), false).count(), 0);
        // Only from a monster.
        let from_monsters = EffectSpec { by_monsters: true, ..Default::default() };
        let effects = [from_monsters];
        assert_eq!(rows(&effects, Some(blow), false).count(), 0);
        let by_monster = Blow { by_monster: true, ..blow };
        assert_eq!(rows(&effects, Some(by_monster), false).count(), 1);
    }

    #[test]
    fn a_party_row_happens_at_its_chance() {
        let mut rng = GameRng::seeded(1);
        assert!(happens(&mut rng, None));
        assert!((0..100).all(|_| happens(&mut rng, Some(100))));
        assert!((0..100).all(|_| !happens(&mut rng, Some(0))));
        let half = (0..2000).filter(|_| happens(&mut rng, Some(50))).count();
        assert!((900..=1100).contains(&half), "{half}");
    }

    #[test]
    fn an_affliction_lasts_its_magnitude_a_one_becomes_two_and_none_ends_when_indefinite() {
        assert_eq!(lasting(false, 6), Lasting::Rounds(6));
        assert_eq!(lasting(false, 1), Lasting::Rounds(2));
        assert_eq!(lasting(false, 0), Lasting::Rounds(0));
        assert_eq!(lasting(true, 6), Lasting::Indefinite);
        assert_eq!(lasting(true, 1), Lasting::Indefinite);
    }

    #[test]
    fn turncoat_does_nothing_to_a_neutral_monster() {
        let turn = |stance| afflicts(Condition::Turncoat, stance, false, 5);
        assert_eq!(turn(Stance::Neutral), None);
        assert_eq!(turn(Stance::Hostile), Some(Lasting::Rounds(5)));
        assert_eq!(turn(Stance::Friendly), Some(Lasting::Rounds(5)));
        assert_eq!(afflicts(Condition::Blind, Stance::Neutral, false, 1), Some(Lasting::Rounds(2)));
    }

    #[test]
    fn a_leech_takes_what_the_health_could_bear_and_a_budget_what_it_has() {
        assert_eq!((bearable(8, 5), bearable(8, 20), bearable(0, 5)), (5, 8, 0));
        assert_eq!((budget_lost(10, 3), budget_lost(2, 3), budget_lost(0, 3)), (3, 2, 0));
    }

    /// (Piety 30 + Vitality 34) >> 2 = 16 percent: of 100 health 16, of 50
    /// stamina 8; of 3 health 0 -> 1.
    #[test]
    fn a_raised_member_lives_with_a_share_of_piety_and_vitality_not_under_one() {
        assert_eq!(raised(100, 50, 30, 34), (16, 8));
        assert_eq!(raised(3, 3, 30, 34), (1, 1));
        assert_eq!(raised(100, 100, 0, 0), (1, 1));
        assert_eq!(raised(100, 100, 100, 100), (50, 50));
    }

    #[test]
    fn purifying_takes_the_magnitude_and_the_purifiers_level_off_a_cloud() {
        assert_eq!(purified(3, 4), 7);
    }

    /// 40 percent of 200 gold is 80; at most 100 percent; 250 is held to 100.
    #[test]
    fn lost_gold_is_a_percent_of_the_purse_at_most_all_of_it() {
        assert_eq!((gold_lost(200, 40), gold_lost(200, 250), gold_lost(200, 100)), (80, 200, 200));
        assert_eq!((gold_lost(0, 40), gold_lost(7, 50), gold_lost(200, 0)), (0, 3, 0));
    }

    #[test]
    fn heat_burns_each_of_a_stack_on_its_own() {
        let mut rng = GameRng::seeded(2);
        assert_eq!(burnt(&mut rng, 30, 100), 30);
        assert_eq!(burnt(&mut rng, 30, 0), 0);
        assert_eq!(burnt(&mut rng, 0, 100), 0);
        let burnt: u32 = (0..1000).map(|_| u32::from(super::burnt(&mut rng, 10, 30))).sum();
        assert!((2_800..=3_200).contains(&burnt), "{burnt}");
    }

    /// 1d1 is 1; times the power 4 is 4; raised by 50 percent: 6. Another spell
    /// absorbs nothing, and draws nothing.
    #[test]
    fn a_guardian_angel_absorbs_its_dice_times_the_power_boosted() {
        let mut rng = GameRng::seeded(3);
        let dice = Some(Dice { base: 0, count: 1, sides: 1 });
        assert_eq!(absorbs(&mut rng, true, dice, 4, 50), 6);
        assert_eq!(absorbs(&mut rng, true, dice, 4, 0), 4);
        assert_eq!(absorbs(&mut rng, false, dice, 4, 50), 0);
        assert_eq!(absorbs(&mut rng, true, None, 4, 50), 0);
    }

    /// An enchantment changes its bearer if it was laid; one first spent wholly
    /// on a slowing changed it too, if it had anything to lay.
    #[test]
    fn an_enchantment_spent_against_a_slowing_still_changed_something() {
        assert!(enchanted(true, false, 5, 5));
        assert!(!enchanted(false, false, 0, 5));
        assert!(enchanted(false, true, 0, 5), "spent wholly on a slowing");
        assert!(!enchanted(false, true, 2, 5), "left to lay but not laid");
        assert!(!enchanted(false, true, 0, 0), "nothing to lay");
    }
}
