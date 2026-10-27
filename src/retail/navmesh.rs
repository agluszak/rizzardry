//! Navmesh inputs (see [`super::source`]): a size class and a level's
//! `.oct` and `.pvl` bytes, decoded into the level's collision.

use super::oct::read_collision;
use super::prop::PROP_COLLIDABLE;
use super::prop_loader::{part_transforms, record_collision};
use super::pvl::read_level_geometry;
use super::sections::read_sections;
use super::source::navmesh_input;
use crate::bytes::{Bytes, DecodeError};
use crate::content::level::CollisionMesh;
use crate::content::navmesh::LevelCollision;

/// A level's collision for one size class, from its navmesh input.
pub fn decode_input(data: &[u8]) -> Result<LevelCollision, String> {
    let (class, octree, level) = navmesh_input(data).ok_or("not a navmesh input")?;
    let collision = level_collision(octree, level).map_err(|error| error.to_string())?;
    Ok(LevelCollision { collision, class: class.into() })
}

/// The level's solid collision surfaces, plus its collidable props in their
/// rest pose, as the level importer converts them.
pub fn level_collision(octree: &[u8], level: &[u8]) -> Result<CollisionMesh, DecodeError> {
    let mut collision = read_collision(octree)?.solid_mesh();
    let geometry = read_level_geometry(level)?;
    let sections = read_sections(&mut Bytes::new(&level[geometry.end..]))?;
    for prop in sections.props.iter().filter(|p| p.flags & PROP_COLLIDABLE != 0) {
        for (part, (transform, _)) in prop.detailed_parts().iter().zip(part_transforms(prop)) {
            let record = &part.mesh.frames[part.mesh.first_frame];
            collision.append(&record_collision(record), &transform, false);
        }
    }
    Ok(collision)
}
