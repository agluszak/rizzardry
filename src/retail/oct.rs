//! Level collision from a `.oct` file (Whizzardry `LevelSpatialIndex` and
//! `ReadProcessedCollision`).
//!
//! The file is a render octree followed by the level's collision geometry:
//! its own vertices and one-sided triangles with surface flags. Retail
//! collides only with these, never with render polygons, so they include
//! invisible walls and walkable riverbeds under translucent water.

use std::collections::HashMap;

use super::bytes::Bytes;
use super::pvl::{PvlError, invalid};
use super::space::retail_position;
use crate::content::level::CollisionMesh;

const VERSION: usize = 0x22;
const HEADER: usize = 0xF5;
const TERMINATOR: u32 = 0xFFFF_FFFF;
const PACKED_BITS_MAGIC: u32 = 0xDEAD_D00D;

/// Surface flag: a floor (designer-marked or flatter than normal.y 0.996).
pub const WALKABLE: u32 = 0x4;
/// Surface flag: disabled by a switch (unused by retail levels).
pub const CONDITIONAL_DISABLED: u32 = 0x10;
/// Surface flag: designer-marked walkable; retail's monster path grid covers
/// exactly these.
pub const PATHFINDING: u32 = 0x40;
/// Surface flag: a trigger plane, fired when crossed; never solid.
pub const CROSSING: u32 = 0x80;
/// Surface flag: an environment-swap plane (unused by retail levels).
pub const ENVIRONMENT: u32 = 0x1000;

/// Footstep material of walkable surfaces under shallow water.
pub const SHALLOW_WATER: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub flags: u32,
    /// The `.pvl` trigger (in section order) a crossing surface fires.
    pub trigger_index: i32,
    /// Retail winding; the stored plane's normal faces the open side.
    pub vertices: [u32; 3],
    pub footstep_material: u8,
}

