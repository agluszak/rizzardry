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

/// The health and stamina a dead member lives again with: `(Piety +
/// Vitality) / 4` percent of each maximum, not under 1 each.
pub fn raised(health_max: u32, stamina_max: u32, piety: u32, vitality: u32) -> (u32, u32) {
    let share = (piety + vitality) >> 2;
    ((health_max * share / 100).max(1), (stamina_max * share / 100).max(1))
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// (Piety 30 + Vitality 34) >> 2 = 16 percent: of 100 health 16, of 50
    /// stamina 8; of 3 health 0 -> 1.
    #[test]
    fn a_raised_member_lives_with_a_share_of_piety_and_vitality_not_under_one() {
        assert_eq!(raised(100, 50, 30, 34), (16, 8));
        assert_eq!(raised(3, 3, 30, 34), (1, 1));
        assert_eq!(raised(100, 100, 0, 0), (1, 1));
        assert_eq!(raised(100, 100, 100, 100), (50, 50));
    }
}
