//! The party's view of the world: the camera on its body, aimed by its
//! facing, and what the pointer is over in it. A click in the view becomes
//! one gameplay request, [`WorldClick`].

use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use crate::content::UNITS_PER_METRE;
use crate::content::level::DEFAULT_VIEW_DISTANCE;
use crate::game::party::body::{EYE_ABOVE_CENTRE, Interact, PartyBody};
use crate::game::world::interaction::WorldClick;
use crate::game::world::items::WorldItem;
use crate::game::world::props::Prop;

/// The first-person camera on the party's body.
#[derive(Component, Debug)]
pub struct PartyCamera;

/// Put the camera on the party's body when it appears.
pub fn attach_camera(mut commands: Commands, party: Query<Entity, Added<PartyBody>>) {
    for party in &party {
        commands.spawn((
            Name::new("Party camera"),
            PartyCamera,
            SpatialListener::new(0.2),
            Camera3d::default(),
            // The sky camera clears the frame and draws first.
            Camera { clear_color: ClearColorConfig::None, ..default() },
            Projection::Perspective(PerspectiveProjection {
                fov: 70f32.to_radians(),
                // Retail's near clip of 250 units; the level sets the far one.
                near: 250.0 / UNITS_PER_METRE,
                far: DEFAULT_VIEW_DISTANCE,
                ..default()
            }),
            Transform::from_xyz(0.0, EYE_ABOVE_CENTRE, 0.0),
            ChildOf(party),
        ));
    }
}

/// Point the camera along the party's facing.
pub fn aim_camera(party: Query<&PartyBody>, mut camera: Query<&mut Transform, With<PartyCamera>>) {
    if let (Ok(body), Ok(mut camera)) = (party.single(), camera.single_mut()) {
        camera.rotation = Quat::from_rotation_y(body.yaw) * Quat::from_rotation_x(body.pitch);
    }
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

/// A click in the view: the prop or item drawn first under the pointer, if
/// any, for gameplay to act on.
pub fn click_in_view(
    interact: Single<&ActionEvents, With<Action<Interact>>>,
    pointers: Query<(&PointerId, &PointerLocation)>,
    camera: Query<(&Camera, &GlobalTransform), With<PartyCamera>>,
    mut meshes: MeshRayCast,
    (sky, parents): (Query<(), With<RenderLayers>>, Query<&ChildOf>),
    (props, items): (Query<(), With<Prop>>, Query<(), With<WorldItem>>),
    mut clicks: MessageWriter<WorldClick>,
) {
    if !interact.contains(ActionEvents::START) {
        return;
    }
    let Ok((camera, transform)) = camera.single() else { return };
    let Some(ray) = ray_under_pointer(&pointers, camera, transform) else { return };
    let under = drawn_under_ray(ray, &mut meshes, &sky).and_then(|(hit, distance)| {
        let thing = owner(hit, |e| props.contains(e) || items.contains(e), &parents)?;
        Some((thing, distance))
    });
    clicks.write(WorldClick { ray, under });
}
