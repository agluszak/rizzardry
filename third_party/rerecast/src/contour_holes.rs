//! Merging region holes into their outlines.
//!
//! Rizzardry patch: a port of recastnavigation's `mergeRegionHoles`
//! (`RecastContour.cpp`, "Merge holes if needed"), which upstream rerecast
//! omits. Without it, a region surrounding an obstacle produces a separate,
//! backwards-wound hole contour: triangulating it fails, and the outline is
//! triangulated across the obstacle as if it were walkable.
//!
//! Differences from the C++: holes and diagonals are sorted stably, and the
//! intersection test skips the diagonal's own outline vertex (the C++ passes
//! the hole index, `diags[i].vert`, where `diags[j].vert` is meant).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::contours::Contour;
use crate::region::RegionId;


fn points(contour: &Contour) -> Vec<[i32; 2]> {
    contour.vertices.iter().map(|(v, _)| [i32::from(v.x), i32::from(v.z)]).collect()
}

fn signed_area(points: &[[i32; 2]]) -> i32 {
    let n = points.len();
    let mut area = 0;
    for i in 0..n {
        let (vi, vj) = (points[i], points[(i + n - 1) % n]);
        area += vi[0] * vj[1] - vj[0] * vi[1];
    }
    (area + 1) / 2
}

fn area2(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> i32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])
}

fn left(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> bool {
    area2(a, b, c) < 0
}

fn left_on(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> bool {
    area2(a, b, c) <= 0
}

fn collinear(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> bool {
    area2(a, b, c) == 0
}

fn intersect_prop(a: [i32; 2], b: [i32; 2], c: [i32; 2], d: [i32; 2]) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) ^ left(a, b, d)) && (left(c, d, a) ^ left(c, d, b))
}

fn between(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a[0] != b[0] {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    } else {
        (a[1] <= c[1] && c[1] <= b[1]) || (a[1] >= c[1] && c[1] >= b[1])
    }
}

fn intersect(a: [i32; 2], b: [i32; 2], c: [i32; 2], d: [i32; 2]) -> bool {
    intersect_prop(a, b, c, d)
        || between(a, b, c)
        || between(a, b, d)
        || between(c, d, a)
        || between(c, d, b)
}

/// Whether segment `d0`-`d1` crosses an edge of `verts` not incident to
/// `skip`.
fn intersect_seg_contour(d0: [i32; 2], d1: [i32; 2], skip: Option<usize>, verts: &[[i32; 2]]) -> bool {
    let n = verts.len();
    for k in 0..n {
        let k1 = (k + 1) % n;
        if skip == Some(k) || skip == Some(k1) {
            continue;
        }
        let (p0, p1) = (verts[k], verts[k1]);
        if d0 == p0 || d1 == p0 || d0 == p1 || d1 == p1 {
            continue;
        }
        if intersect(d0, d1, p0, p1) {
            return true;
        }
    }
    false
}

fn in_cone(i: usize, verts: &[[i32; 2]], pj: [i32; 2]) -> bool {
    let n = verts.len();
    let pi = verts[i];
    let pi1 = verts[(i + 1) % n];
    let pin1 = verts[(i + n - 1) % n];
    if left_on(pin1, pi, pi1) {
        return left(pi, pj, pin1) && left(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
}

/// A contour's leftmost (then lowest) vertex and its index.
fn leftmost(contour: &Contour) -> ([i32; 2], usize) {
    let verts = points(contour);
    let mut best = 0;
    for (i, v) in verts.iter().enumerate() {
        if v[0] < verts[best][0] || (v[0] == verts[best][0] && v[1] < verts[best][1]) {
            best = i;
        }
    }
    (verts[best], best)
}

/// Splice `hole` into `outline` through the diagonal between outline vertex
/// `ia` and hole vertex `ib`.
fn merge_contours(outline: &mut Contour, hole: &mut Contour, ia: usize, ib: usize) {
    let (na, nb) = (outline.vertices.len(), hole.vertices.len());
    let mut merged = Vec::with_capacity(na + nb + 2);
    merged.extend((0..=na).map(|i| outline.vertices[(ia + i) % na]));
    merged.extend((0..=nb).map(|i| hole.vertices[(ib + i) % nb]));
    outline.vertices = merged;
    hole.vertices.clear();
}

/// Merge every region's backwards-wound hole contours into its outline,
/// then drop the emptied holes.
pub(crate) fn merge_region_holes(contours: &mut Vec<Contour>) {
    let mut regions: BTreeMap<RegionId, (Option<usize>, Vec<usize>)> = BTreeMap::new();
    for (index, contour) in contours.iter().enumerate() {
        let entry = regions.entry(contour.region).or_default();
        if signed_area(&points(contour)) < 0 {
            entry.1.push(index);
        } else {
            if entry.0.is_some() {
                #[cfg(feature = "tracing")]
                tracing::error!("Multiple outlines for region {:?}", contour.region);
            }
            entry.0 = Some(index);
        }
    }
    for (region, (outline, mut holes)) in regions {
        if holes.is_empty() {
            continue;
        }
        let Some(outline) = outline else {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Bad outline for region {region:?}, contour simplification is likely too aggressive."
            );
            let _ = region;
            continue;
        };
        // Merge holes left to right.
        holes.sort_by_key(|&h| leftmost(&contours[h]).0);

        for i in 0..holes.len() {
            let hole_points = points(&contours[holes[i]]);
            let mut best_vertex = leftmost(&contours[holes[i]]).1;
            let mut found = None;
            for _ in 0..hole_points.len() {
                let outline_points = points(&contours[outline]);
                let corner = hole_points[best_vertex];
                // Outline vertices whose cone contains the corner, nearest first.
                let mut diagonals: Vec<(i32, usize)> = (0..outline_points.len())
                    .filter(|&j| in_cone(j, &outline_points, corner))
                    .map(|j| {
                        let (dx, dz) =
                            (outline_points[j][0] - corner[0], outline_points[j][1] - corner[1]);
                        (dx * dx + dz * dz, j)
                    })
                    .collect();
                diagonals.sort_by_key(|&(distance, _)| distance);
                // The shortest diagonal crossing neither the outline nor the
                // holes still to merge.
                found = diagonals.iter().map(|&(_, j)| j).find(|&j| {
                    let pt = outline_points[j];
                    !intersect_seg_contour(pt, corner, Some(j), &outline_points)
                        && holes[i..].iter().all(|&k| {
                            !intersect_seg_contour(pt, corner, None, &points(&contours[k]))
                        })
                });
                if found.is_some() {
                    break;
                }
                best_vertex = (best_vertex + 1) % hole_points.len();
            }
            let Some(index) = found else {
                #[cfg(feature = "tracing")]
                tracing::warn!("mergeHoles: Failed to find merge points for region {region:?}");
                continue;
            };
            let mut hole = core::mem::take(&mut contours[holes[i]]);
            merge_contours(&mut contours[outline], &mut hole, index, best_vertex);
            contours[holes[i]] = hole;
        }
    }
    contours.retain(|contour| !contour.vertices.is_empty());
}
