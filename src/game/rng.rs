//! Gameplay randomness: one seeded generator, so the same seed and the same
//! choices give the same game. ChaCha8 is portable and stable across
//! versions, so a seed means the same game everywhere. Presentation has its
//! own generator (`presentation::PresentationRng`) and never draws from this
//! one. Retail's own generator and draw order are not reproduced.

use bevy::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::content::items::Dice;

#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct GameRng(ChaCha8Rng);

impl GameRng {
    pub fn seeded(seed: u64) -> Self {
        GameRng(ChaCha8Rng::seed_from_u64(seed))
    }

    /// A number in `0..n`; 0 when `n` is 0.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.0.random_range(0..n) }
    }

    /// A number in `1..=sides`.
    pub fn die(&mut self, sides: u8) -> u32 {
        self.0.random_range(1..=u32::from(sides.max(1)))
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
        assert_ne!(GameRng::seeded(8).below(1 << 30), GameRng::seeded(7).below(1 << 30));
        assert_eq!(dice.maximum(), 13);
    }
}
