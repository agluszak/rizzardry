//! Usable level props: doors, lids, levers and lifts that move along their
//! authored paths.

use bevy::prelude::*;

use avian3d::prelude::*;

use super::level::LevelOwned;
use super::navigation::links::DoorLink;
use crate::content::level::{Animation, CollisionMesh, PropModel};

/// A placed prop. Its parts are children carrying [`PropPart`]; what it is
/// is read at spawn; playback reads [`PropAnimation`] and [`PropMotion`].
#[derive(Component, Debug)]
pub struct Prop {
    /// Its place among the level's props (`LevelDefinition::props`), which
    /// identifies it within the level.
    pub definition: usize,
}

/// Playback settings belong to the moving prop, not its level's asset.
#[derive(Component, Debug)]
pub struct PropAnimation {
    pub last_frame: f32,
    pub wraps: bool,
    pub slots: Vec<u16>,
    frames_per_second: f32,
}

#[derive(Component, Debug)]
pub(super) struct MotionPath(Vec<Transform>);

/// One rigid part of a placed prop.
#[derive(Component, Debug)]
pub struct PropPart {
    /// Its place among its prop's parts (`PropModel::parts`).
    pub part: usize,
}

impl Prop {
    /// Its authored model, from its level's definition.
    pub fn model<'a>(
        &self,
        owner: &LevelOwned,
        definitions: &'a super::level::OwnedDefinitions,
    ) -> Option<&'a PropModel> {
        definitions.of(owner)?.props.get(self.definition)
    }
}

/// A solid collider of a collision mesh, cleaned up with `extra` flags.
pub fn trimesh(
    mesh: &CollisionMesh,
    extra: TrimeshFlags,
) -> Result<Collider, impl std::fmt::Debug> {
    Collider::try_trimesh_with_config(
        mesh.vertices.clone(),
        mesh.triangles.clone(),
        TrimeshFlags::MERGE_DUPLICATE_VERTICES | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES | extra,
    )
}

/// Make `entity` the level's prop number `index`, at rest at its first frame, with its
/// moving parts and their colliders; a door also gets its doorway, from its
/// closed shape (each part at its first pose), for walkers' links while it
/// is open.
pub fn build_prop(
    entity: &mut EntityCommands,
    index: usize,
    prop: &PropModel,
    door: bool,
    colliders: Vec<Option<Collider>>,
) {
    let frame = 0.0;
    let doorway = door
        .then(|| {
            DoorLink::around(prop.parts.iter().flat_map(|part| {
                let closed = part.keys.first().copied().unwrap_or(part.transform);
                part.collision.vertices.iter().map(move |&v| closed.transform_point(v))
            }))
        })
        .flatten();
    entity.insert((
        Name::new(prop.name.clone()),
        Prop { definition: index },
        PropMotion::default(),
        PropAnimation {
            last_frame: prop.last_frame() as f32,
            wraps: prop.wraps,
            slots: prop.slots.clone(),
            frames_per_second: prop.frames_per_second,
        },
        Transform::default(),
    ));
    if let Some(doorway) = doorway {
        entity.insert(doorway);
    }
    entity.with_children(|parts| {
        for ((part_index, part), collider) in prop.parts.iter().enumerate().zip(colliders) {
            let at = if part.keys.is_empty() { part.transform } else { pose(&part.keys, frame) };
            let mut part_entity = parts.spawn((PropPart { part: part_index }, at));
            if !part.keys.is_empty() {
                part_entity.insert(MotionPath(part.keys.clone()));
            }
            let Some(collider) = collider else { continue };
            // Parts with a path can move (doors, lifts).
            let body = if part.keys.is_empty() { RigidBody::Static } else { RigidBody::Kinematic };
            part_entity.insert((body, collider));
            // Props the party walks through can still be used; they are not
            // scenery that blocks sight or items.
            if prop.collidable {
                part_entity.insert(super::obstacles::scenery());
            } else {
                part_entity.insert(Sensor);
            }
        }
    });
}

/// Where a prop is along its path and where it is heading.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PropMotion {
    pub frame: f32,
    pub target: f32,
}

impl PropMotion {
    pub fn activate(mut motion: Mut<Self>, prop: &PropAnimation, animation: Animation) {
        let playing = motion.frame != motion.target;
        let last = prop.last_frame;
        match animation {
            Animation::Toggle if playing => motion.target = motion.frame,
            Animation::Toggle | Animation::Start if !playing => {
                if prop.wraps {
                    motion.frame = 0.0;
                    motion.target = last;
                } else {
                    motion.target = if motion.frame >= last { 0.0 } else { last };
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
pub(super) fn animate_props(
    time: Res<Time>,
    mut props: Query<(&PropAnimation, &mut PropMotion, &Children)>,
    mut parts: Query<(&MotionPath, &mut Transform)>,
) {
    for (animation, mut motion, children) in &mut props {
        if motion.frame == motion.target {
            continue;
        }
        let step = animation.frames_per_second * time.delta_secs();
        motion.frame = if motion.target > motion.frame {
            (motion.frame + step).min(motion.target)
        } else {
            (motion.frame - step).max(motion.target)
        };
        for child in children {
            if let Ok((path, mut transform)) = parts.get_mut(*child) {
                *transform = pose(&path.0, motion.frame);
            }
        }
    }
}
