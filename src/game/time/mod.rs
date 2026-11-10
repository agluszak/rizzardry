//! Simulation time.

use crate::game::actors::spells::Aged;
use crate::game::camp::Camp;
use crate::game::party::body::{PartyBody, WALK_SPEED};
use crate::game::party::recovery::{Elapsed, Pace};
use crate::game::rng::GameRng;
use crate::game::world::{identify::GroupRecognition, level::level_active};
use avian3d::prelude::LinearVelocity;
use avian3d::prelude::Position;
use bevy::prelude::*;

const RESTING_BELOW: f32 = 0.05;
const RUNNING_ABOVE: f32 = WALK_SPEED * 1.1;

/// Game minutes, hours and days in milliseconds.
pub const MINUTE_MS: u64 = 60_000;
pub const HOUR_MS: u64 = 60 * MINUTE_MS;
pub const DAY_MS: u64 = 24 * HOUR_MS;

/// The game clock. Retail starts a new game at noon and runs the clock at 12
/// times real time while exploring (`g_view_distance`, despite its name), so
/// a day lasts two real hours. Camping runs it at 2880 times, so eight hours
/// pass in ten real seconds.
///
/// Time passes only through [`GameClock::advance`], whose caller applies
/// what that time means for the party at once (`recovery::elapse`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameClock {
    /// Milliseconds since midnight of the first day.
    pub ms: u64,
    /// Game nanoseconds short of the next millisecond, so that fixed steps
    /// do not lose time to rounding.
    nanos: u64,
}

impl Default for GameClock {
    fn default() -> Self {
        GameClock::at(DAY_MS / 2)
    }
}

/// Game milliseconds per real millisecond while exploring.
pub const EXPLORATION_RATE: u64 = 12;
/// Game milliseconds per real millisecond while camping.
pub const CAMPING_RATE: u64 = 2880;

/// Retail ticks stamina every 1.4 game seconds and ages everything (health,
/// conditions) every 120.
const STAMINA_TICK_MS: u64 = 1400;
pub const AGING_TICK_MS: u64 = 120_000;

/// The periodic ticks an advance of the clock crossed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticks {
    pub stamina: u32,
    pub aging: u32,
}

impl GameClock {
    pub fn at(ms: u64) -> Self {
        GameClock { ms, nanos: 0 }
    }

    /// The time of day as 0..=255: 0 is midnight, 128 noon (retail
    /// `((ms / 1000) << 8) / 86400`).
    pub fn phase(&self) -> u8 {
        (((self.ms % DAY_MS) / 1000) * 256 / 86_400) as u8
    }

    /// Pass `real` time at `rate` game milliseconds per real one, keeping
    /// the remainder; returns the ticks crossed.
    pub fn advance_real(&mut self, real: std::time::Duration, rate: u64) -> Ticks {
        let nanos = self.nanos + real.as_nanos() as u64 * rate;
        self.nanos = nanos % 1_000_000;
        self.advance(nanos / 1_000_000)
    }

    /// Pass `ms` of game time; returns the ticks crossed.
    pub fn advance(&mut self, ms: u64) -> Ticks {
        let before = self.ms;
        self.ms += ms;
        let crossed = |period: u64| (self.ms / period - before / period) as u32;
        Ticks { stamina: crossed(STAMINA_TICK_MS), aging: crossed(AGING_TICK_MS) }
    }
}

pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameClock>().add_systems(
            FixedPostUpdate,
            pass_time
                .after(crate::game::party::walk_in::move_party)
                .after(avian3d::prelude::PhysicsSystems::Writeback)
                .run_if(level_active),
        );
    }
}

/// Exploring and camping: the clock runs on the fixed step, and says how
/// much time passed at what pace ([`Elapsed`]), and how much it wore on the
/// spells laid ([`Aged`]); on each aging tick the party looks again at what
/// is about it. In a fight only its rounds pass
/// time.
pub(crate) fn pass_time(
    time: Res<Time>,
    camp: Option<Res<Camp>>,
    search: Res<crate::game::world::search::Search>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    party: Single<(Entity, &Position, &PartyBody, &LinearVelocity)>,
    laid: crate::game::actors::Laid,
    mut recognition: GroupRecognition,
    mut clock: ResMut<GameClock>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    if battle.is_some() {
        return;
    }
    let (party, position, body, velocity) = *party;
    let rate = if camp.is_some() { CAMPING_RATE } else { EXPLORATION_RATE };
    let ticks = clock.advance_real(time.delta(), rate);
    // Retail tells running (fast movement) from walking; here the speed
    // does, horizontally so that falling is not running.
    let speed = Vec2::new(velocity.x, velocity.z).length();
    let pace = if camp.is_some() {
        Pace::Camping
    } else if speed < RESTING_BELOW {
        Pace::Still
    } else if speed <= RUNNING_ABOVE {
        Pace::Walking
    } else {
        Pace::Running
    };
    if ticks.stamina > 0 || ticks.aging > 0 {
        commands.trigger(Elapsed { ticks, pace });
    }
    if ticks.aging > 0 {
        commands.trigger(Aged { ticks: ticks.aging });
    }
    for _ in 0..ticks.aging {
        recognition.detect(
            PartyBody::eye(position),
            body.forward(),
            None,
            laid.bonus(party).warns,
            search.on(false),
            &mut rng,
        );
    }
}
