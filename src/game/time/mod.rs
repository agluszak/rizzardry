//! Simulation time.

use bevy::prelude::*;

use crate::game::camp::Camp;

const DAY_MS: u64 = 86_400_000;

/// The game clock. Retail starts a new game at noon and runs the clock at 12
/// times real time while exploring (`g_view_distance`, despite its name), so
/// a day lasts two real hours. Camping runs it at 2880 times, so eight hours
/// pass in ten real seconds.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameClock {
    /// Milliseconds since midnight of the first day.
    pub ms: u64,
}

impl Default for GameClock {
    fn default() -> Self {
        GameClock { ms: DAY_MS / 2 }
    }
}

/// Game milliseconds per real millisecond while exploring.
pub const EXPLORATION_RATE: u64 = 12;

impl GameClock {
    /// The time of day as 0..=255: 0 is midnight, 128 noon (retail
    /// `((ms / 1000) << 8) / 86400`).
    pub fn phase(&self) -> u8 {
        (((self.ms % DAY_MS) / 1000) * 256 / 86_400) as u8
    }
}

pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameClock>().add_systems(FixedUpdate, advance_clock);
    }
}

/// Game milliseconds per real millisecond while camping.
pub const CAMPING_RATE: u64 = 2880;

pub fn advance_clock(time: Res<Time>, camp: Option<Res<Camp>>, mut clock: ResMut<GameClock>) {
    let rate = if camp.is_some_and(|c| c.racing()) { CAMPING_RATE } else { EXPLORATION_RATE };
    clock.ms += time.delta().as_millis() as u64 * rate;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_follow_the_day() {
        assert_eq!(GameClock::default().phase(), 128);
        assert_eq!(GameClock { ms: 0 }.phase(), 0);
        assert_eq!(GameClock { ms: DAY_MS - 1 }.phase(), 255);
        assert_eq!(GameClock { ms: DAY_MS + DAY_MS / 4 }.phase(), 64);
    }
}
