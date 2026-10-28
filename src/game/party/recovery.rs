//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping in `game::time`, a combat round when it ends.

use bevy::prelude::*;

use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Vitals};
use crate::game::magic::{LearnedSpells, SpellPoints, max_points, recovery_scale};
use crate::game::time::Ticks;

/// Apply the Fighter's combat-rest trait to an already rolled recovery amount.
/// It follows the current profession's level, capped at twenty, and truncates
/// only after scaling the roll (retail ScaleValueByProfessionLevel).
pub fn combat_rest_amount(profile: &Character, rolled: u32) -> u32 {
    use crate::game::characters::Profession;
    if profile.profession != Profession::Fighter {
        return rolled;
    }
    (profile.scale_by_profession(3.3) * rolled as f32) as u32
}

/// How the party spends passing time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Still,
    Walking,
    Running,
    Camping,
    /// A combat round: conditions age, nobody recovers.
    Fighting,
}

impl Pace {
    /// The share of health recovered, which spell points follow.
    pub fn health_scale(self) -> f32 {
        self.scales().1
    }

    /// The share of stamina and of health recovered.
    fn scales(self) -> (f32, f32) {
        match self {
            Pace::Camping => (1.0, 1.0),
            Pace::Still => (0.5, 0.5),
            Pace::Walking => (0.1, 0.0),
            Pace::Running | Pace::Fighting => (0.0, 0.0),
        }
    }
}

/// [`elapse`] on a tracked member, which counts as changed only when
/// something did change.
pub fn pass(
    profile: &Character,
    member: &mut Mut<Vitals>,
    ticks: Ticks,
    pace: Pace,
    mut magic: Option<(&LearnedSpells, &mut SpellPoints, &SpellCatalog)>,
) {
    // GameTurnsPassedChar regenerates before aging personal enchantments.
    // Their expiry then recalculates ceilings and adjusts current points by
    // the difference. No render-frame refresh participates in either step.
    let maximum = magic
        .as_ref()
        .map(|(known, _, spells)| max_points(&profile.stats(&member.enchantments), known, spells));
    if ticks.aging > 0
        && let (Some((_, pool, _)), Some(maximum)) = (&mut magic, &maximum)
    {
        pool.recover(ticks.aging, recovery_scale(pace.health_scale()), maximum);
    }
    let vitals = member.bypass_change_detection();
    let before = vitals.clone();
    elapse(profile, vitals, ticks, pace);
    if let (Some((known, pool, spells)), Some(maximum)) = (magic, maximum) {
        let after = max_points(&profile.stats(&vitals.enchantments), known, spells);
        pool.adjust_limits(&maximum, &after);
    }
    if *vitals != before {
        member.set_changed();
    }
}

/// What the ticks passed at `pace` do to one member.
pub fn elapse(profile: &Character, member: &mut Vitals, ticks: Ticks, pace: Pace) {
    let (stamina, health) = pace.scales();
    if ticks.stamina > 0 {
        member.recover_stamina(
            ticks.stamina,
            stamina,
            profile.stats(&member.enchantments).max_stamina(),
        );
    }
    if ticks.aging > 0 {
        member.recover_health(ticks.aging, health, profile.stats(&member.enchantments).max_hp());
        member.age_conditions(ticks.aging);
        let stats = profile.stats(&member.enchantments);
        member.hp = member.hp.min(stats.max_hp());
        member.stamina = member.stamina.min(stats.max_stamina());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};

    #[test]
    fn fighter_rest_uses_current_profession_level_and_stops_scaling_at_twenty() {
        let mut profile = Character::new("Fighter", Race::Human, Gender::Male, Profession::Fighter);
        profile.level = 40;
        assert_eq!(
            combat_rest_amount(&profile, 10),
            20,
            "total level does not grant the trait's full strength"
        );
        profile.profession_levels[Profession::Fighter] = 20;
        assert_eq!(combat_rest_amount(&profile, 10), 33);
        profile.profession_levels[Profession::Fighter] = 40;
        assert_eq!(combat_rest_amount(&profile, 10), 33);
        profile.profession = Profession::Lord;
        assert_eq!(
            combat_rest_amount(&profile, 10),
            10,
            "past Fighter levels do not retain its current-class trait"
        );
    }

    #[test]
    fn a_combat_round_ages_conditions_but_recovers_nothing() {
        let profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut fighter = Vitals::new(&profile);
        fighter.hp -= 5;
        fighter.stamina -= 5;
        fighter.inflict(Condition::Afraid, Lasting::Rounds(1));
        let (hp, stamina) = (fighter.hp, fighter.stamina);
        elapse(&profile, &mut fighter, Ticks { stamina: 85, aging: 1 }, Pace::Fighting);
        assert_eq!((fighter.hp, fighter.stamina), (hp, stamina));
        assert!(!fighter.has(Condition::Afraid));
        // Standing still for two hours heals.
        elapse(&profile, &mut fighter, Ticks { stamina: 85, aging: 60 }, Pace::Still);
        assert!(fighter.hp > hp && fighter.stamina > stamina);
    }
}
