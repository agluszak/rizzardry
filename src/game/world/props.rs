//! Usable level props: doors, lids, levers and lifts that move along their
//! authored paths.

use bevy::prelude::*;

use super::level::PropPart;
use crate::content::level::Animation;

/// A placed prop. Its parts are children carrying [`PropPart`].
#[derive(Component, Debug)]
pub struct Prop {
    /// Its place among the level's props (`LevelDefinition::props`), which
    /// identifies it within the level.
    pub definition: usize,
    /// The middle of its parts at rest, where its sounds play.
    pub centre: Vec3,
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

impl PropMotion {
    pub fn activate(mut motion: Mut<Self>, prop: &Prop, animation: Animation) {
        let playing = motion.frame != motion.target;
        match animation {
            Animation::Toggle if playing => motion.target = motion.frame,
            Animation::Toggle | Animation::Start if !playing => {
                if prop.wraps {
                    motion.frame = 0.0;
                    motion.target = prop.last_frame as f32;
                } else {
                    motion.target = if motion.frame >= prop.last_frame as f32 {
                        0.0
                    } else {
                        prop.last_frame as f32
                    };
                }
            }
            Animation::Stop if playing => motion.target = motion.frame,
            _ => {}
        }
    }
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
