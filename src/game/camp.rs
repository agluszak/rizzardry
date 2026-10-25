//! Camping: the party rests eight game hours with time racing, or is forced
//! to rest while nobody can act until someone can. Retail's sequence
//! (`BeginPartyCamping`, `UpdateSurpriseMode`, `EndSurprise`): a second's
//! fade into the camping screen, the rest itself, a second's fade back.
//! Ambushes and the waking Senses roll wait for combat.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::game::characters::{Character, Condition};
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CampPhase {
    /// Seconds into the fade to the camping screen.
    FadingIn(f32),
    Resting,
    /// Seconds into the fade back to the world.
    FadingOut(f32),
}

impl Camp {
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
pub fn can_act(member: &Character) -> bool {
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
    commands.insert_resource(Camp { forced: false, phase: CampPhase::FadingIn(0.0), until: None });
    news.write(CampNews::Started { forced: false });
}

/// When the living can no longer act, the party must camp.
pub fn force_camp(
    mut commands: Commands,
    camp: Option<Res<Camp>>,
    battle: Option<Res<crate::game::combat::Battle>>,
    roster: Res<PartyRoster>,
    members: Query<&Character>,
    mut news: MessageWriter<CampNews>,
) {
    let anyone_alive = members.iter_many(&roster.0).flatten().any(|m| !m.is_dead());
    let anyone_able = members.iter_many(&roster.0).flatten().any(can_act);
    if camp.is_some() || battle.is_some() || !anyone_alive || anyone_able {
        return;
    }
    commands.insert_resource(Camp { forced: true, phase: CampPhase::FadingIn(0.0), until: None });
    news.write(CampNews::Started { forced: true });
}

/// Escape while camping: a forced camp cannot be left, a rest is cut short.
pub fn cancel_camp(
    keys: Res<ButtonInput<KeyCode>>,
    camp: Option<ResMut<Camp>>,
    mut news: MessageWriter<CampNews>,
) {
    let Some(mut camp) = camp else { return };
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if camp.forced {
        news.write(CampNews::Started { forced: true });
    } else if camp.phase == CampPhase::Resting {
        camp.phase = CampPhase::FadingOut(0.0);
    }
}

pub fn advance_camp(
    mut commands: Commands,
    time: Res<Time>,
    clock: Res<GameClock>,
    camp: Option<ResMut<Camp>>,
    roster: Res<PartyRoster>,
    members: Query<&Character>,
    mut news: MessageWriter<CampNews>,
) {
    let Some(mut camp) = camp else { return };
    let dt = time.delta_secs();
    let anyone_able = members.iter_many(&roster.0).flatten().any(can_act);
    match camp.phase {
        CampPhase::FadingIn(elapsed) if elapsed + dt >= FADE_SECONDS => {
            camp.phase = CampPhase::Resting;
            camp.until = Some(clock.ms + REST_MS);
        }
        CampPhase::FadingIn(elapsed) => camp.phase = CampPhase::FadingIn(elapsed + dt),
        CampPhase::Resting => {
            let woke = if camp.forced {
                anyone_able
            } else {
                camp.until.is_some_and(|until| clock.ms >= until)
            };
            if woke {
                camp.phase = CampPhase::FadingOut(0.0);
            }
        }
        CampPhase::FadingOut(elapsed) if elapsed + dt >= FADE_SECONDS => {
            news.write(match (camp.forced, camp.until) {
                (true, _) if anyone_able => CampNews::Recovered,
                (true, _) => CampNews::Interrupted,
                (false, None) => CampNews::Cancelled,
                (false, Some(until)) if clock.ms >= until => CampNews::Done,
                (false, Some(_)) => CampNews::Interrupted,
            });
            commands.remove_resource::<Camp>();
        }
        CampPhase::FadingOut(elapsed) => camp.phase = CampPhase::FadingOut(elapsed + dt),
    }
}
