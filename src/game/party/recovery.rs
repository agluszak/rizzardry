//! The party's passive recovery as game time passes. Retail ticks stamina
//! every 1.4 game seconds outside combat and heals on its 120-second aging
//! tick; both depend on whether the party stands still, walks or runs, and
//! run at full rate while camping. Combat is not modelled yet.

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::characters::Character;
use crate::game::party::body::{PartyBody, WALK_SPEED};
use crate::game::time::GameClock;

const STAMINA_TICK_MS: u64 = 1400;
const AGING_TICK_MS: u64 = 120_000;
/// Slower than this, in metres a second, the party is resting.
const RESTING_BELOW: f32 = 0.05;
/// Faster than walking: running.
const RUNNING_ABOVE: f32 = WALK_SPEED * 1.1;

/// Whole periods of `period` the clock crossed between two readings.
fn ticks(previous: u64, now: u64, period: u64) -> u32 {
    (now / period).saturating_sub(previous / period) as u32
}

pub fn recover(
    clock: Res<GameClock>,
    mut last: Local<Option<u64>>,
    party: Query<&LinearVelocity, With<PartyBody>>,
    camp: Option<Res<Camp>>,
    mut members: Query<&mut Character>,
) {
    let previous = last.replace(clock.ms).unwrap_or(clock.ms);
    let (stamina_ticks, aging_ticks) =
        (ticks(previous, clock.ms, STAMINA_TICK_MS), ticks(previous, clock.ms, AGING_TICK_MS));
    if stamina_ticks == 0 && aging_ticks == 0 {
        return;
    }
    // Retail tells running (fast movement) from walking; here the speed
    // does, horizontally so that falling is not running.
    let speed = party.single().map_or(0.0, |v| Vec2::new(v.x, v.z).length());
    let (stamina_scale, health_scale) = if camp.is_some() {
        (1.0, 1.0)
    } else if speed < RESTING_BELOW {
        (0.5, 0.5)
    } else if speed <= RUNNING_ABOVE {
        (0.1, 0.0)
    } else {
        (0.0, 0.0)
    };
    for mut member in &mut members {
        if stamina_ticks > 0 {
            member.recover_stamina(stamina_ticks, stamina_scale);
        }
        if aging_ticks > 0 {
            member.recover_health(aging_ticks, health_scale);
            member.age_conditions(aging_ticks);
        }
    }
}
