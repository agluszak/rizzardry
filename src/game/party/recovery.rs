//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping in `game::time`, a combat round when it ends.

use bevy::prelude::*;

use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Vitals};
use crate::game::magic::{LearnedSpells, SpellPoints, recovery_scale};
use crate::game::time::Ticks;

/// Stamina a round's rest brings back to anyone in a fight (retail
/// `AdvanceCombatRound`): two dice of a twentieth of their maximum, less
/// one, from one to 255 sides.
pub fn rest_roll(max_stamina: u32, rng: &mut crate::game::rng::GameRng) -> u32 {
    let sides = (max_stamina / 20).clamp(2, 256) as i32 - 1;
    rng.roll(crate::content::items::Dice { count: 2, sides, base: 0 })
}

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
    profile: &mut Mut<Character>,
    member: &mut Mut<Vitals>,
    ticks: Ticks,
    pace: Pace,
    (known, pool): (&LearnedSpells, &mut Mut<SpellPoints>),
    spells: &SpellCatalog,
    fate: &mut crate::game::characters::Fate,
) {
    let magic = (ticks.aging > 0 && pool.needs_recovery()).then(|| (known, &mut **pool, spells));
    let vitals = member.bypass_change_detection();
    let before = vitals.clone();
    if elapse(profile.bypass_change_detection(), vitals, ticks, pace, fate, magic).is_some() {
        profile.set_changed();
    }
    if *vitals != before {
        member.set_changed();
    }
}

