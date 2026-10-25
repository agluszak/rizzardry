//! Usable level props: doors, lids, levers and lifts that move along their
//! authored paths.

use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::picking::pointer::{PointerId, PointerLocation};
use bevy::prelude::*;

use super::level::PropPart;

/// A placed prop. Its parts are children carrying [`PropPart`].
#[derive(Component, Debug)]
pub struct Prop {
    /// Its place among the level's props (`LevelGeometry::props`), which
    /// identifies it within the level.
    pub definition: usize,
    /// The trigger using the prop runs.
    pub trigger: Option<String>,
    /// Its trigger guards a doorway (navigation links follow it).
    pub door: bool,
    pub frames_per_second: f32,
    /// The highest path key of any part.
    pub last_frame: usize,
    /// Each play starts from the first frame instead of reversing the last.
    pub wraps: bool,
    /// Path keys splitting the animation into segments (a dial).
    pub slots: Vec<u16>,
}

/// Where a prop is along its path and where it is heading.
#[derive(Component, Debug, Default)]
pub struct PropMotion {
    pub frame: f32,
    pub target: f32,
}

/// The pose at a fractional `frame` along `keys`.
pub fn pose(keys: &[Transform], frame: f32) -> Transform {
    let last = keys.len() - 1;
    let frame = frame.clamp(0.0, last as f32);
    let (index, t) = (frame.floor() as usize, frame.fract());
    if index >= last {
        return keys[last];
    }
    let (a, b) = (keys[index], keys[index + 1]);
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation: a.rotation.slerp(b.rotation, t),
        scale: a.scale.lerp(b.scale, t),
    }
}

/// Move props toward their target frame on the simulation step, carrying
/// their parts (and the parts' colliders) along the authored paths.
pub fn animate_props(
    time: Res<Time>,
    mut props: Query<(&Prop, &mut PropMotion, &Children)>,
    mut parts: Query<(&PropPart, &mut Transform)>,
) {
    for (prop, mut motion, children) in &mut props {
        if motion.frame == motion.target {
            continue;
        }
        let step = prop.frames_per_second * time.delta_secs();
        motion.frame = if motion.target > motion.frame {
            (motion.frame + step).min(motion.target)
        } else {
            (motion.frame - step).max(motion.target)
        };
        for child in children {
            if let Ok((part, mut transform)) = parts.get_mut(*child)
                && !part.keys.is_empty()
            {
                *transform = pose(&part.keys, motion.frame);
            }
        }
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
/// below the floor's collider is still there to click. Each trigger's own
/// range decides whether it can be used from that far.
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
