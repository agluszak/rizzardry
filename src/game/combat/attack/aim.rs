//! The roll to hit, a swing's fate, where a blow lands and getting through
//! armour (`docs/rules/attack.md` section 4).

use bevy::math::Vec3Swizzles;
use bevy::prelude::Vec3;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::{Location, choose_mode};
use crate::content::characters::BodyPart;
use crate::content::items::AttackMode;
use crate::game::actors::HitPlaces;
use crate::game::characters::scale_by_level;
use crate::game::rng::GameRng;

/// A blind body's aim past 10 is lost; one who fights blind keeps a share of
/// it by its level: `10 + (figure - 10) * (level * 2 + 60) / 100`, cut down,
/// the level over 20 counting as 100.
pub fn blind_cap(figure: i32, blind: bool, fights_blind: Option<u8>) -> i32 {
    if figure <= 10 || !blind {
        return figure;
    }
    fights_blind.map_or(10, |level| scale_by_level(level, (figure - 10) as f32) as i32 + 10)
}

/// The chance a swing reaches, and the odds it goes astray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chance {
    /// The aim, to be at least the roll.
    pub aim: i32,
    /// The figure the fumble chance is worked from: the base with the blind
    /// rule and difficulty applied, but not the target's armour or agility.
    pub fumble_odds: i32,
}

/// The chance a swing goes astray onto someone else in reach (retail
/// `GetCombatFumbleChance`): none from an aim of 100 or more, otherwise
/// cubic in what it falls short by and growing with who could be struck.
pub fn fumble_chance(bystanders: usize, aim: i32) -> u32 {
    if aim >= 100 {
        return 0;
    }
    let chance = (f64::from(100 - aim).powi(3) * 1e-5 + 0.5) as u32;
    (bystanders as u32 * chance * 10 / 100).min(100)
}

/// What a close swing's d100 says of it (retail: 5 or under always reaches, a
/// fumble comes next, over 95 always misses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Misses,
    /// It reaches; a roll of 5 or under is sure to get through armour.
    Reaches {
        sure: bool,
    },
    Fumbles,
}

/// What a roll of a close swing comes to, with `bystanders` in reach and the
/// given fumble chance.
pub fn verdict(roll: u32, bystanders: usize, fumble: u32) -> Verdict {
    if roll <= 5 {
        Verdict::Reaches { sure: true }
    } else if bystanders > 0 && roll > 100 - fumble {
        Verdict::Fumbles
    } else if roll > 95 {
        Verdict::Misses
    } else {
        Verdict::Reaches { sure: false }
    }
}

/// The world units in a metre (retail's 500).
const UNITS_PER_METRE: f32 = 500.0;

/// Where a missile aimed from `from` at `to` actually heads (retail
/// `ScatterMissileAimPoint`, in metres): a random direction across the line
/// of flight, at a distance accuracy shrinks. A blind shooter scatters
/// coarsely.
pub fn scatter(rng: &mut GameRng, from: Vec3, to: Vec3, accuracy: i32, blind: bool) -> Vec3 {
    let d = to - from;
    // Two directions across the line of flight, as retail builds them.
    let across = Vec3::new(d.y * -d.x, d.z * d.z - d.x * -d.x, -(d.z * d.y));
    let other = Vec3::new(
        across.y * d.z - across.z * d.y,
        across.z * d.x - d.z * across.x,
        across.x * d.y - across.y * d.x,
    );
    let angle = (rng.below(360) as f32).to_radians();
    let direction =
        across.normalize_or_zero() * angle.cos() + other.normalize_or_zero() * angle.sin();
    let distance = d.length();
    let units = f64::from(distance * UNITS_PER_METRE);
    if units < 1.0 {
        return to;
    }
    let distance = f64::from(distance);
    let (factor, widest) = if blind { ((1.0 / distance).min(1.75), 1.75) } else { (0.08, 0.2) };
    let closest = factor * distance;
    let spread = (units * 0.002) as i32;
    let effective = (100 - spread) * accuracy / 100;
    let mut offset = if effective <= 0 {
        widest * distance
    } else if effective >= 300 {
        closest
    } else {
        closest + (widest - factor) * f64::from(300 - effective) / 300.0 * distance
    };
    let roll = rng.below(100) as i32 + 1;
    if effective.min(0x5f) >= roll {
        offset = f64::from(roll) / f64::from(effective) * closest;
    } else if effective >= 1 || roll + effective >= 1 {
        let weight = if effective < 1 {
            f64::from(100 - (roll + effective)) * 0.01
        } else {
            f64::from(roll - effective) / f64::from(roll)
        };
        offset = (offset - closest) * weight + closest;
    }
    to + direction.normalize_or_zero() * offset as f32
}

/// Whether a missile heading for `aimed` strikes a body centred at `at`:
/// within its radius across, and its half-height up or down.
pub fn strikes(at: Vec3, aimed: Vec3, radius: f32, half_height: f32) -> bool {
    let off = aimed - at;
    off.xz().length() <= radius && off.y.abs() <= half_height
}

/// Where a blow lands on a body with these places, for one roll below 100:
/// by the attacker's own chances for the parts of a character's body where
/// it has them, otherwise by each place's; past them, on the trunk.
pub fn lands_at(roll: u32, places: &HitPlaces, own: Option<&EnumMap<BodyPart, u8>>) -> Location {
    let mut total = 0;
    for (index, place) in places.places.iter().enumerate() {
        total += match (own, place.part) {
            (Some(own), Some(part)) => u32::from(own[part]),
            _ => place.chance,
        };
        if roll < total {
            return Location(index);
        }
    }
    Location(places.trunk)
}

/// The mode that pierces: a launcher's missile mode, drawn from the missile's
/// modes, otherwise the swing's own.
pub fn piercing_mode(
    rng: &mut GameRng,
    swing: AttackMode,
    loaded: Option<EnumSet<AttackMode>>,
) -> AttackMode {
    loaded.map_or(swing, |modes| choose_mode(rng, modes))
}

/// Whether what is shot or thrown is turned aside by a Missile Shield, with
/// chance equal to the shield in percent, for a roll below 100.
pub fn turned_aside(mode: AttackMode, shield: u32, roll: u32) -> bool {
    matches!(mode, AttackMode::Shoot | AttackMode::Throw) && roll < shield
}
