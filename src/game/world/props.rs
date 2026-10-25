//! Usable level props: doors, lids, levers and lifts that move along their
//! authored paths.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::level::PropPart;
use crate::content::level::PropInteraction;

/// Retail trigger action codes a prop can run when used.
mod action {
    pub const DOOR_OPEN: u32 = 0x01;
    pub const DOOR_TOGGLE: u32 = 0x02;
    pub const ANIMATION_TOGGLE: u32 = 0x03;
    pub const DOOR_TOGGLE_WITH_KEY: u32 = 0x2c;
    pub const ANIMATION_START: u32 = 0x32;
    pub const ANIMATION_STOP: u32 = 0x33;
}

/// A placed prop. Its parts are children carrying [`PropPart`].
#[derive(Component, Debug)]
pub struct Prop {
    pub interaction: Option<PropInteraction>,
    pub frames_per_second: f32,
    /// The highest path key of any part.
    pub last_frame: usize,
    uses: u32,
}

impl Prop {
    pub fn new(
        interaction: Option<PropInteraction>,
        frames_per_second: f32,
        last_frame: usize,
    ) -> Self {
        Prop { interaction, frames_per_second, last_frame, uses: 0 }
    }
}

/// Where a prop is along its path and where it is heading.
#[derive(Component, Debug, Default)]
pub struct PropMotion {
    pub frame: f32,
    pub target: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UseOutcome {
    Moving,
    Locked,
    NothingHappens,
    /// An action this rewrite does not perform yet.
    Unsupported(u32),
}

/// Run a prop's trigger action. The first use runs the initial action and
/// later uses the alternate one, when set (retail `SelectAction`).
pub fn use_prop(prop: &mut Prop, motion: &mut PropMotion) -> UseOutcome {
    let Some(interaction) = &prop.interaction else { return UseOutcome::NothingHappens };
    let action = if prop.uses > 0 && interaction.alternate_action != 0 {
        interaction.alternate_action
    } else {
        interaction.initial_action
    };
    let locked = interaction.door.is_some_and(|door| door.locked);
    prop.uses += 1;
    let end = prop.last_frame as f32;
    let toggle = |motion: &mut PropMotion| {
        motion.target = if motion.target > 0.0 { 0.0 } else { end };
    };
    match action {
        action::DOOR_OPEN | action::DOOR_TOGGLE | action::DOOR_TOGGLE_WITH_KEY if locked => {
            prop.uses -= 1;
            UseOutcome::Locked
        }
        action::DOOR_OPEN => {
            motion.target = end;
            UseOutcome::Moving
        }
        action::DOOR_TOGGLE | action::DOOR_TOGGLE_WITH_KEY | action::ANIMATION_TOGGLE => {
            toggle(motion);
            UseOutcome::Moving
        }
        action::ANIMATION_START => {
            motion.target = end;
            UseOutcome::Moving
        }
        action::ANIMATION_STOP => {
            motion.target = motion.frame;
            UseOutcome::Moving
        }
        other => UseOutcome::Unsupported(other),
    }
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

/// How far the party can reach to use something, in metres.
pub const REACH: f32 = 3.0;

/// The first prop hit by a ray, within reach.
pub fn prop_under_ray(
    ray: Ray3d,
    exclude: Entity,
    spatial: &SpatialQuery,
    props: &Query<(), With<Prop>>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    let filter = SpatialQueryFilter::default().with_excluded_entities([exclude]);
    let hit = spatial.cast_ray(ray.origin, ray.direction, REACH, true, &filter)?;
    owning_prop(hit.entity, props, parents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::level::DoorLock;

    fn door(locked: bool) -> Prop {
        Prop::new(
            Some(PropInteraction {
                trigger: "DOOR".into(),
                initial_action: action::DOOR_OPEN,
                alternate_action: action::DOOR_TOGGLE,
                door: Some(DoorLock { locked, key: None }),
            }),
            10.0,
            15,
        )
    }

    #[test]
    fn a_door_opens_then_toggles() {
        let (mut prop, mut motion) = (door(false), PropMotion::default());
        assert_eq!(use_prop(&mut prop, &mut motion), UseOutcome::Moving);
        assert_eq!(motion.target, 15.0);
        use_prop(&mut prop, &mut motion);
        assert_eq!(motion.target, 0.0);
        use_prop(&mut prop, &mut motion);
        assert_eq!(motion.target, 15.0);
    }

    #[test]
    fn a_locked_door_stays_shut() {
        let (mut prop, mut motion) = (door(true), PropMotion::default());
        assert_eq!(use_prop(&mut prop, &mut motion), UseOutcome::Locked);
        assert_eq!(motion.target, 0.0);
    }

    #[test]
    fn poses_interpolate_between_keys() {
        let keys = [Transform::from_xyz(0.0, 0.0, 0.0), Transform::from_xyz(2.0, 0.0, 0.0)];
        assert_eq!(pose(&keys, 0.5).translation.x, 1.0);
        assert_eq!(pose(&keys, 9.0).translation.x, 2.0);
    }
}
