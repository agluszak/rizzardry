//! Places the party walks into: entering one sets its device going, and one
//! that hurts hurts whoever stays in it.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::{Activate, By, HurtParty, LevelState};
use crate::content::items::Dice;
use crate::game::party::body::PartyBody;
use crate::game::world::level::{Level, LevelOwned};

/// A device's place: the party's eye entering it, between the near and far
/// distances from its centre, sets it going. The level authors an order,
/// which the devices are walked into in.
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

/// How often a place that hurts hurts (retail `Trigger::Run` action 0x23's
/// half-second pulses).
const DAMAGE_ZONE_PERIOD: f32 = 0.5;
/// What it hurts each living member by (retail `TriggerTimedEvent`, 0x23).
const DAMAGE_ZONE_DICE: Dice = Dice { count: 1, sides: 6, base: 2 };

/// A place hurting whoever stays in it, until the party leaves.
#[derive(Component, Debug)]
pub(crate) struct ActiveDamageZone(Timer);

impl ActiveDamageZone {
    pub(super) fn new() -> Self {
        Self(Timer::from_seconds(DAMAGE_ZONE_PERIOD, TimerMode::Repeating))
    }
}

/// The party's eye this step against the last: places it has entered set
/// their devices going, in the level's order; places that hurt hurt while
/// it stays and stop once it leaves.
pub(crate) fn walk(
    time: Res<Time>,
    party: Single<&Position, With<PartyBody>>,
    mut level: Single<(Entity, &mut LevelState), With<Level>>,
    mut places: Query<(Entity, &WalkVolume, &LevelOwned, Option<&mut ActiveDamageZone>)>,
    mut commands: Commands,
) {
    let eye = PartyBody::eye(*party);
    let (level, state) = &mut *level;
    let previous = state.previous_eye;
    if previous != Some(eye) {
        state.previous_eye = Some(eye);
    }
    let mut places: Vec<_> =
        places.iter_mut().filter(|(_, _, owner, _)| owner.0 == *level).collect();
    places.sort_unstable_by_key(|(_, place, ..)| place.order);
    for (device, place, _, zone) in places {
        let inside = place.contains(eye);
        if let Some(mut zone) = zone {
            if !inside {
                commands.entity(device).remove::<ActiveDamageZone>();
            } else if zone.0.tick(time.delta()).just_finished() {
                commands.trigger(HurtParty(DAMAGE_ZONE_DICE));
            }
        }
        if inside && previous.is_some_and(|eye| !place.contains(eye)) {
            commands.trigger(Activate { device, by: By::Walking });
        }
    }
}
