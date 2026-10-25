//! Keyframe paths (Whizzardry `LoadPathAI`): camera moves, animated prop
//! parts, animated lights.

use super::bytes::Bytes;
use super::mesh_record::RECORD_SCALE;
use super::pvl::{PvlError, invalid};

/// One key: a position in retail units and a rotation of `angle` radians
/// about `axis`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathKey {
    pub position: [f32; 3],
    pub angle: f32,
    pub axis: [f32; 3],
    pub scale: Option<[f32; 3]>,
}

pub fn read_path(bytes: &mut Bytes) -> Result<Vec<PathKey>, PvlError> {
    if bytes.u8()? != 0 {
        return Err(invalid("unknown path kind", bytes));
    }
    let version = bytes.u8()?;
    let _position = bytes.f32()?;
    let _unknown = bytes.u32()?;
    let count = bytes.i32()?;
    if count < 0 {
        return Err(invalid("negative path key count", bytes));
    }
    let size = if version == 2 { 40 } else { 28 };
    Ok(bytes.records(count as usize, size, |b| {
        Ok(PathKey {
            position: b.f32s::<3>()?.map(|v| v * RECORD_SCALE),
            angle: b.f32()?,
            axis: b.f32s()?,
            scale: if version == 2 { Some(b.f32s()?) } else { None },
        })
    })?)
}
