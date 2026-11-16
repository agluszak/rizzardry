//! The roll to hit, a swing's fate, where a blow lands and getting through
//! armour (`docs/rules/attack.md` section 4).

use bevy::math::Vec3Swizzles;
use bevy::prelude::Vec3;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::{
    Attack, FighterReadOnlyItem, Location, Setting, agility_difference, choose_mode, is_missile,
    keeps_distance, mode_penetration, mode_to_hit,
};
use crate::content::characters::BodyPart;
use crate::content::items::AttackMode;
use crate::game::actors::{Arm, ArmourPart, HitPlaces, Place};
use crate::game::characters::{Condition, scale_by_level};
use crate::game::rng::GameRng;

/// What a blow gets past of the guard of the one struck (retail
/// `GetTargetArmorClassModifier`): of one caught unaware, what its quickness
/// and its shield add, and 2 if it is vulnerable from behind; of one on the
/// move and in its place, 4, unless the blow keeps its distance.
fn gets_past(attack: &Attack, struck: &FighterReadOnlyItem, setting: Setting) -> i32 {
    let mut past = 0;
    if attack.surprise.any() {
        let parts = &struck.parts.0;
        past += parts[ArmourPart::Speed].max(0)
            + parts[ArmourPart::Shield].max(0)
            + 2 * i32::from(struck.vulnerable_from_behind);
    }
    let in_place = !struck.body.conditions.unseats(struck.body.traits.0);
    if struck.moving(setting) && in_place && !keeps_distance(attack.mode) {
        past += 4;
    }
    past
}

/// How far a blow may miss the one struck and be blocked rather than missed
/// (retail `BlockedForSpecialReason`): by 10 for a monster vulnerable from
/// behind; otherwise by five times its Shield part, unless it was caught
/// unaware.
pub fn block(attack: &Attack, struck: &FighterReadOnlyItem) -> i32 {
    if struck.vulnerable_from_behind {
        10
    } else if attack.surprise.any() {
        0
    } else {
        5 * struck.parts.0[ArmourPart::Shield]
    }
}

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

/// The roll to hit:
///
/// ```text
/// base = arm rating + mode's to-hit + surprise - fatigue
/// aim  = base - 5 * target's armour against the aim + difference of agility
/// ```
///
/// Fatigue tells half as much on a modern arm; what the blow gets past of
/// the target's guard counts once (retail `GetTargetArmorClass`); the blind rule and then difficulty
/// are applied to both figures.
pub fn chance_to_hit(
    attack: &Attack,
    me: &FighterReadOnlyItem,
    target: &FighterReadOnlyItem,
    setting: Setting,
) -> Chance {
    let fatigue = if attack.arm.modern { me.fatigue() / 2 } else { me.fatigue() };
    let surprise =
        10 * i32::from(attack.surprise.unaware) + 10 * i32::from(attack.surprise.natural);
    let base = attack.arm.rating + mode_to_hit(attack.mode) + surprise - fatigue;
    let armour = target.body.armour.total - gets_past(attack, target, setting);
    let aim = base - 5 * armour
        + agility_difference(me.body.attributes.agility(), target.body.attributes.agility());
    let blind = me.has(Condition::Blind);
    let capped =
        |figure| me.scaled(blind_cap(figure, blind, me.body.attacks.blind_fighting), setting);
    Chance { aim: capped(aim), fumble_odds: capped(base) }
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

/// What became of a swing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    Missed,
    /// It reaches its target. It `lands` when the aim is at least the `roll`
    /// (or it is sure, or a shot that struck), and then gets `through` armour
    /// without penetrating when it is sure.
    Reached {
        roll: u32,
        lands: bool,
        through: bool,
    },
    /// It goes astray onto one of the bystanders, by index, with a fresh roll
    /// for penetration; it counts as having reached, but is not sure through
    /// armour.
    Fumbled {
        bystander: usize,
        roll: u32,
    },
}

