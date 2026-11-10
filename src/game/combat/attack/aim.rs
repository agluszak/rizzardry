//! The roll to hit, a swing's fate, where a blow lands and getting through
//! armour (`docs/rules/attack.md` section 4).

use bevy::math::Vec3Swizzles;
use bevy::prelude::Vec3;
use enum_map::EnumMap;
use enumset::EnumSet;

use super::{
    Location, Surprise, agility_difference, choose_mode, is_missile, keeps_distance,
    mode_penetration, mode_to_hit,
};
use crate::content::characters::BodyPart;
use crate::content::items::AttackMode;
use crate::content::species::Reach;
use crate::game::actors::Armour;
use crate::game::characters::scale_by_level;
use crate::game::difficulty::Difficulty;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;

/// A monster's body, where a blow lands past its chances.
pub const MONSTER_BODY: usize = 3;
/// Where blows from the party land on one of its own (retail
/// `gubLocalACPercent`): head, body, legs, hands, feet.
pub const MEMBER_LOCATIONS: [u8; 5] = [15, 40, 30, 10, 5];

/// How the one struck stands as the blow comes: its armour, how tired it is,
/// and what it is doing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defence {
    pub armour: Armour,
    /// What fatigue costs it (retail `FatigueArmorPenalty`).
    pub fatigue: i32,
    /// Braced: its order is Defend.
    pub defending: bool,
    /// On the move, its guard down to blows that close in: a monster that is
    /// walking this round, a member while the party runs.
    pub moving: bool,
}

impl Defence {
    /// What its guard comes to against a blow: fatigue adds a tenth of
    /// itself where it tells on armour, a surprise strips what it exposes
    /// (more if it is braced), and four go to a blow that closes in on it on
    /// the move.
    pub fn guard(&self, mode: AttackMode, surprise: Surprise) -> i32 {
        let mut guard = if self.armour.tires { self.fatigue / 10 } else { 0 };
        if surprise.any() {
            guard -= self.armour.exposed + if self.defending { self.armour.braced.1 } else { 0 };
        }
        if self.moving && !keeps_distance(mode) {
            guard -= 4;
        }
        guard
    }

    fn bracing(&self) -> i32 {
        if self.defending { self.armour.braced.0 } else { 0 }
    }

    /// Its armour against a blow's aim; its guard counts once (retail
    /// `GetTargetArmorClass`, `GetTargetArmorClassModifier`).
    pub fn against_aim(&self, mode: AttackMode, surprise: Surprise) -> i32 {
        self.armour.total + self.bracing() + self.guard(mode, surprise)
    }

    /// Its armour where a blow struck, against the mode that pierces; its
    /// guard counts twice (retail `TargetArmorClassAtLocation`).
    pub fn at(&self, mode: AttackMode, surprise: Surprise, place: usize) -> i32 {
        self.armour.at[place] + self.bracing() - self.armour.by_mode[mode]
            + 2 * self.guard(mode, surprise)
    }
}

/// A figure of the attacker's, scaled for difficulty by its side (retail
/// `ScaleValueForCharacterDifficulty`, `ScaleValueForMonsterDifficulty`).
#[derive(Clone, Copy, Debug)]
pub struct Scaling {
    pub difficulty: Difficulty,
    /// The attacker's side now.
    pub side: Stance,
}

