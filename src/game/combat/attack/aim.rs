//! The roll to hit, a swing's fate, where a blow lands and getting through
//! armour (`docs/rules/attack.md` section 4).

use bevy::math::Vec3Swizzles;
use bevy::prelude::Vec3;
use enumset::EnumSet;

use super::{
    Attack, FighterReadOnlyItem, Location, Setting, agility_difference, choose_mode, is_missile,
    keeps_distance, mode_penetration, mode_to_hit,
};
use crate::content::characters::BodyPart;
use crate::content::items::AttackMode;
use crate::game::actors::{Arm, Place};
use crate::game::characters::{Condition, scale_by_level};
use crate::game::rng::GameRng;

/// A monster's body, where a blow lands past its chances.
pub const MONSTER_BODY: usize = 3;
/// Where blows from the party land on one of its own (retail
/// `gubLocalACPercent`): head, body, legs, hands, feet.
pub const MEMBER_LOCATIONS: [u8; 5] = [15, 40, 30, 10, 5];

/// What the guard of the one struck comes to against a blow: fatigue adds a
/// tenth of itself where it tells on armour, a surprise strips what it
/// exposes (more if it is braced), and four go to a blow that closes in on it
/// on the move (retail `GetTargetArmorClassModifier`).
fn guard(attack: &Attack, struck: &FighterReadOnlyItem, setting: Setting) -> i32 {
    let armour = struck.body.armour;
    let mut guard = if armour.tires { struck.fatigue() / 10 } else { 0 };
    if attack.surprise.any() {
        guard -= armour.exposed + if struck.defending() { armour.braced.1 } else { 0 };
    }
    if struck.moving(setting) && !keeps_distance(attack.mode) {
        guard -= 4;
    }
    guard
}

