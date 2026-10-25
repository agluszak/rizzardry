//! Compiled level geometry at the start of a `.pvl` file.
//!
//! Layout (after Whizzardry's `ReadMultipleLevelMeshes` and
//! `CompiledMesh::Read`): mesh and root counts, the material table, `-1`,
//! one compiled mesh per octree mesh, `-1`. Later sections (lights, monsters,
//! props, triggers, named points, automap) are not decoded yet.

use super::bytes::{Bytes, Truncated, c_string};

const MATERIAL_RECORD: usize = 0x11a;
const TERMINATOR: i32 = -1;

/// Material record bit: render both faces.
pub const MATERIAL_TWO_SIDED: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRecord {
    pub version: u8,
    /// The four texture slots. The first non-empty slot is used; its index
    /// decides the blend mode when the texture carries alpha.
    pub textures: [String; 4],
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub emissive: [f32; 3],
    pub specular: [f32; 3],
    pub shininess: f32,
    pub opacity: f32,
    pub emission: f32,
    pub animation_mode: u8,
    pub animation_frame: i32,
    pub animation_rate: f32,
    pub surface_flags: u32,
    pub texture_modes: [f32; 4],
}

impl MaterialRecord {
    /// The selected texture slot and file name.
    pub fn texture(&self) -> Option<(usize, &str)> {
        self.textures
            .iter()
            .enumerate()
            .find(|(_, name)| !name.is_empty())
            .map(|(i, n)| (i, n.as_str()))
    }

