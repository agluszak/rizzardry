//! Simulation time.

use bevy::prelude::*;

const DAY_MS: u64 = 86_400_000;

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
        app.init_resource::<GameClock>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_follow_the_day() {
        assert_eq!(GameClock::default().phase(), 128);
        assert_eq!(GameClock::at(0).phase(), 0);
        assert_eq!(GameClock::at(DAY_MS - 1).phase(), 255);
        assert_eq!(GameClock::at(DAY_MS + DAY_MS / 4).phase(), 64);
    }

    #[test]
    fn short_steps_lose_no_time_and_count_ticks() {
        let mut clock = GameClock::at(0);
        // 1/64 s steps at 12x: 187.5 game ms each.
        let step = std::time::Duration::from_secs_f64(1.0 / 64.0);
        let mut ticks = Ticks::default();
        for _ in 0..64 {
            let crossed = clock.advance_real(step, EXPLORATION_RATE);
            ticks.stamina += crossed.stamina;
        }
        assert_eq!(clock.ms, 12_000);
        assert_eq!(ticks.stamina, 8);
        assert_eq!(clock.advance(AGING_TICK_MS).aging, 1);
    }
}
