//! Baking a level's navmesh: the Recast pipeline over the level's solid
//! triangles, converted to a Landmass navigation mesh.
//!
//! This is pure computation; the navmesh loader (`super::asset`) runs it
//! when a level is entered and caches the result.

use bevy::math::{Vec2, Vec3, Vec3A};
use bevy::prelude::UVec3;
use bevy_landmass::{HeightNavigationMesh3d, HeightPolygon, NavigationMesh3d};

use crate::content::level::CollisionMesh;
use rerecast::{
    AreaType, ConfigBuilder, DetailNavmesh, HeightfieldBuilder, PolygonNavmesh, TriMesh,
};
use serde::{Deserialize, Serialize};

/// Walker radius classes, in metres: each level gets one navmesh per class,
/// and a walker uses the smallest class that fits it. Wider walkers navigate
/// the 1 m class (`population::MAX_NAVIGATION_RADIUS`: wider navmeshes erode
/// away narrow ground), so only these two are baked. In the retail monster
/// table effective radii fall at up to 0.6 m (41 records), 1.0 (150), 1.6
/// (254) and 3.0 (100), and beyond for 39 bosses.
pub const SIZE_CLASSES: [f32; 2] = [0.6, 1.0];

/// The size class for a walker of `radius` metres.
pub fn size_class(radius: f32) -> usize {
    SIZE_CLASSES.iter().position(|&class| radius <= class).unwrap_or(SIZE_CLASSES.len() - 1)
}

/// Parameters of one navmesh bake. Part of the cache key, so changing a
/// default re-bakes cached navmeshes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavmeshBakeSettings {
    /// Walkers are kept this far from walls. The bake of each size class
    /// replaces it with the class radius.
    pub agent_radius: f32,
    pub agent_height: f32,
    /// Highest step a walker climbs.
    pub walkable_climb: f32,
    /// Recast's slope limit; walkable area itself comes from surface flags.
    pub walkable_slope_degrees: f32,
    /// Horizontal cells are `agent_radius / cell_size_fraction` wide.
    pub cell_size_fraction: f32,
}

impl Default for NavmeshBakeSettings {
    fn default() -> Self {
        NavmeshBakeSettings {
            agent_radius: 0.4,
            agent_height: 1.6,
            walkable_climb: crate::game::world::STEP_HEIGHT,
            walkable_slope_degrees: 50.0,
            // ~0.27 m cells keep a 500 m level within budget.
            cell_size_fraction: 1.5,
        }
    }
}

/// What the precision repair had to change; see [`repair`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    pub removed_vertices: usize,
    /// Polygons with no area in the walking plane.
    pub degenerate_polygons: usize,
    /// Polygons wound backwards (folded over), dropped, and their area in
    /// square metres. Rare; the bake warns when it happens.
    pub inverted_polygons: usize,
    pub inverted_area: u32,
    pub flipped_triangles: usize,
}

/// Bake the navmesh for a level's collision. Walkable area comes from the
/// collision surfaces' flags, not from slope; every solid triangle is an
/// obstacle.
pub fn bake(
    collision: &CollisionMesh,
    settings: &NavmeshBakeSettings,
) -> Result<(NavigationMesh3d, RepairReport), String> {
    let trimesh = TriMesh {
        vertices: collision.vertices.iter().map(|&v| Vec3A::from(v)).collect(),
        indices: collision.triangles.iter().map(|&t| UVec3::from_array(t)).collect(),
        area_types: collision
            .walkable
            .iter()
            .map(
                |&walkable| {
                    if walkable { AreaType::DEFAULT_WALKABLE } else { AreaType::NOT_WALKABLE }
                },
            )
            .collect(),
    };
    let mut builder = ConfigBuilder {
        agent_radius: settings.agent_radius,
        agent_height: settings.agent_height,
        walkable_climb: settings.walkable_climb,
        walkable_slope_angle: settings.walkable_slope_degrees.to_radians(),
        cell_size_fraction: settings.cell_size_fraction,
        ..Default::default()
    };
    builder.aabb = trimesh.compute_aabb().ok_or("the level has no solid triangles")?;
    let config = builder.build();

    // The pipeline of `bevy_rerecast_core`'s generator.
    let error = |e: &dyn std::fmt::Display| e.to_string();
    let mut heightfield = HeightfieldBuilder {
        aabb: config.aabb,
        cell_size: config.cell_size,
        cell_height: config.cell_height,
    }
    .build()
    .map_err(|e| error(&e))?;
    heightfield.rasterize_triangles(&trimesh, config.walkable_climb).map_err(|e| error(&e))?;
    heightfield.filter_low_hanging_walkable_obstacles(config.walkable_climb);
    heightfield.filter_ledge_spans(config.walkable_height, config.walkable_climb);
    heightfield.filter_walkable_low_height_spans(config.walkable_height);
    let mut compact = heightfield
        .into_compact(config.walkable_height, config.walkable_climb)
        .map_err(|e| error(&e))?;
    compact.erode_walkable_area(config.walkable_radius);
    compact.build_distance_field();
    compact
        .build_regions(config.border_size, config.min_region_area, config.merge_region_area)
        .map_err(|e| error(&e))?;
    let contours = compact.build_contours(
        config.max_simplification_error,
        config.max_edge_len,
        config.contour_flags,
    );
    let polygons =
        contours.into_polygon_mesh(config.max_vertices_per_polygon).map_err(|e| error(&e))?;
    let detail = DetailNavmesh::new(
        &polygons,
        &compact,
        config.detail_sample_dist,
        config.detail_sample_max_error,
    )
    .map_err(|e| error(&e))?;
    let mut mesh = to_landmass(&polygons, &detail);
    let report = repair(&mut mesh)?;
    Ok((mesh, report))
}

