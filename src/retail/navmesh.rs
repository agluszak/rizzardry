//! Navmesh inputs (see [`super::source`]): a size class and a level's
//! `.oct` and `.pvl` bytes, decoded into the level's collision.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::reflect::TypePath;

use super::oct::read_collision;
use super::prop::PROP_COLLIDABLE;
use super::prop_loader::{part_transforms, record_collision};
use super::pvl::{PvlError, read_level_geometry};
use super::sections::read_sections;
use super::source::navmesh_input;
use crate::bytes::Bytes;
use crate::content::level::CollisionMesh;
use crate::content::navmesh::LevelCollision;

#[derive(Debug, thiserror::Error)]
pub enum NavmeshInputError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Pvl(#[from] PvlError),
    #[error("not a navmesh input")]
    Format,
}

/// Reads a level's collision from its navmesh input.
#[derive(Default, TypePath)]
pub struct LevelCollisionLoader;

impl AssetLoader for LevelCollisionLoader {
    type Asset = LevelCollision;
    type Settings = ();
    type Error = NavmeshInputError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<LevelCollision, NavmeshInputError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let (class, octree, level) = navmesh_input(&data).ok_or(NavmeshInputError::Format)?;
        Ok(LevelCollision { collision: level_collision(octree, level)?, class: class.into() })
    }
}

/// The level's solid collision surfaces, plus its collidable props in their
/// rest pose, as the level importer converts them.
pub fn level_collision(octree: &[u8], level: &[u8]) -> Result<CollisionMesh, PvlError> {
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
