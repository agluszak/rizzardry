//! Immutable content shared by every campaign session.

pub mod characters;
pub mod items;
pub mod level;
pub mod levels;
pub mod monster;
pub mod music;
pub mod navmesh;
pub mod npcs;
pub mod paths;
pub mod species;
pub mod spells;
pub mod surface;
pub mod ui;

/// Retail length units in one metre: retail's own `kWorldScale`. Authored
/// values (`.mls` monster sizes, `pathparms.txt`, positions stored in level
/// records) are metres multiplied by 500, and the rules quote distances in
/// them: the party eye is 1000 units (2 m) above the feet.
pub const UNITS_PER_METRE: f32 = 500.0;

/// The party's eye above its feet, in metres (retail 1000 units).
pub const EYE_HEIGHT: f32 = 2.0;

pub mod particles;
