//! What a swing rolls, as ordinary functions (retail `Combat Attack.cpp`):
//! the modes' terms, where a blow lands, fatigue, fumbles and scatter. The
//! swing systems ([`super::swings`]) add their terms up. What a member
//! strikes with is [`crate::game::actors::arms`].
//!
//! A missile's flight is not simulated: where a shot lands is retail's
//! scatter around its aim point, and it strikes its target when that point
//! falls within the target's body. Retail flies the shot and can hit
//! whatever else it meets on the way.

use bevy::math::Vec3Swizzles;
use bevy::prelude::{Component, Vec3};
use enumset::EnumSet;

use crate::content::items::{AttackMode, Dice};
use crate::game::characters::Vitals;
use crate::game::rng::GameRng;

/// The to-hit term of a mode (retail `AttackModeMod` for the aim).
pub fn mode_to_hit(mode: AttackMode) -> i32 {
    use AttackMode::*;
    match mode {
        Swing | Bash | Shoot => 0,
        Thrust | Kick => -10,
        Berserk => -30,
        Throw => -5,
        Punch => 5,
        Lash => 10,
    }
}

/// The penetration term of a mode (retail `AttackModeMod` for armour).
pub fn mode_penetration(mode: AttackMode) -> i32 {
    use AttackMode::*;
    match mode {
        Swing | Kick => 0,
        Thrust | Shoot => 10,
        Bash | Punch => -5,
        Berserk => -30,
        Throw => 5,
        Lash => -10,
    }
}

/// Throws, lashes and shots: modes that neither find a gap in a distracted
/// guard nor roll extra dice for it.
pub fn keeps_distance(mode: AttackMode) -> bool {
    matches!(mode, AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot)
}

/// Pick one of the modes (retail `ChooseAttackMode`): draw `w` below 9,
/// then walk the mode bits round from the first, taking the (w+1)-th set
/// one. Not uniform when the count does not divide 9.
pub fn choose_mode(rng: &mut GameRng, modes: EnumSet<AttackMode>) -> AttackMode {
    let wanted = rng.below(9) as usize;
    modes.iter().cycle().nth(wanted).unwrap_or(AttackMode::Swing)
}

/// Stamina spent by one swing (retail `CharacterActionFatigueCost`): punches
/// and kicks cost a flat roll, weapons by their weight; a Berserk order
/// doubles it, and so does disease. Load scaling belongs to the caller.
pub fn swing_stamina(
    rng: &mut GameRng,
    mode: AttackMode,
    weight: Option<u16>,
    berserk: bool,
    diseased: bool,
) -> u32 {
    let cost = match mode {
        AttackMode::Punch => rng.below(3) + 2,
        AttackMode::Kick => rng.below(3) + 3,
        _ => match weight {
            None => rng.below(4) + 3,
            Some(weight) => {
                let band = u32::from(weight) / 40 + 1;
                rng.below(band) + 1 + band
            }
        },
    };
    cost * if berserk { 2 } else { 1 } * if diseased { 2 } else { 1 }
}

/// A monster's body, where a blow lands past its chances.
pub const MONSTER_BODY: usize = 3;
/// Where blows from the party land on one of its own (retail
/// `gubLocalACPercent`): head, body, legs, hands, feet.
pub const MEMBER_LOCATIONS: [u8; 5] = [15, 40, 30, 10, 5];

/// Where a blow lands, by cumulative percentages against one `below(100)`
/// draw; anything past them hits `fallback`.
pub fn locate(rng: &mut GameRng, chances: &[u8], fallback: usize) -> Location {
    let roll = rng.below(100);
    let mut total = 0;
    for (location, &chance) in chances.iter().enumerate() {
        total += u32::from(chance);
        if roll < total {
            return Location(location);
        }
    }
    Location(fallback)
}

/// Aim and armour lost to fatigue by the share of stamina spent (retail
/// `FatigueArmorPenalty`).
pub fn fatigue(vitals: &Vitals) -> i32 {
    match vitals.stamina.spent_percent() {
        0..50 => 0,
        50..70 => 5,
        70..85 => 10,
        85..=94 => 20,
        _ => 40,
    }
}

/// How an attack catches its target (retail `unaware` and `natural_attack`).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Surprise {
    /// Struck from behind, unseen.
    pub unaware: bool,
    /// A rogue's backstab, or a monster's natural ambush.
    pub natural: bool,
}

impl Surprise {
    pub fn any(self) -> bool {
        self.unaware || self.natural
    }
}

/// Where a blow lands: one of a monster's seven locations, or a member's
/// body part.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location(pub usize);

/// Damage with `reduction` percent cut off it.
pub fn reduce(damage: i32, reduction: i32) -> u32 {
    let damage = if reduction != 0 { ((100 - reduction) * damage + 50) / 100 } else { damage };
    damage.max(0) as u32
}

