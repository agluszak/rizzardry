//! Gameplay randomness: one seeded, saveable generator, so the same seed and
//! the same choices give the same game. Presentation draws its own
//! randomness elsewhere.
//!
//! The algorithm is PCG32 (O'Neill's PCG-XSH-RR, 64-bit state). Retail's own
//! generator and draw order are not reproduced.

use bevy::prelude::*;

use crate::content::items::Dice;

#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct GameRng {
    state: u64,
    increment: u64,
}

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

impl GameRng {
    pub fn seeded(seed: u64) -> Self {
        let mut rng = GameRng { state: 0, increment: (seed << 1) | 1 };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULTIPLIER).wrapping_add(self.increment);
        let shifted = (((old >> 18) ^ old) >> 27) as u32;
        shifted.rotate_right((old >> 59) as u32)
    }

    /// A number in `1..=sides`, without modulo bias.
    pub fn die(&mut self, sides: u8) -> u32 {
        let sides = u32::from(sides.max(1));
        let zone = u32::MAX - u32::MAX % sides;
        loop {
            let draw = self.next_u32();
            if draw < zone {
                return draw % sides + 1;
            }
        }
    }

    /// `count` d `sides` + `base`, never below 0.
    pub fn roll(&mut self, dice: Dice) -> u32 {
        let thrown: u32 = (0..dice.count).map(|_| self.die(dice.sides)).sum();
        (i64::from(thrown) + i64::from(dice.base)).max(0) as u32
    }
}

impl Default for GameRng {
    fn default() -> Self {
        GameRng::seeded(0x5743_5249_5a41_5244)
    }
}

impl Dice {
    /// The largest result: every die at its maximum.
    pub fn maximum(self) -> u32 {
        (i64::from(self.base) + i64::from(self.count) * i64::from(self.sides)).max(0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_rolls_within_bounds() {
        let dice = Dice { count: 2, sides: 6, base: 1 };
        let (mut a, mut b) = (GameRng::seeded(7), GameRng::seeded(7));
        let rolls: Vec<u32> = (0..200).map(|_| a.roll(dice)).collect();
        assert_eq!(rolls, (0..200).map(|_| b.roll(dice)).collect::<Vec<_>>());
        assert!(rolls.iter().all(|r| (3..=13).contains(r)));
        assert!(rolls.contains(&3) && rolls.contains(&13), "both extremes appear");
        assert_ne!(GameRng::seeded(8).next_u32(), GameRng::seeded(7).next_u32());
        assert_eq!(dice.maximum(), 13);
    }
}
