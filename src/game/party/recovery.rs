//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping in `game::time`, a combat round when it ends.

use crate::content::characters::Trait;
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
    if !profile.has_trait(Trait::StaminaRegeneration) {
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
/// round ends. Whatever time wears on reacts to it: the party's bodies
/// ([`pass_time`]) and a stance set on a group; the spells laid wear
/// separately ([`Aged`]).
///
/// [`Aged`]: crate::game::actors::spells::Aged
#[derive(Event, Clone, Copy, Debug)]
pub struct Elapsed {
    pub ticks: Ticks,
    pub pace: Pace,
}

/// Time passes for every actor: a party member's poison bites, their
/// disease may strike, and they recover as the pace allows; everyone's
/// conditions run their course. A member weakened by disease is refitted, and
/// a death is settled.
#[allow(clippy::type_complexity)]
pub fn pass_time(
    elapsed: On<Elapsed>,
    mut actors: Query<(
        Entity,
        (&mut Vitals, &mut crate::game::characters::Conditions),
        (&Attributes, &crate::game::actors::Skills),
        Option<(&mut Character, &LearnedSpells, &mut SpellPoints)>,
        Has<crate::game::party::roster::PartyMember>,
    )>,
    mut laying: crate::game::actors::Laying,
    content: Res<crate::game::data::Catalogs>,
    mut rng: ResMut<GameRng>,
    mut damaged: MessageWriter<crate::game::hits::Damaged>,
) {
    use crate::game::characters::{Condition, Lasting, Sickness};
    let Elapsed { ticks, pace } = *elapsed.event();
    for (entity, (mut vitals, mut conditions), figures, sheet, member) in &mut actors {
        let (attributes, _) = figures;
        // A character without a seat is not carried along by the party's time.
        if sheet.is_some() && !member {
            continue;
        }
        if let Some((mut profile, known, mut pool)) = sheet {
            // Poison bites first, past any Guardian Angel.
            let still = matches!(pace, Pace::Still | Pace::Camping);
            let poison = laying.absorb(entity, conditions.poison_damage(ticks.aging, still));
            let lost = vitals.hurt(poison);
            if lost > 0 {
                let died = vitals.is_dead();
                damaged.write(crate::game::hits::Damaged { body: entity, amount: lost, died });
            }
            let magic = (known, &mut *pool, &content.spells);
            let body = (&mut *vitals, &mut *conditions);
            match elapse(&mut profile, body, figures, ticks, pace, &mut rng, magic) {
                // The point lost takes health and stamina with it, and
                // may kill.
                Some(Sickness::Weakened(lost)) => {
                    let less = attributes.weakened(lost);
                    let had = vitals.health.current;
                    let weakened = vitals.weaken(profile.max_hp(&less), profile.max_stamina(&less));
                    if weakened {
                        let fell =
                            crate::game::hits::Damaged { body: entity, amount: had, died: true };
                        damaged.write(fell);
                    }
                }
                // A slowing without end ends Haste.
                Some(Sickness::Condition(Condition::Slowed)) => {
                    laying.slow(entity, Lasting::Indefinite);
                }
                _ => {}
            }
        }
        if ticks.aging > 0 && conditions.any_timed() {
            conditions.age(ticks.aging, &mut rng);
        }
    }
}

/// What the ticks passed at `pace` do to one party member once poison has
/// bitten and before their conditions age: disease may strike, then they
/// recover. Returns what disease did.
pub fn elapse(
    profile: &mut Character,
    (member, conditions): (&mut Vitals, &mut crate::game::characters::Conditions),
    (attributes, skills): (&Attributes, &crate::game::actors::Skills),
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
    let sickness = member.sicken(conditions, profile, vitality, ticks.aging, rng);
    if pool.needs_recovery() {
        let maximum = profile.max_points(attributes, known, catalog, skills);
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
        (member, ailing): (&mut Vitals, &mut crate::game::characters::Conditions),
        ticks: Ticks,
        pace: Pace,
        rng: &mut GameRng,
    ) -> Option<crate::game::characters::Sickness> {
        let attributes = Attributes::of_sheet(profile);
        let magic =
            (&LearnedSpells::default(), &mut SpellPoints::default(), &SpellCatalog::default());
        let skills = crate::game::actors::Skills::of_sheet(profile);
        elapse(profile, (member, ailing), (&attributes, &skills), ticks, pace, rng, magic)
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
        let mut ailing =
            crate::game::characters::Conditions::from([(Condition::Afraid, Lasting::Rounds(1))]);
        let (hp, stamina) = (fighter.health.current, fighter.stamina.current);
        let mut rng = GameRng::default();
        let body = (&mut fighter, &mut ailing);
        pass(&mut profile, body, Ticks { stamina: 85, aging: 1 }, Pace::Fighting, &mut rng);
        assert_eq!((fighter.health.current, fighter.stamina.current), (hp, stamina));
        // Conditions age as the round ends, in `pass_time`.
        ailing.age(1, &mut rng);
        assert!(!ailing.has(Condition::Afraid));
        // Standing still for two hours heals.
        pass(
            &mut profile,
            (&mut fighter, &mut ailing),
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
        use crate::game::characters::Conditions;
        let mut rng = GameRng::default();
        let mut poisoned = Conditions::default();
        poisoned.poison(Lasting::Rounds(10), 4);
        assert_eq!(poisoned.poison_damage(1, false), 6, "four, half again on the move");
        assert_eq!(poisoned.poison_damage(1, true), 4);
        // A tenth of the course gone takes a tenth of the strength, which
        // rounds to nothing here but a roll's chance of one.
        poisoned.age(1, &mut rng);
        assert!(poisoned.poison == 4 || poisoned.poison == 3);
        assert_eq!(poisoned.lasting(Condition::Poisoned), Some(Lasting::Rounds(9)));
        // Half of what is left: half the strength.
        let mut halved = Conditions::default();
        halved.poison(Lasting::Rounds(10), 8);
        halved.age(5, &mut rng);
        assert_eq!(halved.poison, 4);
        // A stronger poison raises the bite; a weaker one does not lower it,
        // and a fresh poisoning after one runs out starts afresh.
        halved.poison(Lasting::Rounds(2), 6);
        assert_eq!(halved.poison, 6);
        halved.poison(Lasting::Rounds(2), 1);
        assert_eq!(halved.poison, 6);
        halved.age(20, &mut rng);
        assert!(!halved.has(Condition::Poisoned));
        assert_eq!(halved.poison_damage(5, false), 0);
        halved.poison(Lasting::Rounds(3), 2);
        assert_eq!(halved.poison, 2);
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
        let mut ailing = crate::game::characters::Conditions::default();
        assert_eq!(member.drain_health(&mut ailing, &mut profile, 3), 3);
        assert_eq!(
            (profile.max_hp(&Attributes::of_sheet(&profile)), member.health.current),
            (max_hp - 3, max_hp - 3)
        );
        assert_eq!(member.stamina.current, max_stamina - 2, "two thirds of it tires");
        assert!(ailing.has(Condition::Drained));
        profile.stamina_drained = 4;
        member.stamina.current =
            member.stamina.current.min(profile.max_stamina(&Attributes::of_sheet(&profile)));
        // Recovery heals only up to the lowered maximum.
        pass(
            &mut profile,
            (&mut member, &mut ailing),
            Ticks { stamina: 0, aging: 60 },
            Pace::Camping,
            &mut GameRng::default(),
        );
        assert_eq!(member.health.current, max_hp - 3);
        let stamina = member.stamina.current;
        assert!(member.lift_drain(&mut ailing, &mut profile));
        assert_eq!(
            (profile.max_hp(&Attributes::of_sheet(&profile)), member.health.current),
            (max_hp, max_hp)
        );
        assert_eq!(member.stamina.current, stamina + 4);
        assert!(!member.lift_drain(&mut ailing, &mut profile), "nothing left to lift");
        // Draining the last of it kills, and the dead stay Drained.
        let mut doomed = Vitals::new(&profile);
        doomed.health.current = 2;
        let mut ailing = crate::game::characters::Conditions::default();
        assert_eq!(doomed.drain_health(&mut ailing, &mut profile, 5), 2);
        assert!(doomed.is_dead() && ailing.has(Condition::Drained));
    }
}