/// What the ticks passed at `pace` do to one member: poison bites first,
/// then disease, then they recover, then their conditions age (retail
/// `GameTurnsPassedChar`). Returns what disease did.
pub fn elapse(
    profile: &mut Character,
    member: &mut Vitals,
    ticks: Ticks,
    pace: Pace,
    fate: &mut crate::game::characters::Fate,
    magic: Option<(&LearnedSpells, &mut SpellPoints, &SpellCatalog)>,
) -> Option<crate::game::characters::Sickness> {
    let mut sickness = None;
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
            member.damage(profile, poisoned, fate);
        }
        sickness = member.sicken(profile, ticks.aging, fate);
        if let Some((known, pool, spells)) = magic {
            let maximum = profile.stats(&member.enchantments).max_points(known, spells);
            pool.recover(ticks.aging, recovery_scale(pace.health_scale()), &maximum);
        }
        member.recover_health(ticks.aging, health, profile.stats(&member.enchantments).max_hp());
        member.age_conditions(ticks.aging, fate.rng);
        let stats = profile.stats(&member.enchantments);
        member.hp = member.hp.min(stats.max_hp());
        member.stamina = member.stamina.min(stats.max_stamina());
    }
    sickness
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::Fate;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};
    use crate::game::rng::GameRng;

    /// Fate outside any fight.
    fn calm(rng: &mut GameRng) -> Fate<'_> {
        Fate { fight: None, rng }
    }

    #[test]
    fn fighter_rest_uses_current_profession_level_and_stops_scaling_at_twenty() {
        let mut profile = Character::new("Fighter", Race::Human, Gender::Male, Profession::Fighter);
        profile.profession_levels[Profession::Mage] = 39;
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
        let mut profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut fighter = Vitals::new(&profile);
        fighter.hp -= 5;
        fighter.stamina -= 5;
        fighter.inflict(Condition::Afraid, Lasting::Rounds(1));
        let (hp, stamina) = (fighter.hp, fighter.stamina);
        elapse(
            &mut profile,
            &mut fighter,
            Ticks { stamina: 85, aging: 1 },
            Pace::Fighting,
            &mut calm(&mut GameRng::default()),
            None,
        );
        assert_eq!((fighter.hp, fighter.stamina), (hp, stamina));
        assert!(!fighter.conditions.has(Condition::Afraid));
        // Standing still for two hours heals.
        elapse(
            &mut profile,
            &mut fighter,
            Ticks { stamina: 85, aging: 60 },
            Pace::Still,
            &mut calm(&mut GameRng::default()),
            None,
        );
        assert!(fighter.hp > hp && fighter.stamina > stamina);
    }

    /// A poison bites at its strength each aging tick, half again on the
    /// move, and weakens by its share of what is left of it (retail
    /// `GameTurnsPassedChar`, `TickCharacterCondition`).
    #[test]
    fn poison_bites_harder_on_the_move_and_wears_off() {
        let mut profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut walking = Vitals::new(&profile);
        assert!(!walking.poison(Lasting::Rounds(10), 0), "no strength, no poisoning");
        assert!(walking.poison(Lasting::Rounds(10), 4));
        let mut still = walking.clone();
        let hp = walking.hp;
        let mut rng = GameRng::default();
        elapse(
            &mut profile,
            &mut walking,
            Ticks { stamina: 0, aging: 1 },
            Pace::Walking,
            &mut calm(&mut rng),
            None,
        );
        elapse(
            &mut profile,
            &mut still,
            Ticks { stamina: 0, aging: 1 },
            Pace::Still,
            &mut calm(&mut rng),
            None,
        );
        assert_eq!(walking.hp, hp - 6, "four, half again on the move");
        assert_eq!(still.hp, hp - 4);
        // A tenth of the course gone takes a tenth of the strength, which
        // rounds to nothing here but a roll's chance of one.
        assert!(still.poison == 4 || still.poison == 3);
        assert_eq!(
            still.conditions,
            crate::game::characters::Conditions::from([(Condition::Poisoned, Lasting::Rounds(9))])
        );
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
        assert!(!halved.conditions.has(Condition::Poisoned));
        assert_eq!(halved.poison_damage(5, false), 0);
        halved.poison(Lasting::Rounds(3), 2);
        assert_eq!(halved.poison, 2);
    }

    /// Disease strikes when a roll under the minutes passed out of ten times
    /// Vitality comes up: a lasting condition, a lost attribute point, or
    /// health or stamina taken from the maxima, which never leaves the pool
    /// below one (retail `GameTurnsPassedChar`).
    #[test]
    fn disease_wears_a_member_down() {
        use crate::game::characters::Sickness;
        let mut profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut seen = std::collections::HashSet::new();
        let mut rng = GameRng::seeded(3);
        for _ in 0..400 {
            let mut sick = profile.clone();
            let mut member = Vitals::new(&sick);
            member.inflict(Condition::Diseased, Lasting::Indefinite);
            let (max_hp, max_stamina) =
                (sick.base_stats().max_hp(), sick.base_stats().max_stamina());
            let attributes = sick.attributes;
            // A long stretch makes the roll sure.
            let struck = member.sicken(&mut sick, 10_000, &mut calm(&mut rng));
            match struck {
                Some(Sickness::Condition(condition)) => {
                    assert!(member.conditions.lasting(condition) == Some(Lasting::Indefinite));
                    seen.insert(0);
                }
                Some(Sickness::Weakened(attribute)) => {
                    assert_eq!(sick.attributes[attribute] + 1, attributes[attribute]);
                    seen.insert(1);
                }
                Some(Sickness::Health(loss)) => {
                    assert!((1..=4).contains(&loss));
                    assert_eq!(sick.base_stats().max_hp(), max_hp - loss);
                    assert_eq!(member.hp, max_hp - loss);
                    seen.insert(2);
                }
                Some(Sickness::Stamina(loss)) => {
                    assert_eq!(sick.base_stats().max_stamina(), max_stamina - loss);
                    seen.insert(3);
                }
                None => {}
            }
            assert!(!member.conditions.has(Condition::Drained), "disease does not drain");
        }
        assert_eq!(seen.len(), 4, "every blow comes up");
        // Without disease, or with no time, nothing happens.
        let mut well = Vitals::new(&profile);
        assert_eq!(well.sicken(&mut profile, 10_000, &mut calm(&mut rng)), None);
        well.inflict(Condition::Diseased, Lasting::Indefinite);
        assert_eq!(well.sicken(&mut profile, 0, &mut calm(&mut rng)), None);
    }

    /// A Vitality point lost to disease lowers the health maximum, and a
    /// wounded member's health falls with it; one with too little left dies
    /// of it (retail `ApplyAttributeChange`). Recovery plays no part.
    #[test]
    fn a_vitality_lost_to_disease_takes_health_with_it() {
        use crate::content::characters::Attribute;
        use crate::game::characters::Sickness;
        let mut veteran = Character::new("Old", Race::Human, Gender::Male, Profession::Fighter);
        veteran.profession_levels[Profession::Fighter] = 40;
        let mut wounded_seen = false;
        let mut dying_seen = false;
        for seed in 0..2000 {
            for dying in [false, true] {
                let mut profile = veteran.clone();
                let mut member = Vitals::new(&profile);
                member.inflict(Condition::Diseased, Lasting::Indefinite);
                let max_hp = profile.base_stats().max_hp();
                member.hp = if dying { 1 } else { max_hp - 20 };
                let hp = member.hp;
                let struck =
                    member.sicken(&mut profile, 10_000, &mut calm(&mut GameRng::seeded(seed)));
                if struck != Some(Sickness::Weakened(Attribute::Vitality)) {
                    continue;
                }
                let lowered = max_hp - profile.base_stats().max_hp();
                assert!(lowered > 0, "a veteran's health follows Vitality");
                if dying {
                    assert!(member.is_dead(), "too little health left to lose");
                    dying_seen = true;
                } else {
                    assert_eq!(member.hp, hp - lowered);
                    wounded_seen = true;
                }
            }
        }
        assert!(wounded_seen && dying_seen);
    }

    /// Draining takes health from the maximum until Drained is lifted, which
    /// gives back what draining and disease took (retail `DamageCharacter`,
    /// `RemoveCharacterCondition`).
    #[test]
    fn drained_health_stays_lost_until_the_drain_is_lifted() {
        let mut profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut member = Vitals::new(&profile);
        let (max_hp, max_stamina) =
            (profile.base_stats().max_hp(), profile.base_stats().max_stamina());
        assert_eq!(member.drain_health(&mut profile, 3, &mut calm(&mut GameRng::default())), 3);
        assert_eq!((profile.base_stats().max_hp(), member.hp), (max_hp - 3, max_hp - 3));
        assert_eq!(member.stamina, max_stamina - 2, "two thirds of it tires");
        assert!(member.conditions.has(Condition::Drained));
        profile.stamina_drained = 4;
        member.stamina = member.stamina.min(profile.base_stats().max_stamina());
        // Recovery heals only up to the lowered maximum.
        elapse(
            &mut profile,
            &mut member,
            Ticks { stamina: 0, aging: 60 },
            Pace::Camping,
            &mut calm(&mut GameRng::default()),
            None,
        );
        assert_eq!(member.hp, max_hp - 3);
        let stamina = member.stamina;
        assert!(member.lift_drain(&mut profile));
        assert_eq!((profile.base_stats().max_hp(), member.hp), (max_hp, max_hp));
        assert_eq!(member.stamina, stamina + 4);
        assert!(!member.lift_drain(&mut profile), "nothing left to lift");
        // Draining the last of it kills, and the dead stay Drained.
        let mut doomed = Vitals { hp: 2, ..Vitals::new(&profile) };
        assert_eq!(doomed.drain_health(&mut profile, 5, &mut calm(&mut GameRng::default())), 2);
        assert!(doomed.is_dead() && doomed.conditions.has(Condition::Drained));
    }
}
