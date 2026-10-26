//! One mesh-selection policy shared by hover and activation. Physics is used
//! only to find a dropped item's landing point, never to replace mesh picking.

use super::party_view::PartyCamera;
use crate::game::inventory::PartyInventory;
use crate::game::party::body::Interact;
use crate::game::world::items::{CLEARANCE, THROW, WorldItem, drop_held, take_item};
use crate::game::world::level::Level;
use crate::game::world::population::Monster;
use crate::game::world::props::Prop;
use crate::game::world::triggers::{self, Device};
use avian3d::prelude::{RigidBody, SpatialQuery, SpatialQueryFilter};
use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct WorldHit {
    pub entity: Entity,
    pub distance: f32,
    pub position: Vec3,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct WorldSelection {
    pub level: Option<Entity>,
    pub ray: Option<Ray3d>,
    pub hit: Option<WorldHit>,
}

pub fn select_world_target(
    freedom: crate::game::screen::Freedom,
    pointers: Query<(&PointerId, &PointerLocation)>,
    camera: Query<(&Camera, &GlobalTransform), With<PartyCamera>>,
    mut meshes: MeshRayCast,
    sky: Query<(), With<RenderLayers>>,
    parents: Query<&ChildOf>,
    owners: Query<(), Or<(With<Prop>, With<WorldItem>, With<Monster>)>>,
    levels: Query<Entity, With<Level>>,
    mut selection: ResMut<WorldSelection>,
) {
    *selection = WorldSelection::default();
    if !freedom.free() {
        return;
    }
    let Ok((camera, transform)) = camera.single() else { return };
    let Some(ray) = ray_under_pointer(&pointers, camera, transform) else { return };
    selection.level = levels.single().ok();
    selection.ray = Some(ray);
    selection.hit = drawn_under_ray(ray, &mut meshes, &sky).and_then(|(hit, distance)| {
        Some(WorldHit {
            entity: owner(hit, |e| owners.contains(e), &parents)?,
            distance,
            position: ray.get_point(distance),
        })
    });
}

pub fn activate_selection(
    interact: Single<&ActionEvents, With<Action<Interact>>>,
    selection: Res<WorldSelection>,
    props: Query<Has<Device>, With<Prop>>,
    held: Res<PartyInventory>,
    spatial: SpatialQuery,
    bodies: Query<&RigidBody>,
    mut commands: Commands,
) {
    if !interact.contains(ActionEvents::START) {
        return;
    }
    if let Some(hit) = selection.hit
        && let Ok(device) = props.get(hit.entity)
    {
        if device {
            commands.queue(move |world: &mut World| {
                triggers::activate(world, hit.entity, Some(hit.distance));
            });
        }
        return;
    }
    if held.held.is_some() {
        let (Some(level), Some(ray)) = (selection.level, selection.ray) else { return };
        let solid = |entity| bodies.get(entity).is_ok_and(RigidBody::is_static);
        if let Some(at) = drop_point(&spatial, ray, &solid) {
            commands.queue(move |world: &mut World| {
                let _ = drop_held(world, level, at);
            });
        }
    } else if let Some(hit) = selection.hit {
        commands.queue(move |world: &mut World| {
            let _ = take_item(world, hit.entity, hit.distance);
        });
    }
}

/// Retail stops the throw short of a collision and probes the ground below it.
fn drop_point(spatial: &SpatialQuery, ray: Ray3d, solid: &dyn Fn(Entity) -> bool) -> Option<Vec3> {
    let filter = SpatialQueryFilter::default();
    let reach = spatial
        .cast_ray_predicate(ray.origin, ray.direction, THROW, true, &filter, solid)
        .map_or(THROW, |hit| hit.distance);
    let above = ray.get_point((reach - CLEARANCE).max(0.0));
    let ground = spatial.cast_ray_predicate(above, Dir3::NEG_Y, 100.0, true, &filter, solid)?;
    Some(above - Vec3::Y * (ground.distance - CLEARANCE))
}

/// What an entity belongs to: itself or the nearest ancestor that
/// `is_owner`.
pub fn owner(
    mut entity: Entity,
    is_owner: impl Fn(Entity) -> bool,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    loop {
        if is_owner(entity) {
            return Some(entity);
        }
        entity = parents.get(entity).ok()?.parent();
    }
}

/// The ray through the mouse pointer, when it is over the view (retail
/// picks props and items under the cursor). Both the pointer and the
/// camera's viewport rectangle are in logical pixels of the window, which
/// is what `viewport_to_world` takes.
pub fn ray_under_pointer(
    pointers: &Query<(&PointerId, &PointerLocation)>,
    camera: &Camera,
    transform: &GlobalTransform,
) -> Option<Ray3d> {
    let (_, location) = pointers.iter().find(|(id, _)| id.is_mouse())?;
    let position = location.location()?.position;
    let view = camera.logical_viewport_rect()?;
    if !view.contains(position) {
        return None;
    }
    camera.viewport_to_world(transform, position).ok()
}

/// The mesh drawn first along a ray, and how far along the ray it is. Like
/// retail, this picks what is drawn, not what collides: a pressed pedal
/// below the floor's collider is still there to click.
pub fn drawn_under_ray(
    ray: Ray3d,
    meshes: &mut MeshRayCast,
    sky: &Query<(), With<RenderLayers>>,
) -> Option<(Entity, f32)> {
    // The sky world is drawn by its own camera, around the world origin.
    let not_sky = |entity| !sky.contains(entity);
    let settings = MeshRayCastSettings::default().with_filter(&not_sky);
    let &(entity, ref hit) = meshes.cast_ray(ray, &settings).first()?;
    Some((entity, hit.distance))
}