    pub fn two_sided(&self) -> bool {
        self.surface_flags & MATERIAL_TWO_SIDED != 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledMesh {
    pub version: u8,
    /// Retail draws a weighted mesh through the sorted (translucent) path.
    pub weighted: bool,
    /// Index of the preceding mesh this one continues, if any.
    pub link: Option<usize>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Precomputed static light per vertex.
    pub vertex_light: Vec<[f32; 3]>,
    /// Per-vertex sun exposure; absent before version 1.
    pub sunlight: Option<Vec<f32>>,
    pub uvs: Vec<[f32; 2]>,
    pub polygons: Vec<Polygon>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Polygon {
    pub vertices: [u32; 3],
    pub uvs: [u32; 3],
    pub material: u32,
    pub plane: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct LevelGeometry {
    /// Index 0 is the engine's default material, not the record on disk.
    pub materials: Vec<MaterialRecord>,
    pub meshes: Vec<CompiledMesh>,
    pub root_count: usize,
    /// Bytes consumed, where the lights section starts.
    pub end: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PvlError {
    #[error(transparent)]
    Truncated(#[from] Truncated),
    #[error("{0} at offset {1}")]
    Invalid(&'static str, usize),
}

pub(crate) fn invalid(message: &'static str, bytes: &Bytes) -> PvlError {
    PvlError::Invalid(message, bytes.offset())
}

pub(crate) fn text(bytes: &mut Bytes, len: usize) -> Result<String, PvlError> {
    let offset = bytes.offset();
    c_string(bytes.take(len)?).ok_or(PvlError::Invalid("unterminated texture name", offset))
}

fn material(bytes: &mut Bytes, with_modes: bool) -> Result<MaterialRecord, PvlError> {
    let version = bytes.u8()?;
    let _legacy_name = text(bytes, 0x28)?;
    let textures = [text(bytes, 0x28)?, text(bytes, 0x28)?, text(bytes, 0x28)?, text(bytes, 0x28)?];
    let ambient = bytes.f32s()?;
    let diffuse = bytes.f32s()?;
    let emissive = bytes.f32s()?;
    let specular = bytes.f32s()?;
    let shininess = bytes.f32()?;
    let opacity = bytes.f32()?;
    let emission = bytes.f32()?;
    bytes.take(8)?;
    let animation_mode = bytes.u8()?;
    let animation_frame = bytes.i32()?;
    let animation_rate = bytes.f32()?;
    let surface_flags = bytes.u32()?;
    let texture_modes = if with_modes { bytes.f32s()? } else { [0.0; 4] };
    Ok(MaterialRecord {
        version,
        textures,
        ambient,
        diffuse,
        emissive,
        specular,
        shininess,
        opacity,
        emission,
        animation_mode,
        animation_frame,
        animation_rate,
        surface_flags,
        texture_modes,
    })
}

/// The material table shared by all meshes. The first record's version
/// decides whether every record carries texture modes.
pub fn read_materials(bytes: &mut Bytes) -> Result<Vec<MaterialRecord>, PvlError> {
    let count = bytes.i16()?;
    if count < 1 {
        return Err(invalid("no materials", bytes));
    }
    let count = count as usize;
    if count * MATERIAL_RECORD > bytes.remaining() {
        return Err(invalid("material count exceeds file", bytes));
    }
    let first = material(bytes, false)?;
    let with_modes = first.version >= 4;
    let mut first = first;
    if with_modes {
        first.texture_modes = bytes.f32s()?;
    }
    let mut materials = vec![first];
    for _ in 1..count {
        materials.push(material(bytes, with_modes)?);
    }
    Ok(materials)
}

fn index(value: i32, len: usize, what: &'static str, bytes: &Bytes) -> Result<u32, PvlError> {
    if value < 0 || value as usize >= len {
        return Err(invalid(what, bytes));
    }
    Ok(value as u32)
}

fn compiled_mesh(
    bytes: &mut Bytes,
    mesh_index: usize,
    material_count: usize,
) -> Result<CompiledMesh, PvlError> {
    let packed = bytes.u32()?;
    let [vertex_count, uv_count, polygon_count, link, _next_link, material] = bytes.i32s()?;
    let weighted = packed & 0xff != 0;
    let version = (packed >> 8) as u8;
    if vertex_count < 0 || uv_count < 0 || polygon_count < 0 {
        return Err(invalid("negative mesh counts", bytes));
    }
    if material < -1 || material >= material_count as i32 {
        return Err(invalid("mesh material out of range", bytes));
    }
    if link >= mesh_index as i32 || link < -1 {
        return Err(invalid("linked mesh does not precede it", bytes));
    }
    let (vertices, uv_count, polygon_count) =
        (vertex_count as usize, uv_count as usize, polygon_count as usize);

    let positions = bytes.records(vertices, 12, |b| b.f32s())?;
    let uvs = bytes.records(uv_count, 8, |b| b.f32s())?;
    if material < 0 {
        // Per-vertex materials only select render state, which the per-polygon
        // texture table below also determines.
        for value in bytes.records(vertices, 4, |b| b.i32())? {
            index(value, material_count, "vertex material out of range", bytes)?;
        }
    }
    let polygon_uvs = bytes.records(polygon_count, 12, |b| b.i32s::<3>())?;
    let polygon_vertices = bytes.records(polygon_count, 12, |b| b.i32s::<3>())?;
    let polygon_materials = bytes.records(polygon_count, 4, |b| b.i32())?;
    let normals = bytes.records(vertices, 12, |b| b.f32s())?;
    let vertex_light = bytes.records(vertices, 12, |b| b.f32s())?;
    let planes = bytes.records(polygon_count, 16, |b| b.f32s())?;
    let sunlight = if version != 0 { Some(bytes.records(vertices, 4, |b| b.f32())?) } else { None };
    if bytes.i32()? != TERMINATOR {
        return Err(invalid("mesh record has the wrong length", bytes));
    }

    let mut polygons = Vec::with_capacity(polygon_count);
    for i in 0..polygon_count {
        let mut corner_vertices = [0; 3];
        let mut corner_uvs = [0; 3];
        for c in 0..3 {
            corner_vertices[c] =
                index(polygon_vertices[i][c], vertices, "vertex out of range", bytes)?;
            corner_uvs[c] = index(polygon_uvs[i][c], uv_count, "uv out of range", bytes)?;
        }
        polygons.push(Polygon {
            vertices: corner_vertices,
            uvs: corner_uvs,
            material: index(
                polygon_materials[i],
                material_count,
                "polygon material out of range",
                bytes,
            )?,
            plane: planes[i],
        });
    }
    Ok(CompiledMesh {
        version,
        weighted,
        link: (link >= 0).then_some(link as usize),
        positions,
        normals,
        vertex_light,
        sunlight,
        uvs,
        polygons,
    })
}

pub fn read_level_geometry(data: &[u8]) -> Result<LevelGeometry, PvlError> {
    let mut bytes = Bytes::new(data);
    let mesh_count = bytes.u32()? as usize;
    let root_count = bytes.u32()? as usize;
    if root_count > mesh_count || mesh_count > bytes.remaining() {
        return Err(invalid("invalid mesh or root count", &bytes));
    }
    let materials = read_materials(&mut bytes)?;
    if bytes.i32()? != TERMINATOR {
        return Err(invalid("material table has the wrong length", &bytes));
    }
    let mut meshes = Vec::with_capacity(mesh_count);
    for mesh_index in 0..mesh_count {
        meshes.push(compiled_mesh(&mut bytes, mesh_index, materials.len())?);
    }
    if bytes.i32()? != TERMINATOR {
        return Err(invalid("mesh table has the wrong length", &bytes));
    }
    Ok(LevelGeometry { materials, meshes, root_count, end: bytes.offset() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against a real installation when `RIZZARDRY_RETAIL` names one.
    #[test]
    #[ignore = "requires a retail installation"]
    fn decodes_retail_level_geometry() {
        let root = std::env::var("RIZZARDRY_RETAIL")
            .expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let levels: Vec<String> =
            files.names().filter(|name| name.ends_with(".pvl")).map(str::to_owned).collect();
        assert!(!levels.is_empty());
        for level in levels {
            let data = files.read(&level).unwrap().unwrap();
            let geometry = read_level_geometry(&data).unwrap_or_else(|e| panic!("{level}: {e}"));
            assert!(!geometry.meshes.is_empty(), "{level}");
        }
    }
}
