//! Single mesh records (Whizzardry `ReadSingleLevelMeshRecord`): the meshes of
//! props, animated props, items and uncompiled levels.

use super::pvl::{MaterialRecord, read_materials};
use crate::bytes::{Bytes, DecodeError};
use crate::content::UNITS_PER_METRE;

// Records store lengths in metres; `UNITS_PER_METRE` scales them to retail
// units.

/// Vertex positions, in retail units.
#[derive(Clone, Debug, PartialEq)]
pub enum MeshVertices {
    Static(Vec<[f32; 3]>),
    /// Vertex animation: one position per vertex per frame.
    Frames(Vec<Vec<[f32; 3]>>),
}

impl MeshVertices {
    pub fn first(&self) -> &[[f32; 3]] {
        match self {
            MeshVertices::Static(v) => v,
            MeshVertices::Frames(frames) => &frames[0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshFace {
    pub vertices: [u32; 3],
    pub uvs: [[f32; 2]; 3],
    pub material: u32,
    pub flags: u8,
}

/// Face flag: draw both sides.
pub const FACE_TWO_SIDED: u8 = 4;

/// Where a record places its mesh: retail units, and a rotation of `angle`
/// radians about `axis`. Records before version 2 have none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshPlacement {
    pub location: [f32; 3],
    pub angle: f32,
    pub axis: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshRecord {
    pub placement: Option<MeshPlacement>,
    pub vertices: MeshVertices,
    pub faces: Vec<MeshFace>,
    /// Index 0 is replaced by the engine's default material.
    pub materials: Vec<MaterialRecord>,
}

const FLAG_VERTEX_FRAMES: u8 = 1;
const FLAG_SHORT_FRAMES: u8 = 2;
const FLAG_COMPACT_FACES: u8 = 4;

pub fn read_mesh_record(bytes: &mut Bytes) -> Result<MeshRecord, DecodeError> {
    let version = bytes.i32()?;
    let vertex_count = bytes.i32()?;
    let face_count = bytes.i32()?;
    if !(1..=100_000).contains(&vertex_count) || !(1..=200_000).contains(&face_count) {
        return Err(bytes.invalid("invalid mesh record counts"));
    }
    let (vertex_count, face_count) = (vertex_count as usize, face_count as usize);
    let flags = if version > 2 { bytes.u8()? } else { 0 };
    let placement = if version > 1 {
        let [x, y, z, angle, ax, ay, az, _sx, _sy, _sz] = bytes.f32s::<10>()?;
        // Retail reads the authored scale but does not apply it.
        Some(MeshPlacement {
            location: [x, y, z].map(|v| v * UNITS_PER_METRE),
            angle,
            axis: [ax, ay, az],
        })
    } else {
        None
    };
    // Marker keys to vertex indices, e.g. a missile launch point.
    if version > 3 {
        let count = bytes.i8()?;
        if count < 0 {
            return Err(bytes.invalid("negative vertex mapping count"));
        }
        for _ in 0..count {
            bytes.i16()?; // key
            let vertex = bytes.i16()?;
            if vertex < 0 || vertex as usize >= vertex_count {
                return Err(bytes.invalid("vertex mapping out of range"));
            }
        }
    }
    let vertices = if flags & FLAG_VERTEX_FRAMES == 0 {
        let raw = bytes.records(vertex_count, 12, |b| b.f32s::<3>())?;
        MeshVertices::Static(raw.into_iter().map(|v| v.map(|c| c * UNITS_PER_METRE)).collect())
    } else {
        let compression = bytes.u8()?;
        let frames = bytes.i16()?;
        let compression_scale = if compression == 2 { bytes.f32()? } else { 1000.0 };
        if frames <= 0 || !compression_scale.is_finite() || compression_scale <= 0.0 {
            return Err(bytes.invalid("invalid vertex frames"));
        }
        let mut all = Vec::with_capacity(frames as usize);
        for _ in 0..frames {
            if flags & FLAG_SHORT_FRAMES != 0 {
                let scale = UNITS_PER_METRE / compression_scale;
                let raw = bytes.records(vertex_count, 6, |b| {
                    Ok([b.i16()?, b.i16()?, b.i16()?].map(|c| f32::from(c) * scale))
                })?;
                all.push(raw);
            } else {
                // Historical float frames; retail cannot render them either.
                let raw = bytes.records(vertex_count, 12, |b| b.f32s::<3>())?;
                all.push(raw.into_iter().map(|v| v.map(|c| c * UNITS_PER_METRE)).collect());
            }
        }
        if all.len() == 1 {
            MeshVertices::Static(all.pop().unwrap())
        } else {
            MeshVertices::Frames(all)
        }
    };
    let compact = flags & FLAG_COMPACT_FACES != 0;
    let faces = bytes.records(face_count, if compact { 0x21 } else { 0x29 }, |b| {
        let vertices = if compact {
            [b.u16()?, b.u16()?, b.u16()?].map(u32::from)
        } else {
            b.i32s::<3>()?.map(|v| v as u32)
        };
        let uvs = [b.f32s::<2>()?, b.f32s::<2>()?, b.f32s::<2>()?];
        let material = if compact { u32::from(b.u16()?) } else { b.i32()? as u32 };
        Ok(MeshFace { vertices, uvs, material, flags: b.u8()? })
    })?;
    let materials = read_materials(bytes)?;
    for face in &faces {
        if face.material as usize >= materials.len()
            || face.vertices.iter().any(|&v| v as usize >= vertex_count)
        {
            return Err(bytes.invalid("mesh face index out of range"));
        }
    }
    Ok(MeshRecord { placement, vertices, faces, materials })
}
