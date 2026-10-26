//! Gameplay state and rules.

pub mod automap;
pub mod camp;
pub mod characters;
pub mod combat;
pub mod data;
pub mod inventory;
pub mod loot;
pub mod party;
pub mod rng;
pub mod screen;
pub mod time;
pub mod world;

/// Only the boundaries that share gameplay randomness or require accepted
/// state from an earlier operation. Presentation remains independently scheduled.
#[derive(bevy::prelude::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameplaySet {
    Equipment,
    LevelActivation,
    Combat,
}
