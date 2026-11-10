//! The rules of an attack with a weapon (`docs/rules/attack.md`), each a
//! function of the facts it is about and nothing else: no query, no world, no
//! log. A rule that rolls takes the random-number generator; none changes
//! anything. [`super::swings`] reads the facts, calls the rules in the order
//! of the page and makes happen what they decide.
//!
//! - [`choose`]: whom and with what to strike, or what to do instead (2)
//! - [`unaware`]: catching the target unaware (3)
//! - [`aim`]: the roll to hit, a swing's fate, getting through armour (4)
//! - [`guards`]: who steps in (4)
//! - [`blow`]: the damage of a blow that lands, and Razor Cloak's answer (5, 7)
//! - [`cost`]: what a swing costs and uses up, and the next attack (6, 7)
//!
//! What a member strikes with is [`crate::game::actors::arms`]. A missile's
//! flight is not simulated: where a shot lands is retail's scatter around its
//! aim point, and it strikes its target when that point falls within the
//! target's body. Retail flies the shot and can hit whatever else it meets on
//! the way.

mod aim;
mod blow;
mod choose;
mod cost;
mod guards;
mod unaware;

pub use aim::*;
pub use blow::*;
pub use choose::*;
pub use cost::*;
pub use guards::*;
pub use unaware::*;

use bevy::prelude::Component;
use enumset::EnumSet;

use crate::content::items::{AttackMode, Dice};
use crate::content::species::Reach;
use crate::game::characters::Vitals;
use crate::game::magic::Strength;
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

/// Whether an arm of this reach shoots rather than closes in: long and
/// extreme reach (retail range category).
pub fn is_missile(reach: Reach) -> bool {
    reach >= Reach::Long
}

/// The mode a draw `wanted` below 9 comes to: count `wanted` modes along the
/// arm's, going round from the first (retail `ChooseAttackMode`). Not uniform
/// when the count does not divide 9: two modes, the first 5/9 of the time.
pub fn mode_at(modes: EnumSet<AttackMode>, wanted: u32) -> AttackMode {
    modes.iter().cycle().nth(wanted as usize).unwrap_or(AttackMode::Swing)
}

/// Pick one of the modes (retail `ChooseAttackMode`).
pub fn choose_mode(rng: &mut GameRng, modes: EnumSet<AttackMode>) -> AttackMode {
    mode_at(modes, rng.below(9))
}

/// A difference of agility, as it weighs in a blow's aim and penetration:
/// a tenth of what the attacker has over the one struck.
pub fn agility_difference(attacker: i32, target: i32) -> i32 {
    (attacker - target) / 10
}

/// A party member who is not under Turncoat provokes the group of whom it
/// strikes as the attack starts (retail `MakeTargetGroupHostile`).
pub fn provokes(member: bool, turncoat: bool) -> bool {
    member && !turncoat
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

/// Roll `dice` `count` times (retail `RollDice` with a roll count).
pub fn roll_dice(rng: &mut GameRng, dice: Dice, count: u32) -> u32 {
    (0..count).map(|_| rng.roll(dice)).sum()
}

/// The strength of a blow's weapon effects (retail effect power level): the
/// attacker's level and as much again up to 15. It is both what they are
/// rolled at and what the victim's level is weighed against.
pub fn effect_strength(level: u32) -> Strength {
    let power = level + if level > 14 { 15 } else { level };
    Strength { power, against: power, boost: 0 }
}

/// When an actor acts in the round, 1 (first) to 100: a d4 plus its
/// initiative, highest first.
pub fn phase(rng: &mut GameRng, initiative: i32) -> u32 {
    let v = rng.below(4) as i32 + initiative;
    (90 - v.clamp(-10, 89)) as u32
}

/// One swing of an attack, for the attack's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swung {
    /// It reached a victim, other than one gone astray, whether or not it
    /// got through armour or did damage.
    pub hit: bool,
    /// The health lost, after Guardian Angel, to a blow on the intended or
    /// intercepting victim.
    pub damage: u32,
}

/// What an attack came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Totals {
    pub hits: u32,
    pub damage: u32,
}

/// What the swings of an attack came to: `hits` counts those that reached
/// a victim, `damage` the health they took.
pub fn tally(swung: &[Swung]) -> Totals {
    Totals {
        hits: swung.iter().filter(|swing| swing.hit).count() as u32,
        damage: swung.iter().map(|swing| swing.damage).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body with 100 stamina of which `left` remains.
    fn tired(left: u32) -> Vitals {
        let mut vitals = Vitals::full(20, 100);
        vitals.stamina.current = left;
        vitals
    }

    #[test]
    fn fatigue_grows_in_bands_of_the_share_of_stamina_spent() {
        // Spent is 100 - left. Page 4: under 50: 0; 50 to 69: 5; 70 to 84: 10;
        // 85 to 94: 20; 95 or more: 40.
        for (left, lost) in [
            (100, 0),
            (51, 0),  // 49 spent
            (50, 5),  // 50 spent
            (31, 5),  // 69 spent
            (30, 10), // 70 spent
            (16, 10), // 84 spent
            (15, 20), // 85 spent
            (6, 20),  // 94 spent
            (5, 40),  // 95 spent
            (0, 40),
        ] {
            assert_eq!(fatigue(&tired(left)), lost, "{left} left");
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
}
