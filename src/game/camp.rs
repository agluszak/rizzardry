//! Camping: the party rests eight game hours with time racing, or is forced
//! to rest while nobody can act until someone can. Retail's sequence
//! (`BeginPartyCamping`, `UpdateSurpriseMode`, `EndSurprise`): a second's
//! fade into the camping screen, the rest itself, a second's fade back.
//! Ambushes and the waking Senses roll wait for combat.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::game::characters::{Condition, Vitals};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::time::GameClock;

/// How long a requested rest lasts, in game milliseconds.
const REST_MS: u64 = 8 * 3_600_000;
/// Each fade: half a second to black, half a second from it.
pub const FADE_SECONDS: f32 = 1.0;
/// Faster than this, in metres a second, the party may not camp.
const STILL_BELOW: f32 = 0.05;

/// The party is camping. Exists only while it does.
#[derive(Resource, Debug)]
pub struct Camp {
    /// Nobody could act: rest until someone can, not for eight hours.
    pub forced: bool,
    pub phase: CampPhase,
    /// When a requested rest ends; set once the fade-in has finished.
    until: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CampPhase {
    /// Fading to the camping screen.
    FadingIn(Timer),
    Resting,
    /// Fading back to the world.
    FadingOut(Timer),
}

impl CampPhase {
    fn fading_in() -> Self {
        CampPhase::FadingIn(Timer::from_seconds(FADE_SECONDS, TimerMode::Once))
    }

    fn fading_out() -> Self {
        CampPhase::FadingOut(Timer::from_seconds(FADE_SECONDS, TimerMode::Once))
    }
}

impl Camp {
    fn new(forced: bool) -> Self {
        Camp { forced, phase: CampPhase::fading_in(), until: None }
    }

    /// Whether game time races (retail's accelerated calendar).
    pub fn racing(&self) -> bool {
        self.phase == CampPhase::Resting
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
    Cancelled,
    Recovered,
}

/// Rest (retail's Camp command).
#[derive(InputAction)]
#[action_output(bool)]
pub struct Rest;

/// Whether a member can act: alive and nothing worse than webbed.
pub fn can_act(member: &Vitals) -> bool {
    !member.is_dead() && member.worst_condition().is_none_or(|c| c < Condition::Turncoat)
}

/// The Camp button (retail `RequestCamp`, as the key does).
#[derive(Message, Debug, Clone, Copy)]
pub struct CampRequest;

pub fn request_camp(
    mut commands: Commands,
    rest: Single<&ActionEvents, With<Action<Rest>>>,
    mut button: MessageReader<CampRequest>,
    camp: Option<Res<Camp>>,
    battle: Option<Res<crate::game::combat::Battle>>,
    party: Query<&LinearVelocity, With<PartyBody>>,
    mut news: MessageWriter<CampNews>,
) {
    let asked = button.read().count() > 0;
    if !(rest.contains(ActionEvents::START) || asked) || camp.is_some() {
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
    battle: Option<Res<crate::game::combat::Battle>>,
    roster: Res<PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CampNews>,
) {
    let anyone_alive = members.iter_many(&roster.0).flatten().any(|m| !m.is_dead());
    let anyone_able = members.iter_many(&roster.0).flatten().any(can_act);
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
    if camp.forced {
        news.write(CampNews::Started { forced: true });
    } else if camp.phase == CampPhase::Resting {
        camp.phase = CampPhase::fading_out();
    }
}

pub fn advance_camp(
    mut commands: Commands,
    time: Res<Time>,
    clock: Res<GameClock>,
    camp: Option<ResMut<Camp>>,
    roster: Res<PartyRoster>,
    members: Query<&Vitals>,
    mut news: MessageWriter<CampNews>,
) {
    let Some(mut camp) = camp else { return };
    let anyone_able = members.iter_many(&roster.0).flatten().any(can_act);
    match &mut camp.phase {
        CampPhase::FadingIn(fade) => {
            if fade.tick(time.delta()).is_finished() {
                camp.phase = CampPhase::Resting;
                camp.until = Some(clock.ms + REST_MS);
            }
        }
        CampPhase::Resting => {
            let woke = if camp.forced {
                anyone_able
            } else {
                camp.until.is_some_and(|until| clock.ms >= until)
            };
            if woke {
                camp.phase = CampPhase::fading_out();
            }
        }
        CampPhase::FadingOut(fade) => {
            if !fade.tick(time.delta()).is_finished() {
                return;
            }
            news.write(match (camp.forced, camp.until) {
                (true, _) if anyone_able => CampNews::Recovered,
                (true, _) => CampNews::Interrupted,
                (false, None) => CampNews::Cancelled,
                (false, Some(until)) if clock.ms >= until => CampNews::Done,
                (false, Some(_)) => CampNews::Interrupted,
            });
            commands.remove_resource::<Camp>();
        }
    }
}
