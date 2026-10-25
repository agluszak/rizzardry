//! What passing game time does to the party. Retail ticks stamina every
//! 1.4 game seconds and heals and ages conditions on its 120-second aging
//! tick; how much they recover depends on how the time is spent.
//!
//! Whatever passes time applies this at once, with its own pace:
//! exploring and camping here, a combat round when it ends.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::characters::Character;
use crate::game::party::body::{PartyBody, WALK_SPEED};
use crate::game::party::roster::PartyRoster;
use crate::game::time::{CAMPING_RATE, EXPLORATION_RATE, GameClock, Ticks};
use crate::game::world::identify::DetectGroups;

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

/// What the ticks passed at `pace` do to one member.
pub fn elapse(member: &mut Character, ticks: Ticks, pace: Pace) {
    let (stamina, health) = pace.scales();
    if ticks.stamina > 0 {
        member.recover_stamina(ticks.stamina, stamina);
    }
    if ticks.aging > 0 {
        member.recover_health(ticks.aging, health);
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
    mut members: Query<&mut Character>,
    mut clock: ResMut<GameClock>,
    mut detect: MessageWriter<DetectGroups>,
) {
    if battle.is_some() {
        return;
    }
    let racing = camp.as_ref().is_some_and(|c| c.racing());
    let ticks =
        clock.advance_real(time.delta(), if racing { CAMPING_RATE } else { EXPLORATION_RATE });
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
        if let Ok(mut member) = members.get_mut(member) {
            elapse(&mut member, ticks, pace);
        }
    }
    // Retail looks the monsters over at the end of each aging pass.
    if ticks.aging > 0 {
        detect.write(DetectGroups);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};

    #[test]
    fn a_combat_round_ages_conditions_but_recovers_nothing() {
        let mut fighter = Character::new("Brute", Race::Human, Gender::Male, Profession::Fighter);
        fighter.hp -= 5;
        fighter.stamina -= 5;
        fighter.inflict(Condition::Afraid, Lasting::Rounds(1));
        let (hp, stamina) = (fighter.hp, fighter.stamina);
        elapse(&mut fighter, Ticks { stamina: 85, aging: 1 }, Pace::Fighting);
        assert_eq!((fighter.hp, fighter.stamina), (hp, stamina));
        assert!(!fighter.has(Condition::Afraid));
        // Standing still for two hours heals.
        elapse(&mut fighter, Ticks { stamina: 85, aging: 60 }, Pace::Still);
        assert!(fighter.hp > hp && fighter.stamina > stamina);
    }
}
