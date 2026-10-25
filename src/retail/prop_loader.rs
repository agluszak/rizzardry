//! Turning decoded props into Bevy meshes in their rest pose.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::keyframes::PathKey;
use super::mesh_record::{FACE_TWO_SIDED, MeshRecord};
use super::prop::Prop;
use super::sections::Light;
use super::space::{UNITS_PER_METRE, retail_direction, retail_position};
use crate::content::level::CollisionMesh;

/// Retail lights props at runtime from the level's lights. This bakes that
/// lighting at load for the rest pose: ambient light, plus each light's
/// linear falloff `intensity * (1 - d/range) * facing`, plus the level's
/// environment offset.
pub struct PropLighting<'a> {
    pub ambient: Vec3,
    pub offset: Vec3,
    pub lights: &'a [Light],
}

impl PropLighting<'_> {
    fn at(&self, position: Vec3, normal: Vec3) -> Vec3 {
        let mut light = self.ambient + self.offset;
        for source in self.lights {
            let range = source.range / UNITS_PER_METRE;
            let to_light = retail_position(source.location) - position;
            let distance = to_light.length();
            if distance >= range || distance == 0.0 {
                continue;
            }
            let facing = normal.dot(to_light / distance).max(0.0);
            light +=
                Vec3::from(source.colour) * source.intensity * (1.0 - distance / range) * facing;
        }
        light
    }
}

/// A path key as a Bevy transform.
pub fn key_transform(key: &PathKey) -> Transform {
    let axis = retail_direction(key.axis).normalize_or(Vec3::Y);
    Transform {
        translation: retail_position(key.position),
        // Mirroring Z reverses the sense of rotation.
        rotation: Quat::from_axis_angle(axis, -key.angle),
        scale: key.scale.map_or(Vec3::ONE, Vec3::from),
    }
}

/// Each detailed part's rest transform and its path poses.
pub fn part_transforms(prop: &Prop) -> Vec<(Transform, Vec<Transform>)> {
    prop.detailed_parts()
        .iter()
        .map(|part| match part.path.as_deref().or(prop.path.as_deref()) {
            Some(path) if !path.is_empty() => {
                let keys: Vec<Transform> = path.iter().map(key_transform).collect();
                (keys[0], if keys.len() > 1 { keys } else { vec![] })
            }
            _ => (Transform::from_translation(retail_position(prop.location)), vec![]),
        })
        .collect()
}

/// One mesh per material of a record, in part-local Bevy space, lit for the
/// part's rest `transform`. Returns (material index, mesh).
pub fn record_meshes(
    record: &MeshRecord,
    transform: &Transform,
    lighting: &PropLighting,
    emission: impl Fn(usize) -> f32,
) -> Vec<(usize, Mesh)> {
    let positions: Vec<Vec3> =
        record.vertices.first().iter().map(|&v| retail_position(v)).collect();
    // Smooth normals from the faces, in Bevy winding.
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for face in &record.faces {
        let [a, b, c] = face.vertices.map(|v| v as usize);
        let normal = (positions[c] - positions[a]).cross(positions[b] - positions[a]);
        for v in [a, b, c] {
            normals[v] += normal;
        }
    }
    let normals: Vec<Vec3> = normals.into_iter().map(|n| n.normalize_or(Vec3::Y)).collect();
    let world = transform.compute_affine();
    let lit: Vec<Vec3> = positions
        .iter()
        .zip(&normals)
        .map(|(&p, &n)| {
            lighting.at(world.transform_point3(p), world.transform_vector3(n).normalize_or(n))
        })
        .collect();

    let mut materials: Vec<usize> = record.faces.iter().map(|f| f.material as usize).collect();
    materials.sort_unstable();
    materials.dedup();
    let mut meshes = Vec::new();
    for material in materials {
        let glow = emission(material);
        let (mut p, mut n, mut uv, mut colour) = (vec![], vec![], vec![], vec![]);
        let mut corners = |face_vertices: [u32; 3], face_uvs: [[f32; 2]; 3], order: [usize; 3]| {
            for corner in order {
                let v = face_vertices[corner] as usize;
                p.push(positions[v].to_array());
                n.push(normals[v].to_array());
                uv.push(face_uvs[corner]);
                let c = (lit[v] + glow).min(Vec3::ONE);
                colour.push([c.x, c.y, c.z, 1.0]);
            }
        };
        for face in record.faces.iter().filter(|f| f.material as usize == material) {
            // Reversed winding keeps retail front faces in front.
            corners(face.vertices, face.uvs, [0, 2, 1]);
            if face.flags & FACE_TWO_SIDED != 0 {
                corners(face.vertices, face.uvs, [0, 1, 2]);
            }
        }
        let count = p.len() as u32;
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colour);
        mesh.insert_indices(Indices::U32((0..count).collect()));
        meshes.push((material, mesh));
    }
    meshes
}

/// A record's faces as part-local collision triangles.
pub fn record_collision(record: &MeshRecord) -> CollisionMesh {
    CollisionMesh {
        vertices: record.vertices.first().iter().map(|&v| retail_position(v)).collect(),
        triangles: record
            .faces
            .iter()
            .map(|f| [f.vertices[0], f.vertices[2], f.vertices[1]])
            .collect(),
        walkable: vec![false; record.faces.len()],
    }
}
