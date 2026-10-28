//! Navmesh inputs (see [`super::source`]): a level's collision, decoded
//! from its `.oct` and `.pvl` files.

use super::oct::read_collision;
use super::prop::PROP_COLLIDABLE;
use super::prop_loader::{part_transforms, record_collision};
use super::pvl::compiled_level_end;
use super::sections::read_sections;
use crate::bytes::{Bytes, DecodeError};
use crate::content::level::CollisionMesh;

/// The level's solid collision surfaces, plus its collidable props and
/// bitmaps in their rest pose, as the level gives them colliders.
pub fn level_collision(octree: &[u8], level: &[u8]) -> Result<CollisionMesh, DecodeError> {
    let mut collision = read_collision(octree)?.solid_mesh();
    let sections = read_sections(&mut Bytes::new(&level[compiled_level_end(level)?..]))?;
    let props = sections.props.iter().chain(&sections.bitmaps);
    for prop in props.filter(|p| p.flags & PROP_COLLIDABLE != 0) {
        for (part, (transform, _)) in prop.detailed_parts().iter().zip(part_transforms(prop)) {
            let record = &part.mesh.frames[part.mesh.first_frame];
            collision.append(&record_collision(record), &transform, false);
        }
    }
    Ok(collision)
}
