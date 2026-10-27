//! Bevy's CPU raycaster does not yet deform morph meshes. Sample the native
//! mesh and weights only under the picking ray; never maintain another Mesh.
use bevy::camera::{primitives::Aabb, visibility::RenderLayers};
use bevy::ecs::system::SystemParam;
use bevy::mesh::{
    Indices, PrimitiveTopology, VertexAttributeValues,
    morph::{MeshMorphWeights, MorphWeights},
};
use bevy::picking::mesh_picking::ray_cast::{
    Backfaces, RayCastBackfaces, ray_aabb_intersection_3d, ray_mesh_intersection,
};
use bevy::prelude::*;

#[derive(SystemParam)]
pub struct MorphRayCast<'w, 's> {
    meshes: Res<'w, Assets<Mesh>>,
    parts: Query<
        'w,
        's,
        (
            Entity,
            &'static Mesh3d,
            &'static MeshMorphWeights,
            &'static GlobalTransform,
            &'static Aabb,
            &'static ViewVisibility,
            Has<RayCastBackfaces>,
        ),
        Without<RenderLayers>,
    >,
    weights: Query<'w, 's, &'static MorphWeights>,
    positions: Local<'s, Vec<[f32; 3]>>,
}

impl MorphRayCast<'_, '_> {
    pub fn contains(&self, entity: Entity) -> bool {
        self.parts.contains(entity)
    }

    pub fn nearest(
        &mut self,
        ray: Ray3d,
        mut nearest: Option<(Entity, f32)>,
    ) -> Option<(Entity, f32)> {
        for (entity, handle, morph, transform, bounds, visible, backfaces) in &self.parts {
            if !visible.get() {
                continue;
            }
            let transform = transform.affine();
            let Some(near) = ray_aabb_intersection_3d(
                ray,
                &Aabb3d::new(bounds.center, bounds.half_extents),
                &transform,
            ) else {
                continue;
            };
            if nearest.is_some_and(|(_, distance)| near > distance) {
                continue;
            }
            let weights = match morph {
                MeshMorphWeights::Value { weights } => weights.as_slice(),
                MeshMorphWeights::Reference(source) => {
                    let Ok(weights) = self.weights.get(*source) else { continue };
                    weights.weights()
                }
            };
            let Some(mesh) = self.meshes.get(&handle.0) else { continue };
            if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
                continue;
            }
            let Some(VertexAttributeValues::Float32x3(base)) =
                mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()
            else {
                continue;
            };
            let Ok(targets) = mesh.try_morph_targets() else { continue };
            self.positions.clone_from(base);
            for (target, &weight) in weights.iter().enumerate().filter(|(_, w)| **w != 0.) {
                let Some(deltas) = targets.get(target * base.len()..(target + 1) * base.len())
                else {
                    continue;
                };
                for (position, delta) in self.positions.iter_mut().zip(deltas) {
                    *position = (Vec3::from(*position) + delta.position * weight).to_array();
                }
            }
            let backfaces = if backfaces { Backfaces::Include } else { Backfaces::Cull };
            let hit = match mesh.try_indices().ok() {
                Some(Indices::U16(indices)) => ray_mesh_intersection(
                    ray,
                    &transform,
                    &self.positions,
                    None,
                    Some(indices.as_slice()),
                    None,
                    backfaces,
                ),
                Some(Indices::U32(indices)) => ray_mesh_intersection(
                    ray,
                    &transform,
                    &self.positions,
                    None,
                    Some(indices.as_slice()),
                    None,
                    backfaces,
                ),
                None => ray_mesh_intersection::<u32>(
                    ray,
                    &transform,
                    &self.positions,
                    None,
                    None,
                    None,
                    backfaces,
                ),
            };
            if let Some(hit) = hit
                && nearest.is_none_or(|(_, distance)| hit.distance < distance)
            {
                nearest = Some((entity, hit.distance));
            }
        }
        nearest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::ecs::system::SystemState;
    use bevy::mesh::morph::MorphAttributes;

    fn triangle() -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        mesh
    }
    fn hit(world: &mut World, ray: Ray3d) -> Option<(Entity, f32)> {
        use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
        let mut state: SystemState<(MeshRayCast, MorphRayCast, Query<(), With<RenderLayers>>)> =
            SystemState::new(world);
        let (mut meshes, mut morphs, sky) = state.get_mut(world).unwrap();
        crate::presentation::selection::drawn_under_ray(ray, &mut meshes, &sky, &mut morphs)
    }

    #[test]
    fn native_weights_move_picking_outside_the_base_pose_without_hiding_nearer_geometry() {
        let mut app = App::new();
        app.add_plugins(bevy::app::TaskPoolPlugin::default());
        let world = app.world_mut();
        world.init_resource::<Assets<Mesh>>();
        let mut mesh = triangle();
        mesh.set_morph_targets(vec![MorphAttributes::new(Vec3::X * 3., Vec3::ZERO, Vec3::ZERO); 3]);
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let source = world.spawn(MorphWeights::new(vec![1.], Some(handle.clone())).unwrap()).id();
        let visible = ViewVisibility::VISIBLE;
        let actor = world
            .spawn((
                Mesh3d(handle),
                MeshMorphWeights::Reference(source),
                GlobalTransform::IDENTITY,
                InheritedVisibility::VISIBLE,
                visible,
                Aabb::from_min_max(Vec3::new(-1., -1., 0.), Vec3::new(4., 1., 0.)),
            ))
            .id();
        let moved = Ray3d::new(Vec3::new(3., 0., 2.), Dir3::NEG_Z);
        assert_eq!(hit(world, moved), Some((actor, 2.)));
        assert!(
            hit(world, Ray3d::new(Vec3::new(0., 0., 2.), Dir3::NEG_Z)).is_none(),
            "base pose must not be picked"
        );
        let wall_mesh = world.resource_mut::<Assets<Mesh>>().add(triangle());
        let wall = world
            .spawn((
                Mesh3d(wall_mesh),
                GlobalTransform::from_translation(Vec3::new(3., 0., 1.)),
                InheritedVisibility::VISIBLE,
                visible,
                Aabb::from_min_max(Vec3::new(-1., -1., 0.), Vec3::new(1., 1., 0.)),
            ))
            .id();
        assert_eq!(hit(world, moved), Some((wall, 1.)));
        world.despawn(wall);
        world.get_mut::<MorphWeights>(source).unwrap().weights_mut()[0] = 0.;
        assert!(hit(world, moved).is_none());
        assert_eq!(hit(world, Ray3d::new(Vec3::new(0., 0., 2.), Dir3::NEG_Z)), Some((actor, 2.)));
    }
}
