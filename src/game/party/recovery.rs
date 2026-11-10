//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping in `game::time`, a combat round when it ends.

use crate::game::actors::Attributes;
use bevy::prelude::*;

use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Vitals};
use crate::game::magic::{LearnedSpells, SpellPoints, recovery_scale};
use crate::game::rng::GameRng;
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

/// Game time has passed, at some pace: so many stamina ticks and aging
/// ticks. Exploring and camping say so as the clock runs, a fight as each
/// round ends. Whatever time wears on reacts to it.
#[derive(Message, Clone, Copy, Debug)]
pub struct Elapsed {
    pub ticks: Ticks,
    pub pace: Pace,
}

/// Time passes for every actor: a party member's poison bites, their
/// disease may strike, and they recover as the pace allows; everyone's
/// conditions run their course. A member weakened by disease is refitted.
/// The spells laid wear separately ([`spells::age`]), once a round's last
/// pulses have landed.
///
/// [`spells::age`]: crate::game::actors::spells::age
#[allow(clippy::type_complexity)]
pub fn pass_time(
    mut elapsed: MessageReader<Elapsed>,
    mut actors: Query<(
        Entity,
        &mut Vitals,
        &Attributes,
        Option<(&mut Character, &LearnedSpells, &mut SpellPoints)>,
        Has<crate::game::party::roster::PartyMember>,
    )>,
    mut laying: crate::game::actors::Laying,
    content: Res<crate::game::data::Catalogs>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
    mut damaged: MessageWriter<crate::game::hits::Damaged>,
) {
    use crate::game::characters::{Condition, Lasting, Sickness};
    for &Elapsed { ticks, pace } in elapsed.read() {
        for (entity, mut vitals, attributes, sheet, member) in &mut actors {
            // A character without a seat is not carried along by the party's time.
            if sheet.is_some() && !member {
                continue;
            }
            if let Some((mut profile, known, mut pool)) = sheet {
                // Poison bites first, past any Guardian Angel.
                let still = matches!(pace, Pace::Still | Pace::Camping);
                let poison = laying.absorb(entity, vitals.poison_damage(ticks.aging, still));
                let lost = vitals.hurt(poison);
                if lost > 0 {
                    let died = vitals.is_dead();
                    damaged.write(crate::game::hits::Damaged { body: entity, amount: lost, died });
                }
                let magic = (known, &mut *pool, &content.spells);
                match elapse(&mut profile, &mut vitals, attributes, ticks, pace, &mut rng, magic) {
                    // The point lost takes health and stamina with it, and
                    // may kill.
                    Some(Sickness::Weakened(lost)) => {
                        let less = attributes.weakened(lost);
                        let had = vitals.health.current;
                        if vitals.weaken(profile.max_hp(&less), profile.max_stamina(&less)) {
                            let fell = crate::game::hits::Damaged {
                                body: entity,
                                amount: had,
                                died: true,
                            };
                            damaged.write(fell);
                        }
                        commands.trigger(crate::game::actors::Refit { entity })
                    }
                    // A slowing without end ends Haste.
                    Some(Sickness::Condition(Condition::Slowed)) => {
                        laying.slow(entity, Lasting::Indefinite);
                    }
                    _ => {}
                }
            }
            if ticks.aging > 0 && vitals.conditions.any_timed() {
                vitals.age_conditions(ticks.aging, &mut rng);
            }
        }
    }
}