/// What bracing adds to armour everywhere.
fn bracing(struck: &FighterReadOnlyItem) -> i32 {
    if struck.defending() { struck.body.armour.braced.0 } else { 0 }
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
/// Fatigue tells half as much on a modern arm; the target's guard counts
/// once (retail `GetTargetArmorClass`); the blind rule and then difficulty
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
    let armour = target.body.armour.total + bracing(target) + guard(attack, target, setting);
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

/// Where a blow lands, by cumulative percentages against one roll below 100;
/// anything past them hits `fallback`.
pub fn locate_at(roll: u32, chances: &[u8], fallback: usize) -> Location {
    let mut total = 0;
    for (location, &chance) in chances.iter().enumerate() {
        total += u32::from(chance);
        if roll < total {
            return Location(location);
        }
    }
    Location(fallback)
}

/// Where a blow lands: on a monster by its species' chances for its seven
/// locations (past them, location 3, its body); on a member by the
/// attacker's species' chances if a monster struck, otherwise 15, 40, 30, 10
/// and 5 percent for head, body, legs, hands and feet (past them, the body).
pub fn where_it_lands(rng: &mut GameRng, arm: &Arm, struck: &FighterReadOnlyItem) -> Location {
    let roll = rng.below(100);
    match (struck.body.armour.hit_locations, arm.lands_on_party) {
        (Some(table), _) => locate_at(roll, &table, MONSTER_BODY),
        (None, Some(hits)) => locate_at(roll, hits.as_slice(), BodyPart::Body as usize),
        (None, None) => locate_at(roll, &MEMBER_LOCATIONS, BodyPart::Body as usize),
    }
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
/// The armour there is less what it has against the mode, and the guard
/// counts twice (retail `TargetArmorClassAtLocation`). The blind rule is not
/// applied.
pub fn penetration(
    attack: &Attack,
    (place, by): (Location, AttackMode),
    me: &FighterReadOnlyItem,
    struck: &FighterReadOnlyItem,
    setting: Setting,
) -> i32 {
    let fatigue = if attack.arm.fatigue_pierces { me.fatigue() } else { 0 };
    let armour = struck.body.armour;
    let piercing = Attack { mode: by, ..*attack };
    let there = armour.at[place.0] + bracing(struck) - armour.by_mode[by]
        + 2 * guard(&piercing, struck, setting);
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
    let aside = turned_aside(attack.mode, struck.body.armour.missile_shield, rng.below(100));
    !aside && (sure || penetration(attack, at, me, struck, setting) >= roll as i32)
}

#[cfg(test)]
mod tests {
    use super::super::arena::{NORMAL, arena, arm, duel, monster, swing, tire};
    use super::*;
    use crate::content::characters::Attribute;
    use crate::game::actors::{Armour, Attacks, Attributes};
    use crate::game::characters::{Lasting, Vitals};
    use crate::game::combat::approach::{CombatMove, MoveGoal};
    use crate::game::combat::attack::Surprise;
    use crate::game::world::hostility::Stance;
    use bevy::prelude::{Entity, World};

    /// Two monsters facing each other.
    fn pair() -> (World, (Entity, Entity)) {
        let mut world = arena();
        let pair = (
            monster(&mut world, Vec3::ZERO, Stance::Hostile),
            monster(&mut world, Vec3::NEG_Z, Stance::Friendly),
        );
        (world, pair)
    }

    fn aim(world: &mut World, pair: (Entity, Entity), rating: i32) -> i32 {
        duel(world, pair, |me, target| chance_to_hit(&swing(&arm(rating)), me, target, NORMAL).aim)
    }

    fn blind(world: &mut World, actor: Entity) {
        world.get_mut::<Vitals>(actor).unwrap().afflict(Condition::Blind, Lasting::Rounds(9));
    }

    #[test]
    fn a_blind_attackers_aim_past_ten_is_lost() {
        let (mut world, pair) = pair();
        assert_eq!(aim(&mut world, pair, 80), 80);
        blind(&mut world, pair.0);
        // 80 blind: held to 10. Already at 10 or under: unchanged.
        assert_eq!(aim(&mut world, pair, 80), 10);
        assert_eq!(aim(&mut world, pair, 10), 10);
        assert_eq!(aim(&mut world, pair, 4), 4);
    }

    #[test]
    fn one_who_fights_blind_keeps_a_share_by_level() {
        let (mut world, pair) = pair();
        blind(&mut world, pair.0);
        // 10 + (80 - 10) * (level * 2 + 60) / 100: level 0 keeps 42, level 10
        // keeps 56, from level 20 all 70.
        for (level, aim_blind) in [(0, 52), (10, 66), (20, 80), (21, 80), (40, 80)] {
            world.get_mut::<Attacks>(pair.0).unwrap().blind_fighting = Some(level);
            assert_eq!(aim(&mut world, pair, 80), aim_blind, "level {level}");
        }
    }

    #[test]
    fn the_fumble_odds_leave_out_the_targets_armour_and_agility() {
        let (mut world, pair) = pair();
        world.get_mut::<Armour>(pair.1).unwrap().total = 6;
        // Fifty more Senses than the target: a difference of agility of 5.
        world.get_mut::<Attributes>(pair.0).unwrap().0[Attribute::Senses] = 51;
        // 90 of 100 stamina spent: fatigue 20.
        tire(&mut world, pair.0, 10);
        let chance = duel(&mut world, pair, |me, target| {
            let arm = arm(80);
            let thrust = Attack {
                mode: AttackMode::Thrust,
                surprise: Surprise { unaware: true, natural: false },
                ..swing(&arm)
            };
            chance_to_hit(&thrust, me, target, NORMAL)
        });
        // Base 80 - 10 + 10 - 20 = 60; the aim also loses 5 * 6 and gains 5.
        assert_eq!(chance.fumble_odds, 60);
        assert_eq!(chance.aim, 60 - 30 + 5);
    }

    #[test]
    fn a_swing_goes_astray_by_the_cube_of_what_its_odds_fall_short_of_100() {
        // chance = (100 - odds)^3 / 100000 + 0.5, cut down; the fumble chance
        // is bystanders * chance * 10 / 100, at most 100.
        assert_eq!(fumble_chance(5, 100), 0);
        assert_eq!(fumble_chance(5, 140), 0);
        assert_eq!(fumble_chance(0, 0), 0);
        // odds 0: 1000000 / 100000 + 0.5 = 10.5 -> 10; 3 * 10 * 10 / 100 = 3.
        assert_eq!(fumble_chance(3, 0), 3);
        // 10 bystanders: 10 * 10 * 10 / 100 = 10; 100: 100; 150: 150, held to 100.
        assert_eq!(fumble_chance(10, 0), 10);
        assert_eq!(fumble_chance(100, 0), 100);
        assert_eq!(fumble_chance(150, 0), 100);
        // odds -500: 600^3 = 216000000 / 100000 + 0.5 = 2160; 10 of them is held to 100.
        assert_eq!(fumble_chance(10, -500), 100);
        // odds 20: 80^3 = 512000 / 100000 + 0.5 = 5.62 -> 5; 4 * 5 * 10 / 100 = 2.
        assert_eq!(fumble_chance(4, 20), 2);
        // odds -50: 150^3 = 3375000 / 100000 + 0.5 = 34.25 -> 34; 2 * 34 * 10 / 100 = 6.
        assert_eq!(fumble_chance(2, -50), 6);
        // odds 90: 1000 / 100000 + 0.5 = 0.51 -> 0.
        assert_eq!(fumble_chance(9, 90), 0);
        // One bystander at odds 50: 125000 / 100000 + 0.5 = 1.75 -> 1; 1 * 1 * 10 / 100 = 0.
        assert_eq!(fumble_chance(1, 50), 0);
    }

    #[test]
    fn a_roll_of_five_or_less_reaches_and_is_sure_whatever_the_fumble_chance() {
        for roll in 1..=5 {
            assert_eq!(verdict(roll, 4, 100), Verdict::Reaches { sure: true });
        }
    }

    #[test]
    fn a_roll_above_100_less_the_fumble_chance_fumbles_if_anyone_is_in_reach() {
        // Fumble chance 30: rolls above 70.
        assert_eq!(verdict(70, 2, 30), Verdict::Reaches { sure: false });
        assert_eq!(verdict(71, 2, 30), Verdict::Fumbles);
        // The fumble is decided before the miss over 95.
        assert_eq!(verdict(99, 2, 30), Verdict::Fumbles);
        // Nobody in reach, no fumble.
        assert_eq!(verdict(71, 0, 30), Verdict::Reaches { sure: false });
        assert_eq!(verdict(96, 0, 30), Verdict::Misses);
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

    #[test]
    fn a_blow_lands_by_cumulative_percentages_and_past_them_on_the_fallback() {
        // 15, 40, 30, 10, 5: cumulative 15, 55, 85, 95, 100.
        for (roll, part) in [(0, 0), (14, 0), (15, 1), (54, 1), (55, 2), (84, 2), (85, 3), (94, 3)]
        {
            assert_eq!(locate_at(roll, &MEMBER_LOCATIONS, 1), Location(part), "roll {roll}");
        }
        assert_eq!(locate_at(95, &MEMBER_LOCATIONS, 1), Location(4));
        assert_eq!(locate_at(99, &MEMBER_LOCATIONS, 1), Location(4));
        // Chances short of 100: the rest on the fallback.
        assert_eq!(locate_at(20, &[10, 10], 7), Location(7));
        assert_eq!(locate_at(19, &[10, 10], 7), Location(1));
    }

    const BODY: (Location, AttackMode) = (Location(MONSTER_BODY), AttackMode::Swing);

    fn pierces(world: &mut World, pair: (Entity, Entity), attack: &Attack) -> i32 {
        duel(world, pair, |me, struck| {
            penetration(attack, (BODY.0, attack.mode), me, struck, NORMAL)
        })
    }

    #[test]
    fn the_victims_guard_counts_twice_in_penetration() {
        let (mut world, pair) = pair();
        let arm = arm(80);
        world.get_mut::<Armour>(pair.1).unwrap().at[MONSTER_BODY] = 4;
        assert_eq!(pierces(&mut world, pair, &swing(&arm)), 80 - 20);
        // 95 of 100 stamina spent is fatigue 40 on armour that tires: guard 4,
        // twice is 8: 4 + 8 = 12.
        world.get_mut::<Armour>(pair.1).unwrap().tires = true;
        tire(&mut world, pair.1, 5);
        assert_eq!(pierces(&mut world, pair, &swing(&arm)), 80 - 60);
        tire(&mut world, pair.1, 100);
        // A surprise strips 3 (twice, 6): 4 - 6 = -2: 80 + 10.
        world.get_mut::<Armour>(pair.1).unwrap().exposed = 3;
        let unaware =
            Attack { surprise: Surprise { unaware: true, natural: false }, ..swing(&arm) };
        assert_eq!(pierces(&mut world, pair, &unaware), 90);
        // On the move 4 less, twice is 8 (not against a throw): 4 - 8 = -4.
        let walking = CombatMove { budget: 1.0, moved: 0.0, goal: MoveGoal::Party };
        world.entity_mut(pair.1).insert(walking);
        assert_eq!(pierces(&mut world, pair, &swing(&arm)), 100);
        let throw = Attack { mode: AttackMode::Throw, ..swing(&arm) };
        assert_eq!(pierces(&mut world, pair, &throw), 80 + 5 - 20);
    }

    #[test]
    fn a_shield_of_100_stops_every_shot_even_a_sure_one_and_no_close_blow() {
        let (mut world, pair) = pair();
        world.get_mut::<Armour>(pair.1).unwrap().missile_shield = 100;
        let arm = arm(80);
        duel(&mut world, pair, |me, struck| {
            let through = |mode, seed| {
                let attack = Attack { mode, ..swing(&arm) };
                let rng = &mut GameRng::seeded(seed);
                gets_through(rng, &attack, (BODY.0, mode), (50, true), me, struck, NORMAL)
            };
            for seed in 0..50 {
                assert!(!through(AttackMode::Shoot, seed));
                assert!(through(AttackMode::Swing, seed));
            }
        });
    }
}
