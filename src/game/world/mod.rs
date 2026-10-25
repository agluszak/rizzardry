//! The active level and its local objects.

pub mod level;
pub mod navigation;

use bevy::prelude::*;

/// The highest step the party and walking monsters climb, in metres.
///
/// Retail has no explicit step height: it slides a 500-unit (0.83 m) sphere,
/// which rides over edges well above the first 0.35 m guess. At 0.35 m the
/// upper monastery's navmesh split into 22 pieces, the largest 61% of the
/// walkable area; at 0.5 m one piece covers 98%. Raising it to 1 m merged
/// nothing further. Characterize against retail stairs when collision
/// surfaces are decoded.
pub const STEP_HEIGHT: f32 = 0.5;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(navigation::NavigationPlugin)
            .add_message::<level::LevelReady>()
            .add_systems(Update, level::spawn_active_level);
    }
}
