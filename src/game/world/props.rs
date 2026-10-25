//! Usable level props: doors, lids, levers and lifts that move along their
//! authored paths.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::level::PropPart;

/// A placed prop. Its parts are children carrying [`PropPart`].
#[derive(Component, Debug)]
pub struct Prop {
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
fn pose(keys: &[Transform], frame: f32) -> Transform {
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

/// The prop an entity belongs to: itself or an ancestor.
pub fn owning_prop(
    mut entity: Entity,
    props: &Query<(), With<Prop>>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    loop {
        if props.contains(entity) {
            return Some(entity);
        }
        entity = parents.get(entity).ok()?.parent();
    }
}

/// The farthest the party picks things, in metres; each trigger's own range
/// decides whether it can be used from there.
const PICK_DISTANCE: f32 = 50.0;

/// The first prop hit by a ray, and how far along the ray it is.
pub fn prop_under_ray(
    ray: Ray3d,
    exclude: Entity,
    spatial: &SpatialQuery,
    props: &Query<(), With<Prop>>,
    parents: &Query<&ChildOf>,
) -> Option<(Entity, f32)> {
    let filter = SpatialQueryFilter::default().with_excluded_entities([exclude]);
    let hit = spatial.cast_ray(ray.origin, ray.direction, PICK_DISTANCE, true, &filter)?;
    Some((owning_prop(hit.entity, props, parents)?, hit.distance))
}
