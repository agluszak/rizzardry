//! Turning decoded props into Bevy meshes in their rest pose.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::keyframes::PathKey;
use super::mesh_record::{FACE_TWO_SIDED, MeshRecord};
use super::prop::Prop;
use super::sections::Light;
use super::space::{retail_position, retail_rotation};
use crate::content::UNITS_PER_METRE;
use crate::content::level::{CollisionMesh, LevelLighting, PointLightSource, VertexLighting};

/// Retail lights props at runtime from the level's lights. This bakes that
/// lighting at load for the rest pose (`LevelLighting::at`), with the sky
/// light (`ambient` at load) added per time of day.
pub struct PropLighting<'a> {
    pub ambient: Vec3,
    pub level: &'a LevelLighting,
}

/// The level lights that reach props and actors. `Sun` lights are
/// ambient-only: retail's prop bake skips them (Whizzardry `update.cpp`) and
/// the day ramp stands in for them.
pub fn level_lighting(lights: &[Light], offset: [f32; 3]) -> LevelLighting {
    let is_sun = |light: &Light| light.name.get(..3).is_some_and(|n| n.eq_ignore_ascii_case("sun"));
    LevelLighting {
        offset: Vec3::from(offset),
        lights: lights
            .iter()
            .filter(|light| !is_sun(light) && (light.runtime || light.enabled))
            .map(|light| PointLightSource {
                name: light.name.to_uppercase(),
                runtime: light.runtime,
                enabled: light.enabled,
                position: retail_position(light.location),
                colour: Vec3::from(light.colour) * light.intensity,
                range: light.range / UNITS_PER_METRE,
            })
            .collect(),
        sun: lights
            .iter()
            .find(|light| light.name.eq_ignore_ascii_case("sun"))
            .map(|light| retail_position(light.location)),
    }
}

/// A path key as a Bevy transform.
pub fn key_transform(key: &PathKey) -> Transform {
    Transform {
        translation: retail_position(key.position),
        rotation: retail_rotation(key.angle, key.axis),
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
) -> Vec<(usize, Mesh, VertexLighting)> {
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
            lighting
                .level
                .at(world.transform_point3(p), Some(world.transform_vector3(n).normalize_or(n)))
        })
        .collect();

    let mut materials: Vec<usize> = record.faces.iter().map(|f| f.material as usize).collect();
    materials.sort_unstable();
    materials.dedup();
    let mut meshes = Vec::new();
    for material in materials {
        let glow = emission(material);
        let (mut p, mut n, mut uv) = (vec![], vec![], vec![]);
        // Props take the sky light in full.
        let mut vertex_lighting = VertexLighting { reflectance: Vec3::ONE, ..default() };
        let mut corners = |face_vertices: [u32; 3], face_uvs: [[f32; 2]; 3], order: [usize; 3]| {
            for corner in order {
                let v = face_vertices[corner] as usize;
                p.push(positions[v].to_array());
                n.push(normals[v].to_array());
                uv.push(face_uvs[corner]);
                vertex_lighting.fixed.push(lit[v] + glow);
                vertex_lighting.sun.push(1.0);
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
        // Kept in the main world too, so the colours can be redone.
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, p);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_lighting.colours(lighting.ambient));
        mesh.insert_indices(Indices::U32((0..count).collect()));
        meshes.push((material, mesh, vertex_lighting));
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