/// The most one blow that went astray takes off its victim: a fifth of its
/// health, at least one (retail `CapAttackDamageByTargetHealth`).
pub fn fumble_cap(damage: u32, health: u32) -> u32 {
    damage.min((health / 5).max(1))
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

/// One swing's to-hit roll and what decides it (retail: 5 or under always
/// lands, over 95 always misses, a fumble lands elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    Missed,
    /// The swing reaches its target: `roll` against the aim and
    /// penetration, which a roll of 5 or under gets through.
    Reaches {
        roll: u32,
        sure: bool,
    },
    /// It goes astray: the bystander it strikes, by index, with a fresh
    /// roll for penetration.
    Fumbled {
        bystander: usize,
        roll: u32,
    },
}

/// Roll a melee swing (retail `ResolveCharacterAttack`,
/// `ContinueMonsterAttack`): the roll, then a fumble onto one of
/// `bystanders` when it falls within the fumble chance.
pub fn swing(rng: &mut GameRng, bystanders: usize, fumble: u32) -> Aim {
    let roll = rng.below(100) + 1;
    let sure = roll <= 5;
    if !sure && bystanders > 0 && roll > 100 - fumble {
        let bystander = rng.below(bystanders as u32) as usize;
        return Aim::Fumbled { bystander, roll: rng.below(100) + 1 };
    }
    if roll > 95 { Aim::Missed } else { Aim::Reaches { roll, sure } }
}

/// Extra dice a backstab rolls (retail `RollBackstabExtraDice`), up to
/// three with a high enough chance.
pub fn backstab_dice(rng: &mut GameRng, chance: i32) -> u32 {
    let roll = rng.below(100) as i32;
    [15, 75, 95].iter().take_while(|&&over| roll + over < chance).count() as u32
}

/// Roll `dice` `count` times (retail `RollDice` with a roll count).
pub fn roll_dice(rng: &mut GameRng, dice: Dice, count: u32) -> u32 {
    (0..count).map(|_| rng.roll(dice)).sum()
}

/// The effects' strength (retail effect power level): the attacker's level
/// and as much again up to 15.
pub fn effect_power(level: u32) -> u32 {
    level + if level > 14 { 15 } else { level }
}

/// The kill chance training adds to a blow: a point per 25 of `skill` and
/// a chance at one more.
pub fn critical_chance(rng: &mut GameRng, skill: u32) -> u32 {
    let mut bonus = skill / 25;
    if !skill.is_multiple_of(25) && rng.below(25) < skill % 25 {
        bonus += 1;
    }
    bonus
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

/// When an actor acts in the round, 1 (first) to 100: a d4 plus its
/// initiative, highest first.
pub fn phase(rng: &mut GameRng, initiative: i32) -> u32 {
    let v = rng.below(4) as i32 + initiative;
    (90 - v.clamp(-10, 89)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_costs_follow_the_attack_and_disease_and_berserk_double_them() {
        for (mode, weight, range) in [
            (AttackMode::Punch, Some(800), 2..=4),
            (AttackMode::Kick, Some(800), 3..=5),
            (AttackMode::Swing, None, 3..=6),
            (AttackMode::Swing, Some(0), 2..=2),
            (AttackMode::Swing, Some(80), 4..=6),
        ] {
            for seed in 0..16 {
                let cost = swing_stamina(&mut GameRng::seeded(seed), mode, weight, false, false);
                assert!(range.contains(&cost));
                let rng = || GameRng::seeded(seed);
                assert_eq!(swing_stamina(&mut rng(), mode, weight, false, true), 2 * cost);
                assert_eq!(swing_stamina(&mut rng(), mode, weight, true, true), 4 * cost);
            }
        }
    }

    #[test]
    fn early_phases_go_to_high_initiative() {
        let mut rng = GameRng::seeded(1);
        for _ in 0..20 {
            assert!(phase(&mut rng, 200) == 1);
            assert!(phase(&mut rng, -50) == 100);
            assert!((87..=90).contains(&phase(&mut rng, 0)));
        }
    }

    #[test]
    fn steadier_aim_scatters_closer_and_blind_poor_shots_scatter_wide() {
        let from = Vec3::ZERO;
        let to = Vec3::new(0.0, 0.0, -10.0);
        let spread = |accuracy, blind| -> f32 {
            (0..200)
                .map(|seed| {
                    let aimed = scatter(&mut GameRng::seeded(seed), from, to, accuracy, blind);
                    // Always across the line of flight.
                    assert!((aimed - to).z.abs() < 1e-3);
                    aimed.distance(to)
                })
                .sum::<f32>()
                / 200.0
        };
        let (poor, good, blind) = (spread(20, false), spread(150, false), spread(20, true));
        assert!(good < poor, "{good} < {poor}");
        assert!(poor < blind, "{poor} < {blind}");
        // At ten metres the tightest scatter is within 8% of the distance.
        assert!(spread(400, false) <= 0.8);
    }
}
