//! Prop records (Whizzardry `Prop::LoadProp`, `AnimObjReadFromFile`,
//! `ReadAnimatedMesh`): placed scenery, doors, levers, lifts and billboards.

use super::bytes::Bytes;
use super::keyframes::{PathKey, read_path};
use super::mesh_record::{MeshRecord, MeshVertices, RECORD_SCALE, read_mesh_record};
use super::pvl::{PvlError, invalid, text};
use super::trigger::{Trigger, read_trigger};

/// Prop flag: the party and monsters collide with the prop.
pub const PROP_COLLIDABLE: u32 = 1;

/// A mesh with optional frame alternatives: either one record whose
/// vertices animate, or one record per frame.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedMesh {
    pub first_frame: usize,
    pub frames: Vec<MeshRecord>,
}

/// One part of a prop. Positioned parts follow their own path; key 0 is the
/// rest pose and replaces the mesh record's placement.
#[derive(Clone, Debug, PartialEq)]
pub struct PropPart {
    pub mesh: AnimatedMesh,
    pub path: Option<Vec<PathKey>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub name: String,
    /// `W8_PROP_*` flags.
    pub flags: u32,
    /// Drawn turned toward the viewer (billboards: the Sun, the Moon).
    pub face_camera: bool,
    /// Retail units; the origin of props without positioned parts.
    pub location: [f32; 3],
    pub frames_per_second: f32,
    /// Three detail channels; retail draws the highest non-empty one up
    /// close.
    pub channels: [Vec<PropPart>; 3],
    /// A whole-prop path, for props without positioned parts.
    pub path: Option<Vec<PathKey>>,
    pub trigger: Option<Trigger>,
}

impl Prop {
    /// The most detailed non-empty channel.
    pub fn detailed_parts(&self) -> &[PropPart] {
        self.channels.iter().rev().find(|c| !c.is_empty()).map_or(&[], Vec::as_slice)
    }
}

pub fn read_animated_mesh(bytes: &mut Bytes) -> Result<AnimatedMesh, PvlError> {
    let count = bytes.u8()? as usize;
    let first_frame = bytes.u8()? as usize;
    if count == 0 || first_frame >= count {
        return Err(invalid("invalid animated mesh frame count", bytes));
    }
    let first = read_mesh_record(bytes)?;
    let vertex_frames = matches!(first.vertices, MeshVertices::Frames(_));
    let mut frames = vec![first];
    if !vertex_frames {
        for _ in 1..count {
            if bytes.u8()? as usize >= count {
                return Err(invalid("animated mesh frame index out of range", bytes));
            }
            frames.push(read_mesh_record(bytes)?);
        }
    }
    Ok(AnimatedMesh { first_frame, frames })
}

/// An animation object (Whizzardry `AnimObjReadFromFile`): the shared part
/// of props and monster cycles.
pub struct Animation {
    pub frames_per_second: f32,
    pub path: Option<Vec<PathKey>>,
    /// Three detail channels of parts.
    pub channels: [Vec<PropPart>; 3],
}

/// Skip an animation's embedded lights. Props may not carry any; monster
/// cycles can.
fn skip_lights(bytes: &mut Bytes, allowed: bool) -> Result<(), PvlError> {
    let count = bytes.u8()?;
    if count != 0 && !allowed {
        return Err(invalid("prop animation carries lights", bytes));
    }
    for _ in 0..count {
        let version = bytes.u8()?;
        bytes.take(12 + 12 + 4 + 4)?; // position, colour, intensity, range
        // Version 2 lights always carry the kind 1 (animated) block.
        let kind = match version {
            2 => 1,
            0..2 => 0,
            _ => bytes.u8()?,
        };
        match kind {
            1 => {
                bytes.take(60)?;
            }
            2 => {
                bytes.take(129)?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn read_animation(bytes: &mut Bytes, lights_allowed: bool) -> Result<Animation, PvlError> {
    let version = bytes.u8()?;
    let groups = bytes.u8()? as usize;
    // Playing, frame method, behaviour, cycle.
    bytes.take(4)?;
    let positioned = bytes.u8()? != 0;
    let frames_per_second = if version >= 3 { bytes.f32()? } else { 15.0 };
    if version >= 5 {
        bytes.u8()?; // start frame
    }
    if version >= 11 {
        bytes.u8()?; // end frame
    }
    if version >= 6 {
        bytes.take(1 + 4)?; // random play, play chance
    }
    bytes.take(50)?;
    if groups > 3 {
        return Err(invalid("more than three animation groups", bytes));
    }
    bytes.take(groups)?; // channel bytes
    if version > 6 {
        let count = bytes.u8()? as usize;
        bytes.take(count * 24)?; // frame bounds
    }
    if version > 7 {
        skip_lights(bytes, lights_allowed)?;
    }
    let mut path = None;
    if !positioned && version > 8 && bytes.u8()? != 0 {
        path = Some(read_path(bytes)?);
    }
    if version > 9 {
        bytes.u8()?;
    }
    let mut channels: [Vec<PropPart>; 3] = Default::default();
    for _ in 0..groups {
        let count = if positioned { bytes.i8()?.max(0) } else { 1 };
        for _ in 0..count {
            let channel = bytes.i8()?;
            if !(0..3).contains(&channel) {
                return Err(invalid("animation channel out of range", bytes));
            }
            let mesh = read_animated_mesh(bytes)?;
            let channel = &mut channels[channel as usize];
            if positioned {
                channel.push(PropPart { mesh, path: Some(read_path(bytes)?) });
            } else {
                *channel = vec![PropPart { mesh, path: None }];
            }
        }
    }
    Ok(Animation { frames_per_second, path, channels })
}

pub fn read_prop(bytes: &mut Bytes) -> Result<Prop, PvlError> {
    let version = bytes.i8()?;
    let mut prop = Prop {
        name: String::new(),
        flags: 0,
        face_camera: false,
        location: [0.0; 3],
        frames_per_second: 15.0,
        channels: Default::default(),
        path: None,
        trigger: None,
    };
    if version < 4 {
        bytes.take(3)?; // playing, frame method, behaviour
        if version >= 2 {
            prop.frames_per_second = bytes.f32()?;
        }
        prop.channels[0] = vec![PropPart { mesh: read_animated_mesh(bytes)?, path: None }];
    } else {
        bytes.u8()?; // frame count
        if version > 4 {
            prop.face_camera = bytes.u8()? != 0;
            prop.location = bytes.f32s::<3>()?.map(|v| v * RECORD_SCALE);
        }
        if version > 5 {
            prop.flags = bytes.u32()?;
        }
        if version > 6 {
            prop.name = text(bytes, 64)?;
        }
        if version > 7 {
            let slots = bytes.i8()?.max(0) as usize;
            bytes.take(slots * 4)?;
        }
        let animation = read_animation(bytes, false)?;
        prop.frames_per_second = animation.frames_per_second;
        prop.path = animation.path;
        prop.channels = animation.channels;
    }
    if version > 2 && bytes.u8()? != 0 {
        prop.trigger = Some(read_trigger(bytes)?);
    }
    if version >= 9 && bytes.u8()? != 0 {
        bytes.take(2)?; // footstep sound
    }
    Ok(prop)
}