impl Scaling {
    fn of(&self, figure: i32) -> i32 {
        self.difficulty.scale(figure, self.side)
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

/// What the roll to hit weighs.
#[derive(Clone, Copy, Debug)]
pub struct Hitting {
    /// The arm's rating, its to-hit and the spell bonus counted.
    pub rating: i32,
    pub mode: AttackMode,
    /// The attacker's fatigue, and whether the arm's aim feels it half as much
    /// (a modern weapon, an attack that tires little).
    pub fatigue: i32,
    pub modern: bool,
    pub blind: bool,
    /// The level it fights blind by: a member with the trait, a Monk.
    pub fights_blind: Option<u8>,
    pub surprise: Surprise,
    /// Senses + Speed + Dexterity + Intelligence, each.
    pub agility: i32,
    pub target_agility: i32,
    pub target: Defence,
    pub scaling: Scaling,
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
/// base  = arm rating + mode's to-hit + surprise - fatigue
/// guard = base - 5 * target's armour against the aim + difference of agility
/// ```
pub fn chance_to_hit(facts: &Hitting) -> Chance {
    let fatigue = if facts.modern { facts.fatigue / 2 } else { facts.fatigue };
    let surprise = 10 * i32::from(facts.surprise.unaware) + 10 * i32::from(facts.surprise.natural);
    let base = facts.rating + mode_to_hit(facts.mode) + surprise - fatigue;
    let guard = base - 5 * facts.target.against_aim(facts.mode, facts.surprise)
        + agility_difference(facts.agility, facts.target_agility);
    let capped = |figure| facts.scaling.of(blind_cap(figure, facts.blind, facts.fights_blind));
    Chance { aim: capped(guard), fumble_odds: capped(base) }
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

/// One swing's to-hit roll and what decides it.
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
    match verdict(roll, bystanders, fumble) {
        Verdict::Misses => Aim::Missed,
        Verdict::Reaches { sure } => Aim::Reaches { roll, sure },
        Verdict::Fumbles => {
            let bystander = rng.below(bystanders as u32) as usize;
            Aim::Fumbled { bystander, roll: rng.below(100) + 1 }
        }
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

/// A shot's line from the shooter's eye to its target.
#[derive(Clone, Copy, Debug)]
pub struct Flight {
    pub from: Vec3,
    /// The target's centre.
    pub at: Vec3,
    pub radius: f32,
    pub half_height: f32,
    pub blind: bool,
}

/// What a swing's fate weighs.
#[derive(Clone, Copy, Debug)]
pub struct Reaching {
    pub reach: Reach,
    pub chance: Chance,
    /// Bodies a close swing could go astray onto: the living in reach other
    /// than its target.
    pub bystanders: usize,
    /// A thrown weapon of a Ninja gets through armour whenever it strikes.
    pub sure_throw: bool,
    pub flight: Flight,
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
    /// It goes astray onto one of the bystanders, with a fresh roll for
    /// penetration; it counts as having reached, but is not sure through armour.
    Fumbled {
        bystander: usize,
        roll: u32,
    },
}

/// The fate of a swing: a close one rolls against its aim and may fumble; a
/// shot is scattered from its aim and strikes or misses, with no fumble and
/// no automatic hit or miss.
pub fn fate(rng: &mut GameRng, facts: &Reaching) -> Fate {
    if is_missile(facts.reach) {
        let flight = &facts.flight;
        let point = scatter(rng, flight.from, flight.at, facts.chance.aim, flight.blind);
        return if strikes(flight.at, point, flight.radius, flight.half_height) {
            Fate::Reached { roll: rng.below(100) + 1, lands: true, through: facts.sure_throw }
        } else {
            Fate::Missed
        };
    }
    let fumble = fumble_chance(facts.bystanders, facts.chance.fumble_odds);
    match swing(rng, facts.bystanders, fumble) {
        Aim::Missed => Fate::Missed,
        Aim::Reaches { roll, sure } => {
            Fate::Reached { roll, lands: sure || facts.chance.aim >= roll as i32, through: sure }
        }
        Aim::Fumbled { bystander, roll } => Fate::Fumbled { bystander, roll },
    }
}

/// Where blows land on the one struck.
#[derive(Clone, Copy, Debug)]
pub struct Landing {
    /// A monster's species' chances for its seven locations.
    pub monster_locations: Option<[u8; 7]>,
    /// The attacker's species' chances for the five parts of a member, if it
    /// has them; a member's blows land by [`MEMBER_LOCATIONS`].
    pub on_party: Option<EnumMap<BodyPart, u8>>,
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

/// Where a blow lands for a roll below 100: on a monster by its species'
/// chances for its seven locations (past them, location 3, its body); on a
/// member by the attacker's species' chances if a monster struck, otherwise
/// 15, 40, 30, 10 and 5 percent for head, body, legs, hands and feet (past
/// them, the body).
pub fn location_at(roll: u32, facts: &Landing) -> Location {
    match (facts.monster_locations, facts.on_party) {
        (Some(table), _) => locate_at(roll, &table, MONSTER_BODY),
        (None, Some(hits)) => locate_at(roll, hits.as_slice(), BodyPart::Body as usize),
        (None, None) => locate_at(roll, &MEMBER_LOCATIONS, BodyPart::Body as usize),
    }
}

/// Where a blow lands, rolled.
pub fn where_it_lands(rng: &mut GameRng, facts: &Landing) -> Location {
    location_at(rng.below(100), facts)
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

/// What getting through the armour of the one struck weighs.
#[derive(Clone, Copy, Debug)]
pub struct Piercing {
    /// The arm's rating counts against armour as it does for the aim.
    pub rating: i32,
    /// The mode of the swing.
    pub swing: AttackMode,
    /// The mode that pierces.
    pub by: AttackMode,
    /// The attacker's fatigue, and whether it tells on penetration: a
    /// member's weapon that is not modern, a monster's attack that tires
    /// little.
    pub fatigue: i32,
    pub fatigue_pierces: bool,
    /// The one struck: the target, or the guard who took the blow, or the
    /// bystander a fumble struck.
    pub victim: Defence,
    /// The attack's surprise, as it is after an interception or a fumble.
    pub surprise: Surprise,
    pub place: Location,
    pub agility: i32,
    pub victim_agility: i32,
    pub scaling: Scaling,
    /// The d100 of the swing, or the fresh one of a fumble or a shot.
    pub roll: u32,
    /// Sure through armour: a roll of 5 or under in close combat, a Ninja's
    /// throw.
    pub sure: bool,
}

/// The chance of getting through armour:
///
/// ```text
/// penetration = scaled( mode's penetration + rating
///                       - attacker's fatigue (only if it tells on penetration)
///                       - 5 * victim's armour at the location
///                       + difference of agility )
/// ```
///
/// The blind rule is not applied.
pub fn penetration(facts: &Piercing) -> i32 {
    let fatigue = if facts.fatigue_pierces { facts.fatigue } else { 0 };
    facts.scaling.of(facts.rating + mode_penetration(facts.by)
        - fatigue
        - 5 * facts.victim.at(facts.by, facts.surprise, facts.place.0)
        + agility_difference(facts.agility, facts.victim_agility))
}

/// Whether what is shot or thrown is turned aside by a Missile Shield, with
/// chance equal to the shield in percent, for a roll below 100.
pub fn turned_aside(mode: AttackMode, shield: u32, roll: u32) -> bool {
    matches!(mode, AttackMode::Shoot | AttackMode::Throw) && roll < shield
}

/// Whether the swing gets through armour: not turned aside by a Missile
/// Shield, and sure or with a penetration at least the roll. One that does
/// not does no damage.
pub fn gets_through(rng: &mut GameRng, facts: &Piercing) -> bool {
    let aside = turned_aside(facts.swing, facts.victim.armour.missile_shield, rng.below(100));
    !aside && (facts.sure || penetration(facts) >= facts.roll as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMAL: Scaling = Scaling { difficulty: Difficulty::Normal, side: Stance::Hostile };

    fn hitting() -> Hitting {
        Hitting {
            rating: 80,
            mode: AttackMode::Swing,
            fatigue: 0,
            modern: false,
            blind: false,
            fights_blind: None,
            surprise: Surprise::default(),
            agility: 100,
            target_agility: 100,
            target: Defence::default(),
            scaling: NORMAL,
        }
    }

    fn aim(facts: &Hitting) -> i32 {
        chance_to_hit(facts).aim
    }

    #[test]
    fn the_aim_starts_from_the_arms_rating_and_adds_its_modes_term() {
        // 80 + the mode's term (page 4).
        for (mode, term) in [
            (AttackMode::Swing, 0),
            (AttackMode::Bash, 0),
            (AttackMode::Shoot, 0),
            (AttackMode::Thrust, -10),
            (AttackMode::Kick, -10),
            (AttackMode::Berserk, -30),
            (AttackMode::Throw, -5),
            (AttackMode::Punch, 5),
            (AttackMode::Lash, 10),
        ] {
            assert_eq!(aim(&Hitting { mode, ..hitting() }), 80 + term, "{mode:?}");
        }
    }

    #[test]
    fn fatigue_takes_from_the_aim_and_half_of_it_rounded_down_for_a_modern_arm() {
        assert_eq!(aim(&Hitting { fatigue: 20, ..hitting() }), 60);
        assert_eq!(aim(&Hitting { fatigue: 20, modern: true, ..hitting() }), 70);
        // 5 / 2 = 2.
        assert_eq!(aim(&Hitting { fatigue: 5, modern: true, ..hitting() }), 78);
        assert_eq!(aim(&Hitting { fatigue: 40, modern: true, ..hitting() }), 60);
    }

    #[test]
    fn a_surprise_adds_ten_for_each_way_it_holds() {
        let by = |unaware, natural| Surprise { unaware, natural };
        assert_eq!(aim(&Hitting { surprise: by(true, false), ..hitting() }), 90);
        assert_eq!(aim(&Hitting { surprise: by(false, true), ..hitting() }), 90);
        assert_eq!(aim(&Hitting { surprise: by(true, true), ..hitting() }), 100);
    }

    fn defended(armour: Armour) -> Defence {
        Defence { armour, ..Defence::default() }
    }

    #[test]
    fn five_off_the_aim_for_each_point_of_the_targets_armour() {
        let target = defended(Armour { total: 6, ..Armour::default() });
        // 80 - 5 * 6.
        assert_eq!(aim(&Hitting { target, ..hitting() }), 50);
    }

    #[test]
    fn a_defending_target_is_braced() {
        let armour = Armour { total: 6, braced: (2, 0), ..Armour::default() };
        let target = Defence { armour, defending: true, ..Defence::default() };
        // Armour 6 + 2 = 8: 80 - 40.
        assert_eq!(aim(&Hitting { target, ..hitting() }), 40);
        // Not defending, no bracing.
        let target = Defence { defending: false, ..target };
        assert_eq!(aim(&Hitting { target, ..hitting() }), 50);
    }

    #[test]
    fn a_target_whose_fatigue_tells_on_armour_guards_a_tenth_of_it_better() {
        let armour = Armour { total: 6, tires: true, ..Armour::default() };
        let target = Defence { armour, fatigue: 40, ..Defence::default() };
        // Armour 6 + 40 / 10 = 10: 80 - 50.
        assert_eq!(aim(&Hitting { target, ..hitting() }), 30);
        // A member's fatigue does not tell on its armour.
        let armour = Armour { tires: false, ..armour };
        assert_eq!(aim(&Hitting { target: Defence { armour, ..target }, ..hitting() }), 50);
    }

    #[test]
    fn a_surprise_strips_the_armour_it_exposes_and_more_from_the_braced() {
        let armour = Armour { total: 6, exposed: 3, braced: (2, 2), ..Armour::default() };
        let unaware = Surprise { unaware: true, natural: false };
        let target = Defence { armour, ..Defence::default() };
        // Base 80 + 10 for surprise; armour 6 - 3 = 3: 90 - 15.
        assert_eq!(aim(&Hitting { target, surprise: unaware, ..hitting() }), 75);
        // Braced: armour 6 + 2 bracing - (3 + 2) stripped = 3 again.
        let braced = Defence { defending: true, ..target };
        assert_eq!(aim(&Hitting { target: braced, surprise: unaware, ..hitting() }), 75);
        // A natural attack strips it just the same.
        let natural = Surprise { unaware: false, natural: true };
        assert_eq!(aim(&Hitting { target, surprise: natural, ..hitting() }), 75);
    }

    #[test]
    fn a_target_on_the_move_is_four_less_guarded_except_against_throws_lashes_and_shots() {
        let target = Defence {
            armour: Armour { total: 6, ..Armour::default() },
            moving: true,
            ..Defence::default()
        };
        // Armour 6 - 4 = 2: 80 - 10.
        assert_eq!(aim(&Hitting { target, ..hitting() }), 70);
        // Throw: 80 - 5 = 75, armour 6: 75 - 30.
        assert_eq!(aim(&Hitting { target, mode: AttackMode::Throw, ..hitting() }), 45);
        assert_eq!(aim(&Hitting { target, mode: AttackMode::Shoot, ..hitting() }), 50);
        assert_eq!(aim(&Hitting { target, mode: AttackMode::Lash, ..hitting() }), 60);
    }

    #[test]
    fn a_tenth_of_the_difference_of_agility_is_added() {
        assert_eq!(aim(&Hitting { agility: 120, ..hitting() }), 82);
        assert_eq!(aim(&Hitting { agility: 90, ..hitting() }), 79);
        assert_eq!(aim(&Hitting { target_agility: 160, ..hitting() }), 74);
    }

    #[test]
    fn a_blind_attackers_aim_past_ten_is_lost() {
        // 80 blind: held to 10. Already at 10 or under: unchanged.
        assert_eq!(aim(&Hitting { blind: true, ..hitting() }), 10);
        assert_eq!(aim(&Hitting { blind: true, rating: 10, ..hitting() }), 10);
        assert_eq!(aim(&Hitting { blind: true, rating: 4, ..hitting() }), 4);
        assert_eq!(aim(&Hitting { rating: 4, ..hitting() }), 4);
    }

    #[test]
    fn one_who_fights_blind_keeps_a_share_by_level() {
        // 10 + (80 - 10) * (level * 2 + 60) / 100.
        for (level, aim_blind) in [(0, 52), (10, 66), (20, 80), (21, 80), (40, 80)] {
            let facts = Hitting { blind: true, fights_blind: Some(level), ..hitting() };
            assert_eq!(aim(&facts), aim_blind, "level {level}");
        }
        // Level 0: 10 + 70 * 60 / 100 = 10 + 42 = 52. Level 10: 10 + 70 * 80 / 100
        // = 10 + 56 = 66. Level 20: 10 + 70 * 100 / 100 = 80.
        // Not blind, nothing is lost.
        let facts = Hitting { fights_blind: Some(0), ..hitting() };
        assert_eq!(aim(&facts), 80);
    }

    #[test]
    fn difficulty_scales_the_aim_by_the_attackers_side() {
        let at =
            |difficulty, side| aim(&Hitting { scaling: Scaling { difficulty, side }, ..hitting() });
        // Page 4: Novice hostile 0.6, friendly 1.4; Expert hostile 1.4,
        // friendly 0.6; Normal and neutral unchanged.
        assert_eq!(at(Difficulty::Normal, Stance::Hostile), 80);
        assert_eq!(at(Difficulty::Novice, Stance::Hostile), 48);
        assert_eq!(at(Difficulty::Novice, Stance::Friendly), 112);
        assert_eq!(at(Difficulty::Expert, Stance::Hostile), 112);
        assert_eq!(at(Difficulty::Expert, Stance::Friendly), 48);
        assert_eq!(at(Difficulty::Novice, Stance::Neutral), 80);
        assert_eq!(at(Difficulty::Expert, Stance::Neutral), 80);
    }

    #[test]
    fn the_blind_cap_comes_before_difficulty() {
        let facts = Hitting {
            blind: true,
            scaling: Scaling { difficulty: Difficulty::Expert, side: Stance::Hostile },
            ..hitting()
        };
        // 80 held to 10, then 10 * 140 / 100.
        assert_eq!(aim(&facts), 14);
    }

    #[test]
    fn the_fumble_odds_leave_out_the_targets_armour_and_agility() {
        let target = defended(Armour { total: 6, ..Armour::default() });
        let facts = Hitting {
            target,
            agility: 150,
            fatigue: 20,
            surprise: Surprise { unaware: true, natural: false },
            mode: AttackMode::Thrust,
            ..hitting()
        };
        // Base 80 - 10 + 10 - 20 = 60; the aim also loses 5 * 6 and gains 5.
        let chance = chance_to_hit(&facts);
        assert_eq!(chance.fumble_odds, 60);
        assert_eq!(chance.aim, 60 - 30 + 5);
    }

    #[test]
    fn the_fumble_odds_have_the_blind_rule_and_difficulty_applied() {
        let blind = Hitting { blind: true, ..hitting() };
        assert_eq!(chance_to_hit(&blind).fumble_odds, 10);
        let expert = Hitting {
            scaling: Scaling { difficulty: Difficulty::Expert, side: Stance::Hostile },
            ..hitting()
        };
        assert_eq!(chance_to_hit(&expert).fumble_odds, 112);
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
    fn a_roll_above_95_misses_and_otherwise_reaches() {
        assert_eq!(verdict(6, 0, 0), Verdict::Reaches { sure: false });
        assert_eq!(verdict(95, 3, 0), Verdict::Reaches { sure: false });
        assert_eq!(verdict(96, 3, 0), Verdict::Misses);
        assert_eq!(verdict(100, 3, 0), Verdict::Misses);
    }

    fn reaching(aim: i32, bystanders: usize) -> Reaching {
        Reaching {
            reach: Reach::Touch,
            chance: Chance { aim, fumble_odds: 100 },
            bystanders,
            sure_throw: false,
            flight: Flight {
                from: Vec3::ZERO,
                at: Vec3::ZERO,
                radius: 0.5,
                half_height: 1.0,
                blind: false,
            },
        }
    }

    #[test]
    fn a_close_swing_lands_when_the_aim_is_at_least_the_roll_or_the_roll_is_sure() {
        for seed in 0..300 {
            match fate(&mut GameRng::seeded(seed), &reaching(50, 0)) {
                Fate::Reached { roll, lands, through } => {
                    assert_eq!(lands, roll <= 50 || roll <= 5, "roll {roll}");
                    assert_eq!(through, roll <= 5, "roll {roll}");
                }
                Fate::Missed => {}
                Fate::Fumbled { .. } => panic!("nobody to fumble onto"),
            }
        }
        // An aim of nothing lands only the sure ones.
        for seed in 0..300 {
            if let Fate::Reached { roll, lands, .. } =
                fate(&mut GameRng::seeded(seed), &reaching(0, 0))
            {
                assert_eq!(lands, roll <= 5);
            }
        }
    }

    #[test]
    fn a_swing_that_reaches_may_still_fall_short_of_landing() {
        // Aim 20: some rolls from 6 to 95 reach but do not land.
        let reached_short = (0..300).any(|seed| {
            matches!(
                fate(&mut GameRng::seeded(seed), &reaching(20, 0)),
                Fate::Reached { lands: false, .. }
            )
        });
        assert!(reached_short);
    }

    #[test]
    fn a_swing_with_odds_far_below_100_fumbles_onto_a_bystander() {
        // Odds -500 with ten in reach: a fumble chance of 100, so every roll
        // above 5 and none other fumbles.
        let facts =
            Reaching { chance: Chance { aim: -500, fumble_odds: -500 }, ..reaching(-500, 10) };
        for seed in 0..100 {
            match fate(&mut GameRng::seeded(seed), &facts) {
                Fate::Fumbled { bystander, roll } => {
                    assert!(bystander < 10);
                    assert!((1..=100).contains(&roll));
                }
                Fate::Reached { roll, through, .. } => {
                    assert!(roll <= 5 && through);
                }
                Fate::Missed => panic!("a miss is decided after a fumble"),
            }
        }
    }

    #[test]
    fn a_shot_has_no_fumble_and_a_shot_that_strikes_lands_and_is_sure_if_a_ninjas() {
        // From the target's own centre, scatter is nil: it always strikes.
        for sure_throw in [false, true] {
            let facts = Reaching {
                reach: Reach::Long,
                sure_throw,
                chance: Chance { aim: -50, fumble_odds: -50 },
                ..reaching(-50, 10)
            };
            for seed in 0..50 {
                let Fate::Reached { roll, lands, through } =
                    fate(&mut GameRng::seeded(seed), &facts)
                else {
                    panic!("it strikes")
                };
                assert!(lands && (1..=100).contains(&roll));
                assert_eq!(through, sure_throw);
            }
        }
    }

    #[test]
    fn a_shot_wide_of_its_target_misses() {
        let far = Reaching {
            reach: Reach::Extreme,
            chance: Chance { aim: -100, fumble_odds: 0 },
            flight: Flight {
                from: Vec3::ZERO,
                at: Vec3::new(0.0, 0.0, -40.0),
                radius: 0.1,
                half_height: 0.1,
                blind: true,
            },
            ..reaching(-100, 0)
        };
        assert!((0..50).any(|seed| fate(&mut GameRng::seeded(seed), &far) == Fate::Missed));
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
    fn a_shot_strikes_within_the_targets_radius_and_half_height() {
        let at = Vec3::new(1.0, 1.0, 1.0);
        assert!(strikes(at, at + Vec3::new(0.3, 0.9, 0.4), 0.5, 1.0));
        assert!(!strikes(at, at + Vec3::new(0.4, 0.0, 0.4), 0.5, 1.0), "0.566 across");
        assert!(!strikes(at, at + Vec3::new(0.0, 1.1, 0.0), 0.5, 1.0));
        assert!(strikes(at, at + Vec3::new(0.0, -1.0, 0.5), 0.5, 1.0));
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

    #[test]
    fn a_monster_is_struck_by_its_species_seven_locations_and_past_them_on_its_body() {
        let facts = Landing {
            monster_locations: Some([10, 10, 10, 0, 0, 0, 0]),
            on_party: Some(EnumMap::from_fn(|_| 20)),
        };
        assert_eq!(location_at(25, &facts), Location(2));
        // Past 30: location 3.
        assert_eq!(location_at(30, &facts), Location(MONSTER_BODY));
        assert_eq!(location_at(99, &facts), Location(3));
    }

    #[test]
    fn a_member_struck_by_a_monster_is_struck_by_its_species_chances_for_five_parts() {
        let mut hits = EnumMap::default();
        hits[BodyPart::Head] = 50;
        hits[BodyPart::Foot] = 30;
        let facts = Landing { monster_locations: None, on_party: Some(hits) };
        assert_eq!(location_at(49, &facts), Location(0));
        // Body, leg and hand have none: feet from 50 to 79.
        assert_eq!(location_at(50, &facts), Location(4));
        assert_eq!(location_at(79, &facts), Location(4));
        // Past them, the body.
        assert_eq!(location_at(80, &facts), Location(BodyPart::Body as usize));
    }

    #[test]
    fn a_member_struck_by_a_member_is_struck_15_40_30_10_5() {
        let facts = Landing { monster_locations: None, on_party: None };
        assert_eq!(location_at(0, &facts), Location(0));
        assert_eq!(location_at(15, &facts), Location(1));
        assert_eq!(location_at(55, &facts), Location(2));
        assert_eq!(location_at(85, &facts), Location(3));
        assert_eq!(location_at(95, &facts), Location(4));
    }

    #[test]
    fn a_launchers_missile_pierces_by_its_own_modes() {
        let mut rng = GameRng::seeded(3);
        assert_eq!(piercing_mode(&mut rng, AttackMode::Shoot, None), AttackMode::Shoot);
        let loaded = AttackMode::Throw.into();
        assert_eq!(piercing_mode(&mut rng, AttackMode::Shoot, Some(loaded)), AttackMode::Throw);
    }

    fn piercing() -> Piercing {
        Piercing {
            rating: 80,
            swing: AttackMode::Swing,
            by: AttackMode::Swing,
            fatigue: 0,
            fatigue_pierces: false,
            victim: Defence::default(),
            surprise: Surprise::default(),
            place: Location(3),
            agility: 100,
            victim_agility: 100,
            scaling: NORMAL,
            roll: 50,
            sure: false,
        }
    }

    fn armoured(at: i32) -> Armour {
        let mut armour = Armour::default();
        armour.at[3] = at;
        armour
    }

    #[test]
    fn penetration_is_the_rating_and_the_modes_term_less_five_for_each_point_of_armour() {
        for (by, term) in [
            (AttackMode::Swing, 0),
            (AttackMode::Kick, 0),
            (AttackMode::Thrust, 10),
            (AttackMode::Shoot, 10),
            (AttackMode::Bash, -5),
            (AttackMode::Punch, -5),
            (AttackMode::Berserk, -30),
            (AttackMode::Throw, 5),
            (AttackMode::Lash, -10),
        ] {
            let facts = Piercing { by, ..piercing() };
            assert_eq!(penetration(&facts), 80 + term, "{by:?}");
        }
        let victim = Defence { armour: armoured(4), ..Defence::default() };
        // 80 - 5 * 4.
        assert_eq!(penetration(&Piercing { victim, ..piercing() }), 60);
        // Thrust: 80 + 10 - 20.
        assert_eq!(penetration(&Piercing { victim, by: AttackMode::Thrust, ..piercing() }), 70);
    }

    #[test]
    fn fatigue_takes_from_penetration_only_where_it_tells_on_it() {
        let tired = Piercing { fatigue: 20, ..piercing() };
        assert_eq!(penetration(&tired), 80);
        assert_eq!(penetration(&Piercing { fatigue_pierces: true, ..tired }), 60);
        // The full amount, not halved.
        assert_eq!(penetration(&Piercing { fatigue: 40, fatigue_pierces: true, ..piercing() }), 40);
    }

    #[test]
    fn the_victims_armour_at_the_place_counts_bracing_and_what_it_has_against_the_mode() {
        let armour = Armour { braced: (2, 0), ..armoured(4) };
        let mut by_mode = armour;
        by_mode.by_mode[AttackMode::Swing] = 1;
        let victim = Defence { armour: by_mode, defending: true, ..Defence::default() };
        // 4 + 2 braced - 1 against the mode = 5: 80 - 25.
        assert_eq!(penetration(&Piercing { victim, ..piercing() }), 55);
        // Against another mode, nothing: 4 + 2 = 6: 80 + 10 - 30.
        let facts = Piercing { victim, by: AttackMode::Thrust, ..piercing() };
        assert_eq!(penetration(&facts), 60);
    }

    #[test]
    fn the_victims_guard_counts_twice_in_penetration() {
        // Fatigue 40 on armour that tires: guard 4, twice is 8: 4 + 8 = 12.
        let tires = Armour { tires: true, ..armoured(4) };
        let victim = Defence { armour: tires, fatigue: 40, ..Defence::default() };
        assert_eq!(penetration(&Piercing { victim, ..piercing() }), 80 - 60);
        // A surprise strips 3 (twice, 6): 4 - 6 = -2: 80 + 10.
        let exposed = Armour { exposed: 3, ..armoured(4) };
        let victim = Defence { armour: exposed, ..Defence::default() };
        let unaware = Surprise { unaware: true, natural: false };
        assert_eq!(penetration(&Piercing { victim, surprise: unaware, ..piercing() }), 90);
        // On the move 4 less, twice is 8 (not against a throw): 4 - 8 = -4.
        let moving = Defence { armour: armoured(4), moving: true, ..Defence::default() };
        assert_eq!(penetration(&Piercing { victim: moving, ..piercing() }), 100);
        let throw = Piercing { victim: moving, by: AttackMode::Throw, ..piercing() };
        assert_eq!(penetration(&throw), 80 + 5 - 20);
    }

    #[test]
    fn the_difference_of_agility_in_penetration_is_with_the_actual_victim() {
        assert_eq!(penetration(&Piercing { victim_agility: 60, ..piercing() }), 84);
        assert_eq!(penetration(&Piercing { agility: 70, ..piercing() }), 77);
    }

    #[test]
    fn penetration_is_scaled_for_difficulty_by_the_attackers_side_without_the_blind_rule() {
        let scaling = Scaling { difficulty: Difficulty::Novice, side: Stance::Hostile };
        assert_eq!(penetration(&Piercing { scaling, ..piercing() }), 48);
        let scaling = Scaling { difficulty: Difficulty::Expert, side: Stance::Hostile };
        assert_eq!(penetration(&Piercing { scaling, ..piercing() }), 112);
    }

    #[test]
    fn a_swing_gets_through_when_its_penetration_is_at_least_the_roll() {
        let at = |roll| gets_through(&mut GameRng::seeded(0), &Piercing { roll, ..piercing() });
        assert!(at(80));
        assert!(!at(81));
        assert!(at(1));
    }

    #[test]
    fn a_sure_swing_gets_through_whatever_the_penetration() {
        let facts = Piercing { rating: -100, roll: 100, sure: true, ..piercing() };
        assert!(gets_through(&mut GameRng::seeded(0), &facts));
        let unsure = Piercing { sure: false, ..facts };
        assert!(!gets_through(&mut GameRng::seeded(0), &unsure));
    }

    #[test]
    fn a_missile_shield_turns_aside_a_shot_or_a_throw_with_chance_equal_to_it() {
        // A roll below the shield.
        assert!(turned_aside(AttackMode::Shoot, 30, 29));
        assert!(!turned_aside(AttackMode::Shoot, 30, 30));
        assert!(turned_aside(AttackMode::Throw, 100, 99));
        assert!(!turned_aside(AttackMode::Throw, 0, 0));
        // Not blows struck close.
        assert!(!turned_aside(AttackMode::Swing, 100, 0));
        assert!(!turned_aside(AttackMode::Thrust, 100, 0));
    }

    #[test]
    fn a_shield_of_100_stops_every_shot_even_a_sure_one_and_no_close_blow() {
        let armour = Armour { missile_shield: 100, ..Armour::default() };
        let victim = Defence { armour, ..Defence::default() };
        let shot = Piercing {
            victim,
            swing: AttackMode::Shoot,
            by: AttackMode::Shoot,
            sure: true,
            ..piercing()
        };
        for seed in 0..50 {
            assert!(!gets_through(&mut GameRng::seeded(seed), &shot));
            let close = Piercing { swing: AttackMode::Swing, ..shot };
            assert!(gets_through(&mut GameRng::seeded(seed), &close));
        }
    }
}
