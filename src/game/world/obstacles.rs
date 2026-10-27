//! Static world geometry queries shared by perception and item placement.
use avian3d::prelude::{RayHitData, RigidBody, SpatialQuery, SpatialQueryFilter};
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub struct WorldObstacles<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    bodies: Query<'w, 's, &'static RigidBody>,
}

impl WorldObstacles<'_, '_> {
    /// Callers retain their own range, clearance and degenerate-ray policy.
    pub fn first_hit(&self, origin: Vec3, direction: Dir3, reach: f32) -> Option<RayHitData> {
        self.spatial.cast_ray_predicate(
            origin,
            direction,
            reach,
            true,
            &SpatialQueryFilter::default(),
            &|entity| self.bodies.get(entity).is_ok_and(RigidBody::is_static),
        )
    }
}