/// Rerecast's polygon and detail meshes as an unvalidated Landmass navmesh.
///
/// Adapted from `landmass_rerecast::convert_rerecast_navmesh_to_landmass_navmesh`
/// (MIT OR Apache-2.0).
fn to_landmass(polygon: &PolygonNavmesh, detail: &DetailNavmesh) -> NavigationMesh3d {
    let origin = polygon.aabb.min;
    let to_world = Vec3::new(polygon.cell_size, polygon.cell_height, polygon.cell_size);
    let per_polygon = polygon.max_vertices_per_polygon as usize;
    NavigationMesh3d {
        vertices: polygon.vertices.iter().map(|v| origin + v.as_vec3() * to_world).collect(),
        polygons: (0..polygon.polygon_count())
            .map(|i| {
                polygon.polygons[i * per_polygon..][..per_polygon]
                    .iter()
                    .rev()
                    .filter(|&&v| v != PolygonNavmesh::NO_INDEX)
                    .map(|&v| v as usize)
                    .collect()
            })
            .collect(),
        polygon_type_indices: polygon.areas.iter().map(|a| a.0 as usize).collect(),
        height_mesh: Some(HeightNavigationMesh3d {
            polygons: detail
                .meshes
                .iter()
                .map(|m| HeightPolygon {
                    base_vertex_index: m.base_vertex_index,
                    vertex_count: m.vertex_count,
                    base_triangle_index: m.base_triangle_index,
                    triangle_count: m.triangle_count,
                })
                .collect(),
            triangles: detail.triangles.iter().map(|&[a, b, c]| [a, c, b]).collect(),
            vertices: detail.vertices.clone(),
        }),
    }
}

/// The plane Landmass validates Bevy-space meshes in.
fn plane(v: Vec3) -> Vec2 {
    Vec2::new(v.x, -v.z)
}

/// Whether Landmass accepts the corner `center` between `left` and `right`
/// of a polygon in its (reversed) winding.
fn convex_corner(left: Vec2, center: Vec2, right: Vec2) -> bool {
    let (left_edge, right_edge) = (left - center, right - center);
    let turn = right_edge.perp_dot(left_edge);
    turn > 0.0 || (turn == 0.0 && right_edge.dot(left_edge) < 0.0)
}