/// The fate of a swing: a close one rolls against its aim and may fumble onto
/// one of `bystanders`; a shot from `from` is scattered about its target
/// `at` and strikes or misses, with no fumble and no automatic hit or miss.
pub fn fate(
    rng: &mut GameRng,
    arm: &Arm,
    chance: Chance,
    bystanders: usize,
    blind: bool,
    (from, at): (&Place, &Place),
) -> Fate {
    if is_missile(arm.reach) {
        let point = scatter(rng, from.eye, at.centre, chance.aim, blind);
        return if strikes(at.centre, point, at.radius, at.half_height) {
            Fate::Reached { roll: rng.below(100) + 1, lands: true, through: arm.sure_throw }
        } else {
            Fate::Missed
        };
    }
    let roll = rng.below(100) + 1;
    match verdict(roll, bystanders, fumble_chance(bystanders, chance.fumble_odds)) {
        Verdict::Misses => Fate::Missed,
        Verdict::Reaches { sure } => {
            Fate::Reached { roll, lands: sure || chance.aim >= roll as i32, through: sure }
        }
        Verdict::Fumbles => Fate::Fumbled {
            bystander: rng.below(bystanders as u32) as usize,
            roll: rng.below(100) + 1,
        },
    }
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

/// Where a blow lands on the one struck.
pub fn where_it_lands(rng: &mut GameRng, arm: &Arm, struck: &FighterReadOnlyItem) -> Location {
    lands_at(rng.below(100), struck.hit_places, arm.lands_on_party.as_ref())
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

/// The chance of getting through the armour of the one struck (the target,
/// the guard who took the blow or the bystander a fumble struck) at `place`,
/// by the mode that pierces:
///
/// ```text
/// penetration = scaled( mode's penetration + rating
///                       - attacker's fatigue (only if it tells on penetration)
///                       - 5 * armour at the place
///                       + difference of agility )
/// ```
///
/// The armour there is less what it has against the mode, and what the blow
/// gets past of the guard counts twice (retail `TargetArmorClassAtLocation`). The blind rule is not
/// applied.
pub fn penetration(
    attack: &Attack,
    (place, by): (Location, AttackMode),
    me: &FighterReadOnlyItem,
    struck: &FighterReadOnlyItem,
    setting: Setting,
) -> i32 {
    let fatigue = if attack.arm.fatigue_pierces { me.fatigue() } else { 0 };
    let piercing = Attack { mode: by, ..*attack };
    let there = struck.body.armour.at(&struck.hit_places.places[place.0]) + struck.against.0[by]
        - 2 * gets_past(&piercing, struck, setting);
    me.scaled(
        attack.arm.rating + mode_penetration(by) - fatigue - 5 * there
            + agility_difference(me.body.attributes.agility(), struck.body.attributes.agility()),
        setting,
    )
}

/// Whether what is shot or thrown is turned aside by a Missile Shield, with
/// chance equal to the shield in percent, for a roll below 100.
pub fn turned_aside(mode: AttackMode, shield: u32, roll: u32) -> bool {
    matches!(mode, AttackMode::Shoot | AttackMode::Throw) && roll < shield
}

/// Whether a swing that reached with `roll` gets through armour: not turned
/// aside by a Missile Shield, and `sure` (a roll of 5 or under in close
/// combat, a Ninja's throw) or with a penetration at least the roll. One that
/// does not does no damage.
pub fn gets_through(
    rng: &mut GameRng,
    attack: &Attack,
    at: (Location, AttackMode),
    (roll, sure): (u32, bool),
    me: &FighterReadOnlyItem,
    struck: &FighterReadOnlyItem,
    setting: Setting,
) -> bool {
    let aside = turned_aside(attack.mode, struck.body.missile_shield.0, rng.below(100));
    !aside && (sure || penetration(attack, at, me, struck, setting) >= roll as i32)
}
