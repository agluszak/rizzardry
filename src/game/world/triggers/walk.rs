//! Spatial devices and the party crossing their boundaries.

use std::time::Duration;

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::{Device, LevelState, PendingPartyMove, TriggerState, activate};
use crate::content::items::Dice;
use crate::game::party::body::PartyBody;
use crate::game::world::level::{Level, LevelOwned};

/// A device's activation volume. Only devices with a place participate in walking.
#[derive(Component, Debug)]
pub(crate) struct WalkVolume {
    pub centre: Vec3,
    pub range: (f32, f32),
    pub order: usize,
}

impl WalkVolume {
    fn contains(&self, eye: Vec3) -> bool {
        (self.range.0..self.range.1).contains(&eye.distance(self.centre))
    }
}

// Trigger::Run action 0x23 schedules half-second pulses.
const DAMAGE_ZONE_PERIOD: f32 = 0.5;
// TriggerTimedEvent action 0x23 rolls 1d6 + 2 for each living member.
const DAMAGE_ZONE_DICE: Dice = Dice { count: 1, sides: 6, base: 2 };

#[derive(Component, Debug)]
pub(crate) struct ActiveDamageZone(Timer);

impl ActiveDamageZone {
    pub(super) fn new() -> Self {
        Self(Timer::from_seconds(DAMAGE_ZONE_PERIOD, TimerMode::Repeating))
    }
}

#[derive(Event)]
pub(crate) struct WalkedDevice {
    device: Entity,
    inside: bool,
    entered: bool,
    elapsed: Duration,
}

/// Query live spatial devices; no level registry duplicates their membership.
/// Commands resolve in authored order so linked effects affect later crossings.
pub(crate) fn walk(
    time: Res<Time>,
    party: Single<&Position, With<PartyBody>>,
    mut level: Single<(Entity, &mut LevelState), With<Level>>,
    devices: Query<(Entity, &WalkVolume, &LevelOwned)>,
    mut commands: Commands,
) {
    let eye = PartyBody::eye(*party);
    let (level, state) = &mut *level;
    let previous = state.previous_eye;
    if previous != Some(eye) {
        state.previous_eye = Some(eye);
    }
    let mut devices: Vec<_> = devices.iter().filter(|(_, _, owner)| owner.0 == *level).collect();
    devices.sort_unstable_by_key(|(_, volume, _)| volume.order);
    for (device, volume, _) in devices {
        let inside = volume.contains(eye);
        commands.trigger(WalkedDevice {
            device,
            inside,
            entered: inside && previous.is_some_and(|eye| !volume.contains(eye)),
            elapsed: time.delta(),
        });
    }
}

/// Occupancy is sampled once; activation and hazards see effects of preceding devices.
pub(crate) fn crossed(
    event: On<WalkedDevice>,
    mut devices: Query<(&Device, &TriggerState, &LevelOwned, Option<&mut ActiveDamageZone>)>,
    moving: Query<(), With<PendingPartyMove>>,
    mut commands: Commands,
) {
    let (device, state, owner, zone) = devices.get_mut(event.device).unwrap();
    if let Some(mut zone) = zone {
        if !event.inside {
            commands.entity(event.device).remove::<ActiveDamageZone>();
        } else if zone.0.tick(event.elapsed).just_finished() {
            commands.trigger(super::PartyHurt(DAMAGE_ZONE_DICE));
        }
    }
    if event.entered
        && !moving.contains(owner.0)
        && device.program.usable
        && state.on
        && !(device.program.once && state.fired)
    {
        let entity = event.device;
        commands.queue(move |world: &mut World| {
            activate(world, entity, false);
        });
    }
}