impl Surface {
    pub fn solid(&self) -> bool {
        self.flags & (CROSSING | ENVIRONMENT | CONDITIONAL_DISABLED) == 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Collision {
    /// Retail units.
    pub vertices: Vec<[f32; 3]>,
    pub surfaces: Vec<Surface>,
}

impl Collision {
    /// The solid surfaces in Bevy space, compacted to the vertices they use.
    /// Designer-marked walkable surfaces (the ones retail's monster path
    /// grid covers) are walkable.
    pub fn solid_mesh(&self) -> CollisionMesh {
        let mut mesh = CollisionMesh::default();
        let mut remap: HashMap<u32, u32> = HashMap::new();
        for surface in self.surfaces.iter().filter(|s| s.solid()) {
            let [a, b, c] = surface.vertices.map(|v| {
                *remap.entry(v).or_insert_with(|| {
                    mesh.vertices.push(retail_position(self.vertices[v as usize]));
                    mesh.vertices.len() as u32 - 1
                })
            });
            // Mirroring Z reverses winding; swap to keep the open side out.
            mesh.triangles.push([a, c, b]);
            mesh.walkable.push(surface.flags & PATHFINDING != 0);
        }
        mesh
    }
}

fn u32_at(header: &[u8], at: usize) -> usize {
    u32::from_le_bytes(header[at..at + 4].try_into().unwrap()) as usize
}

fn u16_at(header: &[u8], at: usize) -> usize {
    u16::from_le_bytes(header[at..at + 2].try_into().unwrap()) as usize
}

fn terminator(bytes: &mut Bytes) -> Result<(), PvlError> {
    if bytes.u32()? != TERMINATOR {
        return Err(invalid("octree section has the wrong length", bytes));
    }
    Ok(())
}

fn skip_packed_bits(bytes: &mut Bytes) -> Result<(), PvlError> {
    let magic = bytes.u32()?;
    let _bits = bytes.u32()?;
    let size = bytes.u32()? as usize;
    bytes.take(size)?;
    if magic != PACKED_BITS_MAGIC || bytes.u32()? != PACKED_BITS_MAGIC {
        return Err(invalid("bad packed bits", bytes));
    }
    Ok(())
}

pub fn read_collision(data: &[u8]) -> Result<Collision, PvlError> {
    let mut bytes = Bytes::new(data);
    let header = bytes.take(HEADER)?;
    if u16_at(header, 0) != VERSION {
        return Err(invalid("unsupported octree version", &bytes));
    }
    terminator(&mut bytes)?;
    let grid = u32_at(header, 0x56) * u32_at(header, 0x5A) * u32_at(header, 0x5E);
    let submeshes = u32_at(header, 0x66);
    let branches = u32_at(header, 0x6A);
    let leaves = u32_at(header, 0x6E);
    let polygons = u32_at(header, 0x72);
    let path_nodes = u32_at(header, 0x7E);
    let leaf_polygon_stream = u32_at(header, 0x82);
    let surface_stream = u32_at(header, 0x86);
    let triggers = u32_at(header, 0x8A);
    let region_list = u32_at(header, 0x92);
    let regions = u16_at(header, 0x96);
    let mesh_total = u32_at(header, 0x9E);
    let prop_sun_bits = header[0xB8] != 0;
    let props = u32_at(header, 0xBD);
    let particles = u32_at(header, 0xC1);
    let particle_links = u16_at(header, 0xC5);
    let prop_links = u16_at(header, 0xC7);

    // The render octree: branches, leaves, polygon and region streams.
    bytes.take(branches * 36 + leaves * 40 + leaf_polygon_stream * 4)?;
    if grid < 250_000 {
        bytes.take(grid * 4)?;
    }
    bytes.take(polygons * 4 + region_list * 2 + surface_stream * 4 + triggers * 2)?;
    if regions > 1 {
        bytes.take(regions * 0xE8)?;
    }
    terminator(&mut bytes)?;
    if submeshes != 0 {
        bytes.take((submeshes + 1) * 16)?;
        if mesh_total != 0 {
            skip_packed_bits(&mut bytes)?;
        }
        if particles != 0 {
            bytes.take((mesh_total + 1) * 2 + particle_links * 2)?;
        }
        if props != 0 {
            bytes.take((mesh_total + 1) * 2 + prop_links * 2)?;
        }
    }
    terminator(&mut bytes)?;
    if path_nodes != 0 {
        // The monster path grid and its door-dependent sets.
        bytes.take(path_nodes * 8)?;
        let sets = bytes.u32()? as usize;
        let frames = bytes.u32()? as usize;
        let nodes = bytes.u32()? as usize;
        bytes.u32()?;
        if frames >= 2 && nodes >= 2 {
            bytes.take(sets * 0x44 + frames * 6 + nodes * 8)?;
        }
    }
    if props != 0 && prop_sun_bits {
        skip_packed_bits(&mut bytes)?;
    }
    terminator(&mut bytes)?;
    if surface_stream == 0 {
        return Err(invalid("level has no collision geometry", &bytes));
    }

    let version = bytes.u32()?;
    bytes.take(24)?; // bounds
    let vertex_count = bytes.i32()?.max(0) as usize;
    let surface_count = bytes.i32()?.max(0) as usize;
    bytes.take(12)?; // walkable count, non-walkable count, crossing count
    let interfaces = bytes.i32()?.max(0) as usize;
    let states = bytes.i32()?.max(0) as usize;
    bytes.i32()?; // trigger planes
    let conditional = bytes.i32()?.max(0) as usize;
    let environments = bytes.i32()?.max(0) as usize;
    bytes.take(36)?;
    if version != 1 {
        return Err(invalid("unsupported collision version", &bytes));
    }
    let vertices = bytes.records(vertex_count, 12, |b| b.f32s::<3>())?;
    let surfaces = bytes.records(surface_count, 0x4C, |b| {
        let flags = b.u32()?;
        b.u32()?; // index
        let trigger_index = b.i32()?;
        b.take(12)?; // edge links
        let vertices = b.i32s::<3>()?.map(|v| v as u32);
        b.take(16 + 4 + 4)?; // plane, scratch distance, scratch pointer
        b.u8()?; // footstep surface (acoustics)
        let footstep_material = b.u8()?;
        b.take(2 + 4 + 4 + 4)?; // positional, contact margin, chance, slope
        Ok(Surface { flags, trigger_index, vertices, footstep_material })
    })?;
    if surfaces.iter().any(|s| s.vertices.iter().any(|&v| v as usize >= vertices.len())) {
        return Err(invalid("collision vertex out of range", &bytes));
    }
    bytes.take(interfaces * 12 + states * 12 + conditional * 4 + environments * 0x44)?;
    terminator(&mut bytes)?;
    Ok(Collision { vertices, surfaces })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against a real installation when `RIZZARDRY_RETAIL` names one.
    #[test]
    #[ignore = "requires a retail installation"]
    fn every_retail_level_has_collision() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let levels: Vec<String> =
            files.names().filter(|n| n.ends_with(".oct")).map(str::to_owned).collect();
        assert_eq!(levels.len(), 34);
        for level in levels {
            let data = files.read(&level).unwrap().unwrap();
            let collision = read_collision(&data).unwrap_or_else(|e| panic!("{level}: {e}"));
            assert!(collision.surfaces.iter().any(|s| s.flags & PATHFINDING != 0), "{level}");
        }
        let data = files.read("levels/monastery/monastery1.oct").unwrap().unwrap();
        let collision = read_collision(&data).unwrap();
        assert_eq!(collision.vertices.len(), 12_667);
        assert_eq!(collision.surfaces.len(), 24_706);
    }
}