/// Make Recast output pass Landmass validation, fixing only numerical
/// artifacts.
///
/// Recast quantizes vertices to cells. On ramps this can leave a polygon with
/// two vertices at one plane position (stacked at different heights), a spike,
/// or all its vertices on one line: a polygon with no area. Landmass rejects
/// the whole mesh for any of these. Duplicate and spike vertices are removed
/// and zero-area polygons dropped with their detail mesh, which loses no
/// walkable area; flipped detail triangles are rewound. Rarely, a polygon is
/// folded over (wound backwards); it is dropped and reported. A polygon wound
/// correctly but concave is a real error, not something to drop.
pub fn repair(mesh: &mut NavigationMesh3d) -> Result<RepairReport, String> {
    let mut report = RepairReport::default();
    let vertices = &mesh.vertices;
    let mut keep = vec![true; mesh.polygons.len()];
    for (index, polygon) in mesh.polygons.iter_mut().enumerate() {
        // Landmass checks the reversed order; work in that order.
        polygon.reverse();
        let mut changed = true;
        while changed && polygon.len() >= 3 {
            changed = false;
            for i in 0..polygon.len() {
                let n = polygon.len();
                let [l, c, r] = [polygon[(i + n - 1) % n], polygon[i], polygon[(i + 1) % n]];
                let (left, center, right) =
                    (plane(vertices[l]), plane(vertices[c]), plane(vertices[r]));
                let (left_edge, right_edge) = (left - center, right - center);
                let degenerate = left_edge == Vec2::ZERO
                    || right_edge == Vec2::ZERO
                    || (right_edge.perp_dot(left_edge) == 0.0 && right_edge.dot(left_edge) > 0.0);
                if degenerate {
                    polygon.remove(i);
                    report.removed_vertices += 1;
                    changed = true;
                    break;
                }
            }
        }
        let n = polygon.len();
        let corners: Vec<Vec2> = polygon.iter().map(|&v| plane(vertices[v])).collect();
        let area: f32 = (0..n).map(|i| corners[i].perp_dot(corners[(i + 1) % n])).sum();
        polygon.reverse();
        if n < 3 || area == 0.0 {
            keep[index] = false;
            report.degenerate_polygons += 1;
        } else if area < 0.0 {
            keep[index] = false;
            report.inverted_polygons += 1;
            report.inverted_area += (-area / 2.0).round() as u32;
        } else if !(0..n)
            .all(|i| convex_corner(corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]))
        {
            return Err(format!("navmesh polygon {index} is concave: {corners:?}"));
        }
    }

    if let Some(height) = &mut mesh.height_mesh {
        for polygon in &height.polygons {
            let first = polygon.base_triangle_index as usize;
            let local =
                |i: u8| plane(height.vertices[polygon.base_vertex_index as usize + i as usize]);
            for triangle in &mut height.triangles[first..first + polygon.triangle_count as usize] {
                // Landmass swaps the last two corners before checking.
                let [a, c, b] = *triangle;
                if (local(b) - local(a)).perp_dot(local(c) - local(a)) < 0.0 {
                    triangle.swap(1, 2);
                    report.flipped_triangles += 1;
                }
            }
        }
        let mut index = 0;
        height.polygons.retain(|_| {
            index += 1;
            keep[index - 1]
        });
    }
    let mut index = 0;
    mesh.polygons.retain(|_| {
        index += 1;
        keep[index - 1]
    });
    let mut index = 0;
    mesh.polygon_type_indices.retain(|_| {
        index += 1;
        keep[index - 1]
    });
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_with(extra: Vec3) -> NavigationMesh3d {
        // A unit square in Bevy XZ, wound so Landmass accepts it, with an
        // extra vertex inserted on one edge.
        NavigationMesh3d {
            vertices: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, 0.0),
                extra,
            ],
            polygons: vec![vec![0, 1, 2, 3, 4]],
            polygon_type_indices: vec![0],
            height_mesh: None,
        }
    }

    #[test]
    fn a_duplicated_vertex_is_removed() {
        // Same plane position as vertex 0 at another height: a zero-length
        // edge in Landmass's plane.
        let mut mesh = square_with(Vec3::new(0.0, 0.3, 0.0));
        assert!(mesh.clone().validate().is_err());
        let report = repair(&mut mesh).unwrap();
        assert_eq!(report, RepairReport { removed_vertices: 1, ..Default::default() });
        assert!(mesh.validate().is_ok());
    }

    #[test]
    fn a_reflex_polygon_is_an_error() {
        let mut mesh = square_with(Vec3::new(0.5, 0.0, -0.2));
        assert!(repair(&mut mesh).is_err());
    }

    #[test]
    fn a_folded_polygon_is_dropped_and_counted() {
        // The square from `square_with`, wound backwards.
        let mut mesh = square_with(Vec3::new(0.5, 0.0, 0.0));
        mesh.polygons = vec![vec![3, 2, 1, 0]];
        let report = repair(&mut mesh).unwrap();
        assert_eq!((report.inverted_polygons, report.inverted_area), (1, 1));
        assert!(mesh.polygons.is_empty());
    }

    #[test]
    fn a_polygon_without_area_is_dropped() {
        // Three corners on one line in the plane, at different heights.
        let mut mesh = NavigationMesh3d {
            vertices: vec![Vec3::ZERO, Vec3::new(1.0, 0.2, 0.0), Vec3::new(2.0, 0.4, 0.0)],
            polygons: vec![vec![0, 1, 2]],
            polygon_type_indices: vec![0],
            height_mesh: None,
        };
        let report = repair(&mut mesh).unwrap();
        assert_eq!(report.degenerate_polygons, 1);
        assert!(mesh.polygons.is_empty());
    }
}
