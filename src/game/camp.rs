//! Camping: the party rests eight game hours with time racing, or is forced
//! to rest while nobody can act until someone can. Retail's sequence
//! (`BeginPartyCamping`, `UpdateSurpriseMode`, `EndSurprise`): a second's
//! fade into the camping screen, the rest itself, a second's fade back.
//! Ambushes and the waking Senses roll wait for combat.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::game::characters::{Condition, Vitals};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use std::time::Duration;

/// How long a requested rest lasts, in game milliseconds.
const REST_MS: u64 = 8 * 3_600_000;
/// Each fade: half a second to black, half a second from it.
pub const FADE_SECONDS: f32 = 1.0;
/// Faster than this, in metres a second, the party may not camp.
const STILL_BELOW: f32 = 0.05;

/// The party is camping. Exists only while it does.
#[derive(Resource, Clone, Debug)]
pub enum Camp {
    FadingIn { fade: Timer, forced: bool },
    Resting(RestGoal),
    FadingOut { fade: Timer, outcome: CampNews },
}

#[derive(Clone, Copy, Debug)]
pub enum RestGoal {
    Until(u64),
    UntilAble,
}

fn fade() -> Timer {
    Timer::from_seconds(FADE_SECONDS, TimerMode::Once)
}

impl Camp {
    fn new(forced: bool) -> Self {
        Self::FadingIn { fade: fade(), forced }
    }

    /// Whether game time races (retail's accelerated calendar).
    pub fn racing(&self) -> bool {
        matches!(self, Self::Resting(_))
    }

    fn leave(&mut self) -> Option<CampNews> {
        match self {
            Self::FadingIn { forced: true, .. } | Self::Resting(RestGoal::UntilAble) => {
                Some(CampNews::Started { forced: true })
            }
            Self::Resting(RestGoal::Until(_)) => {
                *self = Self::FadingOut { fade: fade(), outcome: CampNews::Interrupted };
                None
            }
            _ => None,
        }
    }

    /// Called after this interval's clock and recovery have been settled under
    /// the phase that began it. Rendering only observes these simulation phases.
    pub(crate) fn advance(
        &mut self,
        elapsed: Duration,
        now: u64,
        anyone_able: bool,
    ) -> Option<CampNews> {
        match self {
            Self::FadingIn { fade, forced } => {
                if fade.tick(elapsed).is_finished() {
                    *self = Self::Resting(if *forced {
                        RestGoal::UntilAble
                    } else {
                        RestGoal::Until(now + REST_MS)
                    });
                }
            }
            Self::Resting(goal) => {
                let outcome = match goal {
                    RestGoal::Until(until) if now >= *until => Some(CampNews::Done),
                    RestGoal::UntilAble if anyone_able => Some(CampNews::Recovered),
                    _ => None,
                };
                if let Some(outcome) = outcome {
                    *self = Self::FadingOut { fade: fade(), outcome };
                }
            }
            Self::FadingOut { fade, outcome } => {
                if fade.tick(elapsed).is_finished() {
                    return Some(*outcome);
                }
            }
        }
        None
    }
}

/// What camping tells the player.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampNews {
    Started {
        forced: bool,
    },
    /// "You may not camp while moving!"
    Moving,
    /// "Not while in combat!"
    InCombat,
    Done,
    Interrupted,
    Recovered,
}

/// Whether a member can act: alive and nothing worse than webbed.
pub fn can_act(member: &Vitals) -> bool {
    !member.is_dead() && member.worst_condition().is_none_or(|c| c < Condition::Turncoat)
}

/// The Camp button (retail `RequestCamp`, as the key does).
#[derive(Message, Debug, Clone, Copy)]
pub struct CampRequest;

pub fn request_camp(
    mut commands: Commands,
    freedom: crate::game::screen::Freedom,
    mut button: MessageReader<CampRequest>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    party: Query<&LinearVelocity, With<PartyBody>>,
    mut news: MessageWriter<CampNews>,
) {
    let asked = button.read().count() > 0;
    if !asked || !freedom.free() {
        return;
    }
    if battle.is_some() {
        news.write(CampNews::InCombat);
        return;
    }
    if !party.single().is_ok_and(|v| v.length() < STILL_BELOW) {
        news.write(CampNews::Moving);
        return;
    }
    commands.insert_resource(Camp::new(false));
    news.write(CampNews::Started { forced: false });
}

/// When the living can no longer act, the party must camp.
pub fn force_camp(
    mut commands: Commands,
    camp: Option<Res<Camp>>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    roster: Single<&PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CampNews>,
) {
    let anyone_alive = members.iter_many(roster.iter()).matched().any(|m| !m.is_dead());
    let anyone_able = members.iter_many(roster.iter()).matched().any(can_act);
    if camp.is_some() || battle.is_some() || !anyone_alive || anyone_able {
        return;
    }
    commands.insert_resource(Camp::new(true));
    news.write(CampNews::Started { forced: true });
}

/// The player asks to stop camping (Escape on the camping screen).
#[derive(Message, Debug, Clone, Copy)]
pub struct LeaveCamp;

/// A forced camp cannot be left; a rest is cut short.
pub fn leave_camp(
    mut requests: MessageReader<LeaveCamp>,
    camp: Option<ResMut<Camp>>,
    mut news: MessageWriter<CampNews>,
) {
    let (Some(mut camp), Some(LeaveCamp)) = (camp, requests.read().last()) else { return };
    if let Some(outcome) = camp.leave() {
        news.write(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupting_before_the_deadline_stays_interrupted_after_fading() {
        let mut camp = Camp::Resting(RestGoal::Until(1000));
        assert_eq!(camp.leave(), None);
        assert!(!camp.racing());
        assert_eq!(camp.advance(Duration::from_secs(1), 2000, true), Some(CampNews::Interrupted));
    }

    #[test]
    fn forced_rest_cannot_be_cancelled_and_records_recovery_before_fading() {
        let mut camp = Camp::Resting(RestGoal::UntilAble);
        assert_eq!(camp.leave(), Some(CampNews::Started { forced: true }));
        camp.advance(Duration::from_millis(16), 0, false);
        assert!(camp.racing());
        camp.advance(Duration::from_millis(16), 0, true);
        assert!(!camp.racing());
        assert_eq!(camp.advance(Duration::from_secs(1), 0, false), Some(CampNews::Recovered));
    }
}
