//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping in `game::time`, a combat round when it ends.

use bevy::prelude::*;

use crate::game::characters::{Character, Vitals};
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
    rng: &mut crate::game::rng::GameRng,
) {
    let vitals = member.bypass_change_detection();
    let before = vitals.clone();
    elapse(profile, vitals, ticks, pace, rng);
    if *vitals != before {
        member.set_changed();
    }
}

/// What the ticks passed at `pace` do to one member: poison bites first,
/// then they recover, then their conditions age (retail
/// `GameTurnsPassedChar`).
pub fn elapse(
    profile: &Character,
    member: &mut Vitals,
    ticks: Ticks,
    pace: Pace,
    rng: &mut crate::game::rng::GameRng,
) {
    let (stamina, health) = pace.scales();
    if ticks.stamina > 0 {
        member.recover_stamina(
            ticks.stamina,
            stamina,
            profile.stats(&member.enchantments).max_stamina(),
        );
    }
    if ticks.aging > 0 {
        let still = matches!(pace, Pace::Still | Pace::Camping);
        let poisoned = member.poison_damage(ticks.aging, still);
        if poisoned > 0 {
            member.damage(poisoned);
        }
        member.recover_health(ticks.aging, health, profile.stats(&member.enchantments).max_hp());
        member.age_conditions(ticks.aging, rng);
        let stats = profile.stats(&member.enchantments);
        member.hp = member.hp.min(stats.max_hp());
        member.stamina = member.stamina.min(stats.max_stamina());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};
    use crate::game::rng::GameRng;

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
        elapse(
            &profile,
            &mut fighter,
            Ticks { stamina: 85, aging: 1 },
            Pace::Fighting,
            &mut GameRng::default(),
        );
        assert_eq!((fighter.hp, fighter.stamina), (hp, stamina));
        assert!(!fighter.has(Condition::Afraid));
        // Standing still for two hours heals.
        elapse(
            &profile,
            &mut fighter,
            Ticks { stamina: 85, aging: 60 },
            Pace::Still,
            &mut GameRng::default(),
        );
        assert!(fighter.hp > hp && fighter.stamina > stamina);
    }

    /// A poison bites at its strength each aging tick, half again on the
    /// move, and weakens by its share of what is left of it (retail
    /// `GameTurnsPassedChar`, `TickCharacterCondition`).
    #[test]
    fn poison_bites_harder_on_the_move_and_wears_off() {
        let profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut walking = Vitals::new(&profile);
        assert!(!walking.poison(Lasting::Rounds(10), 0), "no strength, no poisoning");
        assert!(walking.poison(Lasting::Rounds(10), 4));
        let mut still = walking.clone();
        let hp = walking.hp;
        let mut rng = GameRng::default();
        elapse(&profile, &mut walking, Ticks { stamina: 0, aging: 1 }, Pace::Walking, &mut rng);
        elapse(&profile, &mut still, Ticks { stamina: 0, aging: 1 }, Pace::Still, &mut rng);
        assert_eq!(walking.hp, hp - 6, "four, half again on the move");
        assert_eq!(still.hp, hp - 4);
        // A tenth of the course gone takes a tenth of the strength, which
        // rounds to nothing here but a roll's chance of one.
        assert!(still.poison == 4 || still.poison == 3);
        assert_eq!(still.conditions, [(Condition::Poisoned, Lasting::Rounds(9))]);
        // Half of what is left: half the strength.
        let mut halved = Vitals::new(&profile);
        halved.poison(Lasting::Rounds(10), 8);
        halved.age_conditions(5, &mut rng);
        assert_eq!(halved.poison, 4);
        // A stronger poison raises the bite; a weaker one does not lower it,
        // and a fresh poisoning after one runs out starts afresh.
        halved.poison(Lasting::Rounds(2), 6);
        assert_eq!(halved.poison, 6);
        halved.poison(Lasting::Rounds(2), 1);
        assert_eq!(halved.poison, 6);
        halved.age_conditions(20, &mut rng);
        assert!(!halved.has(Condition::Poisoned));
        assert_eq!(halved.poison_damage(5, false), 0);
        halved.poison(Lasting::Rounds(3), 2);
        assert_eq!(halved.poison, 2);
    }
}
