//! Bake retail levels' navmeshes outside the game and report the results:
//! repairs, and how connected the walkable area is.
//!
//! cargo run --example navmesh_bake -- --retail <dir> [level.pvl] [--climb 0.5]

use std::collections::HashMap;
use std::time::Instant;

use bevy::math::{Vec2, Vec3};
use bevy_landmass::NavigationMesh3d;
use clap::Parser;

use rizzardry::game::world::navigation::bake::{NavmeshBakeSettings, SIZE_CLASSES, bake};
use rizzardry::retail::files::RetailFiles;
use rizzardry::retail::navmesh::level_collision;

#[derive(clap::Parser)]
#[command(about = "Bake retail levels' navmeshes outside the game and report the results.")]
struct Args {
    /// The Wizardry 8 installation directory.
    #[arg(long, env = "RIZZARDRY_RETAIL")]
    retail: std::path::PathBuf,
    /// One level to inspect; all levels by default.
    level: Option<String>,
    /// Override the walkable climb (metres).
    #[arg(long)]
    climb: Option<f32>,
    /// Override the agent radius (metres); by default every size class.
    #[arg(long)]
    radius: Option<f32>,
    /// Override the agent height (metres).
    #[arg(long)]
    height: Option<f32>,
    /// Report where the largest components come closest to each other.
    #[arg(long)]
    gaps: bool,
}

/// The closest pair of vertices between two components.
fn closest(mesh: &NavigationMesh3d, a: &[usize], b: &[usize]) -> (Vec3, Vec3) {
    let points = |c: &[usize]| -> Vec<Vec3> {
        c.iter().flat_map(|&p| mesh.polygons[p].iter().map(|&v| mesh.vertices[v])).collect()
    };
    let (pa, pb) = (points(a), points(b));
    let mut best = (f32::MAX, Vec3::ZERO, Vec3::ZERO);
    for &x in &pa {
        for &y in &pb {
            let d = x.distance_squared(y);
            if d < best.0 {
                best = (d, x, y);
            }
        }
    }
    (best.1, best.2)
}

/// Connected components of polygons sharing an edge: (polygon count, area,
/// polygon indices) per component, largest area first.
fn components(mesh: &NavigationMesh3d) -> Vec<(usize, f32, Vec<usize>)> {
    let mut parent: Vec<usize> = (0..mesh.polygons.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    for (index, polygon) in mesh.polygons.iter().enumerate() {
        for i in 0..polygon.len() {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            if let Some(&other) = edges.get(&(a.min(b), a.max(b))) {
                let (x, y) = (find(&mut parent, index), find(&mut parent, other));
                parent[x] = y;
            } else {
                edges.insert((a.min(b), a.max(b)), index);
            }
        }
    }
    let flat = |v: Vec3| Vec2::new(v.x, v.z);
    let mut totals: HashMap<usize, (usize, f32, Vec<usize>)> = HashMap::new();
    for (index, polygon) in mesh.polygons.iter().enumerate() {
        let origin = flat(mesh.vertices[polygon[0]]);
        let area: f32 = polygon
            .windows(2)
            .skip(1)
            .map(|w| {
                (flat(mesh.vertices[w[0]]) - origin).perp_dot(flat(mesh.vertices[w[1]]) - origin)
            })
            .sum::<f32>()
            .abs()
            / 2.0;
        let entry = totals.entry(find(&mut parent, index)).or_default();
        entry.0 += 1;
        entry.1 += area;
        entry.2.push(index);
    }
    let mut list: Vec<_> = totals.into_values().collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    list
}

fn main() {
    let args = Args::parse();
    let files = RetailFiles::open(&args.retail).unwrap();
    let names: Vec<String> = match &args.level {
        Some(name) => vec![name.clone()],
        None => files.names().filter(|n| n.ends_with(".pvl")).map(str::to_owned).collect(),
    };
    let mut settings = NavmeshBakeSettings::default();
    settings.walkable_climb = args.climb.unwrap_or(settings.walkable_climb);
    settings.agent_height = args.height.unwrap_or(settings.agent_height);
    let radii = args.radius.map_or(SIZE_CLASSES.to_vec(), |radius| vec![radius]);
    for (name, radius) in names.iter().flat_map(|n| radii.iter().map(move |&r| (n, r))) {
        settings.agent_radius = radius;
        let name = format!("{name} r={radius}");
        let level_name = name.split(' ').next().unwrap();
        let octree =
            files.read(&level_name.replace(".pvl", ".oct")).expect("octree exists").unwrap();
        let level = files.read(level_name).expect("level exists").unwrap();
        let collision = level_collision(&octree, &level).unwrap();
        let start = Instant::now();
        let (mesh, repair) = match bake(&collision, &settings) {
            Ok(result) => result,
            Err(error) => {
                println!("{name}: FAILED after {:.1?}: {error}", start.elapsed());
                continue;
            }
        };
        let elapsed = start.elapsed();
        let parts = components(&mesh);
        let area: f32 = parts.iter().map(|p| p.1).sum();
        let largest: Vec<String> =
            parts.iter().take(5).map(|(n, a, _)| format!("{:.0}m²/{n}", a)).collect();
        let valid = match mesh.clone().validate() {
            Ok(_) => "valid".to_owned(),
            Err(e) => format!("INVALID {e:?}"),
        };
        println!(
            "{name}: {valid}, {} polygons, {:.0} m² in {} components (largest {}) in {elapsed:.1?}; {repair:?}",
            parts.iter().map(|p| p.0).sum::<usize>(),
            area,
            parts.len(),
            largest.join(", "),
        );

        if args.gaps {
            for i in 1..parts.len().min(4) {
                let (x, y) = closest(&mesh, &parts[0].2, &parts[i].2);
                println!(
                    "  component 0 to {i}: {:.1} m apart, {x:.2} -> {y:.2} (height difference {:.2} m)",
                    x.distance(y),
                    y.y - x.y
                );
            }
        }
    }
}
