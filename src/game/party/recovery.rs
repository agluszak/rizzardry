//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping here, a combat round when it ends.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::characters::{Character, Vitals};
use crate::game::party::body::{PartyBody, WALK_SPEED};
use crate::game::party::roster::PartyRoster;
use crate::game::time::{CAMPING_RATE, EXPLORATION_RATE, GameClock, Ticks};
use crate::game::world::identify::DetectGroups;

/// Apply the Fighter's combat-rest trait to an already rolled recovery amount.
/// It follows the current profession's level, capped at twenty, and truncates
/// only after scaling the roll (retail ScaleValueByProfessionLevel).
pub fn combat_rest_amount(profile: &Character, rolled: u32) -> u32 {
    use crate::game::characters::Profession;
    if profile.profession != Profession::Fighter {
        return rolled;
    }
    let level = profile.profession_levels[profile.profession];
    let factor = if level > 20 { 3.3 } else { (f32::from(level) * 2.0 + 60.0) * 3.3 * 0.01 };
    (factor * rolled as f32) as u32
}

/// Slower than this, in metres a second, the party is resting.
const RESTING_BELOW: f32 = 0.05;
/// Faster than walking: running.
const RUNNING_ABOVE: f32 = WALK_SPEED * 1.1;

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

/// A read-only assessment keeps full, unaffected members out of the write
/// query path while still clearing recovery fractions when the policy changes.
pub fn has_work(profile: &Character, member: &Vitals, ticks: Ticks, pace: Pace) -> bool {
    let (stamina, health) = pace.scales();
    (ticks.stamina > 0
        && (member.stamina_fraction != 0.0
            || (!member.is_dead() && stamina > 0.0 && member.stamina < profile.max_stamina())))
        || (ticks.aging > 0
            && (member.hp_fraction != 0.0
                || (!member.is_dead() && health > 0.0 && member.hp < profile.max_hp())
                || member.conditions.iter().any(|(_, lasting)| {
                    matches!(lasting, crate::game::characters::Lasting::Rounds(_))
                })))
}

/// What the ticks passed at `pace` do to one member.
pub fn elapse(profile: &Character, member: &mut Vitals, ticks: Ticks, pace: Pace) {
    let (stamina, health) = pace.scales();
    if ticks.stamina > 0 {
        member.recover_stamina(ticks.stamina, stamina, profile.max_stamina());
    }
    if ticks.aging > 0 {
        member.recover_health(ticks.aging, health, profile.max_hp());
        member.age_conditions(ticks.aging);
    }
}

/// Exploring and camping: the clock runs on the fixed step and the party
/// recovers as it passes. In a fight only its rounds pass time.
pub fn pass_time(
    time: Res<Time>,
    camp: Option<Res<Camp>>,
    battle: Option<Res<crate::game::combat::Battle>>,
    party: Query<&LinearVelocity, With<PartyBody>>,
    roster: Res<PartyRoster>,
    mut members: Query<(&Character, &mut Vitals)>,
    mut clock: ResMut<GameClock>,
    mut detect: MessageWriter<DetectGroups>,
    levels: Query<
        Entity,
        (With<crate::game::world::level::Level>, Without<crate::game::world::level::Departing>),
    >,
) {
    if battle.is_some() {
        return;
    }
    let racing = camp.as_ref().is_some_and(|c| c.racing());
    let ticks =
        clock.advance_real(time.delta(), if racing { CAMPING_RATE } else { EXPLORATION_RATE });
    if ticks.stamina == 0 && ticks.aging == 0 {
        return;
    }
    // Retail tells running (fast movement) from walking; here the speed
    // does, horizontally so that falling is not running.
    let speed = party.single().map_or(0.0, |v| Vec2::new(v.x, v.z).length());
    let pace = if camp.is_some() {
        Pace::Camping
    } else if speed < RESTING_BELOW {
        Pace::Still
    } else if speed <= RUNNING_ABOVE {
        Pace::Walking
    } else {
        Pace::Running
    };
    for &member in &roster.0 {
        if members.get(member).is_ok_and(|(profile, vitals)| has_work(profile, vitals, ticks, pace))
        {
            let (profile, mut vitals) = members.get_mut(member).unwrap();
            elapse(profile, &mut vitals, ticks, pace);
        }
    }
    // Retail looks the monsters over at the end of each aging pass.
    if ticks.aging > 0
        && let Ok(level) = levels.single()
    {
        detect.write(DetectGroups { level });
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

#[cfg(test)]
mod roster_tests {
    use super::*;
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::time::AGING_TICK_MS;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn passing_time_heals_the_party_not_every_character() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<GameClock>()
            .init_resource::<PartyRoster>()
            .add_message::<DetectGroups>()
            .add_systems(Update, pass_time);
        let hurt = || {
            let c = Character::new("Hurt", Race::Human, Gender::Male, Profession::Fighter);
            let mut vitals = Vitals::new(&c);
            vitals.hp = 1;
            (c, vitals)
        };
        let member = app.world_mut().spawn(hurt()).id();
        // An adventurer the party has not recruited.
        let stranger = app.world_mut().spawn(hurt()).id();
        app.world_mut().resource_mut::<PartyRoster>().0.insert(member);
        // A long real step: at 12x, many aging ticks of standing still.
        let step = std::time::Duration::from_millis(AGING_TICK_MS * 30 / EXPLORATION_RATE);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
        app.world_mut().resource_mut::<Time<Virtual>>().set_max_delta(step);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().get::<Vitals>(member).unwrap().hp > 1);
        assert_eq!(app.world().get::<Vitals>(stranger).unwrap().hp, 1);
    }
}
