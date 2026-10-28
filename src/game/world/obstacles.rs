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
    /// The first solid scenery along the ray. Callers retain their own range,
    /// clearance and degenerate-ray policy.
    pub fn first_hit(&self, origin: Vec3, direction: Dir3, reach: f32) -> Option<RayHitData> {
        self.spatial.cast_ray(
            origin,
            direction,
            reach,
            true,
            &SpatialQueryFilter::from_mask(Layer::Scenery),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avian3d::prelude::*;
    use bevy::ecs::system::RunSystemOnce;

    /// A wall and a moving door block; a prop to walk through and a body do not.
    #[test]
    fn scenery_blocks_whether_it_moves_and_walk_through_props_and_bodies_do_not() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::default(),
        ));
        app.finish();
        app.cleanup();
        let slab = || Collider::cuboid(4.0, 4.0, 0.2);
        let at = |z: f32| Transform::from_xyz(0.0, 0.0, z);
        let wall = app.world_mut().spawn((RigidBody::Static, slab(), scenery(), at(-10.0))).id();
        let door = app.world_mut().spawn((RigidBody::Kinematic, slab(), scenery(), at(-6.0))).id();
        let curtain = app.world_mut().spawn((RigidBody::Static, slab(), Sensor, at(-4.0))).id();
        let body = app.world_mut().spawn((RigidBody::Dynamic, slab(), at(-2.0))).id();
        app.update();
        app.update();
        let hit = |app: &mut App| {
            app.world_mut()
                .run_system_once(|obstacles: WorldObstacles| {
                    obstacles.first_hit(Vec3::ZERO, Dir3::NEG_Z, 20.0).map(|hit| hit.entity)
                })
                .unwrap()
        };
        assert_eq!(hit(&mut app), Some(door));
        app.world_mut().despawn(door);
        app.update();
        assert_eq!(hit(&mut app), Some(wall));
        assert!(app.world().get_entity(curtain).is_ok() && app.world().get_entity(body).is_ok());
    }
}