/// What the ticks passed at `pace` do to one party member once poison has
/// bitten and before their conditions age: disease may strike, then they
/// recover. Returns what disease did.
pub fn elapse(
    profile: &mut Character,
    member: &mut Vitals,
    attributes: &Attributes,
    ticks: Ticks,
    pace: Pace,
    rng: &mut GameRng,
    (known, pool, catalog): (&LearnedSpells, &mut SpellPoints, &SpellCatalog),
) -> Option<crate::game::characters::Sickness> {
    let (stamina, health) = pace.scales();
    if ticks.stamina > 0 {
        member.recover_stamina(ticks.stamina, stamina);
    }
    if ticks.aging == 0 {
        return None;
    }
    let vitality = attributes[crate::content::characters::Attribute::Vitality];
    let sickness = member.sicken(profile, vitality, ticks.aging, rng);
    if pool.needs_recovery() {
        let maximum = profile.max_points(attributes, known, catalog);
        pool.recover(ticks.aging, recovery_scale(pace.health_scale()), &maximum);
    }
    member.recover_health(ticks.aging, health);
    sickness
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};
    use crate::game::rng::GameRng;

    /// Time passing for one member with no spells to recover.
    fn pass(
        profile: &mut Character,
        member: &mut Vitals,
        ticks: Ticks,
        pace: Pace,
        rng: &mut GameRng,
    ) -> Option<crate::game::characters::Sickness> {
        let attributes = Attributes::of_sheet(profile);
        let magic =
            (&LearnedSpells::default(), &mut SpellPoints::default(), &SpellCatalog::default());
        elapse(profile, member, &attributes, ticks, pace, rng, magic)
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
        fighter.health.current -= 5;
        fighter.stamina.current -= 5;
        fighter.afflict(Condition::Afraid, Lasting::Rounds(1));
        let (hp, stamina) = (fighter.health.current, fighter.stamina.current);
        let mut rng = GameRng::default();
        pass(&mut profile, &mut fighter, Ticks { stamina: 85, aging: 1 }, Pace::Fighting, &mut rng);
        assert_eq!((fighter.health.current, fighter.stamina.current), (hp, stamina));
        // Conditions age as the round ends, in `pass_time`.
        fighter.age_conditions(1, &mut rng);
        assert!(!fighter.conditions.has(Condition::Afraid));
        // Standing still for two hours heals.
        pass(
            &mut profile,
            &mut fighter,
            Ticks { stamina: 85, aging: 60 },
            Pace::Still,
            &mut GameRng::default(),
        );
        assert!(fighter.health.current > hp && fighter.stamina.current > stamina);
    }

    /// A poison bites at its strength each aging tick, half again on the
    /// move, and weakens by its share of what is left of it (retail
    /// `GameTurnsPassedChar`, `TickCharacterCondition`).
    #[test]
    fn poison_bites_harder_on_the_move_and_wears_off() {
        let profile = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        let mut walking = Vitals::new(&profile);
        walking.poison(Lasting::Rounds(10), 4);
        let mut still = walking.clone();
        let hp = walking.health.current;
        let mut rng = GameRng::default();
        // Poison bites in `pass_time`, then conditions age.
        walking.hurt(walking.poison_damage(1, false));
        still.hurt(still.poison_damage(1, true));
        walking.age_conditions(1, &mut rng);
        still.age_conditions(1, &mut rng);
        assert_eq!(walking.health.current, hp - 6, "four, half again on the move");
        assert_eq!(still.health.current, hp - 4);
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
            member.afflict(Condition::Diseased, Lasting::Indefinite);
            let (max_hp, max_stamina) = (
                sick.max_hp(&Attributes::of_sheet(&sick)),
                sick.max_stamina(&Attributes::of_sheet(&sick)),
            );
            let attributes = sick.attributes;
            // A long stretch makes the roll sure.
            let vitality =
                Attributes::of_sheet(&sick)[crate::content::characters::Attribute::Vitality];
            let struck = member.sicken(&mut sick, vitality, 10_000, &mut rng);
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
                    assert_eq!(sick.max_hp(&Attributes::of_sheet(&sick)), max_hp - loss);
                    assert_eq!(member.health.current, max_hp - loss);
                    seen.insert(2);
                }
                Some(Sickness::Stamina(loss)) => {
                    assert_eq!(sick.max_stamina(&Attributes::of_sheet(&sick)), max_stamina - loss);
                    seen.insert(3);
                }
                None => {}
            }
            assert!(!member.conditions.has(Condition::Drained), "disease does not drain");
        }
        assert_eq!(seen.len(), 4, "every blow comes up");
        // Without disease, or with no time, nothing happens.
        let mut well = Vitals::new(&profile);
        assert_eq!(well.sicken(&mut profile, 10, 10_000, &mut rng), None);
        well.afflict(Condition::Diseased, Lasting::Indefinite);
        assert_eq!(well.sicken(&mut profile, 10, 0, &mut rng), None);
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
                member.afflict(Condition::Diseased, Lasting::Indefinite);
                let max_hp = profile.max_hp(&Attributes::of_sheet(&profile));
                member.health.current = if dying { 1 } else { max_hp - 20 };
                let hp = member.health.current;
                let vitality = Attributes::of_sheet(&profile)[Attribute::Vitality];
                let struck =
                    member.sicken(&mut profile, vitality, 10_000, &mut GameRng::seeded(seed));
                if struck != Some(Sickness::Weakened(Attribute::Vitality)) {
                    continue;
                }
                // The lost point takes health with it.
                let attributes = Attributes::of_sheet(&profile);
                let lowered = max_hp - profile.max_hp(&attributes);
                assert!(lowered > 0, "a veteran's health follows Vitality");
                let died =
                    member.weaken(profile.max_hp(&attributes), profile.max_stamina(&attributes));
                if dying {
                    assert!(died && member.is_dead(), "too little health left to lose");
                    dying_seen = true;
                } else {
                    assert_eq!(member.health.current, hp - lowered);
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
        let (max_hp, max_stamina) = (
            profile.max_hp(&Attributes::of_sheet(&profile)),
            profile.max_stamina(&Attributes::of_sheet(&profile)),
        );
        assert_eq!(member.drain_health(&mut profile, 3), 3);
        assert_eq!(
            (profile.max_hp(&Attributes::of_sheet(&profile)), member.health.current),
            (max_hp - 3, max_hp - 3)
        );
        assert_eq!(member.stamina.current, max_stamina - 2, "two thirds of it tires");
        assert!(member.conditions.has(Condition::Drained));
        profile.stamina_drained = 4;
        member.stamina.current =
            member.stamina.current.min(profile.max_stamina(&Attributes::of_sheet(&profile)));
        // Recovery heals only up to the lowered maximum.
        pass(
            &mut profile,
            &mut member,
            Ticks { stamina: 0, aging: 60 },
            Pace::Camping,
            &mut GameRng::default(),
        );
        assert_eq!(member.health.current, max_hp - 3);
        let stamina = member.stamina.current;
        assert!(member.lift_drain(&mut profile));
        assert_eq!(
            (profile.max_hp(&Attributes::of_sheet(&profile)), member.health.current),
            (max_hp, max_hp)
        );
        assert_eq!(member.stamina.current, stamina + 4);
        assert!(!member.lift_drain(&mut profile), "nothing left to lift");
        // Draining the last of it kills, and the dead stay Drained.
        let mut doomed = Vitals::new(&profile);
        doomed.health.current = 2;
        assert_eq!(doomed.drain_health(&mut profile, 5), 2);
        assert!(doomed.is_dead() && doomed.conditions.has(Condition::Drained));
    }
}
