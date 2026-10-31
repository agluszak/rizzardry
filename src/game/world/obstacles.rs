//! What the world's solid scenery blocks: sight (perception, the radar,
//! searching) and a thrown item's flight and landing.
//!
//! Scenery is the level's own collision and every prop part the party cannot
//! walk through, whether it stands still or moves (a door, a lift): those
//! carry the [`Layer::Scenery`] collision layer. Props the party walks
//! through (sensors) block neither sight nor items, and nor do the party or
//! monsters. A body's mobility plays no part. Retail traces sight through its
//! octree; whether that octree holds props is not established, so props
//! here count by whether they physically block.
use avian3d::prelude::{
    CollisionLayers, LayerMask, PhysicsLayer, RayHitData, SpatialQuery, SpatialQueryFilter,
};
use bevy::{ecs::system::SystemParam, prelude::*};

/// The world's collision layers.
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum Layer {
    #[default]
    Default,
    /// Solid scenery: the level's collision and solid prop parts.
    Scenery,
}

/// Solid scenery's layers: its own, beside the default one every body has,
/// so that what collides with it is unchanged.
pub fn scenery() -> CollisionLayers {
    CollisionLayers::new([Layer::Default, Layer::Scenery], LayerMask::ALL)
}

#[derive(SystemParam)]
pub struct WorldObstacles<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
}

impl WorldObstacles<'_, '_> {
    /// The first solid scenery along the ray.
    pub fn first_hit(&self, origin: Vec3, direction: Dir3, reach: f32) -> Option<RayHitData> {
        self.spatial.cast_ray(
            origin,
            direction,
            reach,
            true,
            &SpatialQueryFilter::from_mask(Layer::Scenery),
        )
    }

    /// Whether nothing solid stands between two points: a line of sight,
    /// or a clear path for something flying. A point is clear of itself.
    pub fn clear(&self, from: Vec3, to: Vec3) -> bool {
        Dir3::new(to - from)
            .map_or(true, |direction| self.first_hit(from, direction, from.distance(to)).is_none())
    }

    /// The ground a spot stands on: looking down from a step above it, at
    /// most `depth` below it.
    pub fn ground_below(&self, spot: Vec3, depth: f32) -> Option<Vec3> {
        let start = spot + Vec3::Y * super::STEP_HEIGHT;
        let hit = self.first_hit(start, Dir3::NEG_Y, super::STEP_HEIGHT + depth)?;
        Some(start - Vec3::Y * hit.distance)
    }
}
