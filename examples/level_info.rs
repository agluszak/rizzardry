//! Print a summary of a retail level's compiled geometry.
//!
//! cargo run --example level_info -- --retail <dir> [levels/monastery/monastery1.pvl]

use clap::Parser;
use rizzardry::retail::{files::RetailFiles, pvl::read_level_geometry};

#[derive(clap::Parser)]
#[command(about = "Print a summary of retail levels' compiled geometry.")]
struct Args {
    /// The Wizardry 8 installation directory.
    #[arg(long, env = "RIZZARDRY_RETAIL")]
    retail: std::path::PathBuf,
    /// One level to inspect; all levels by default.
    level: Option<String>,
}

fn main() {
    let args = Args::parse();
    let files = RetailFiles::open(&args.retail).unwrap();
    let names: Vec<String> = match args.level {
        Some(name) => vec![name],
        None => files.names().filter(|n| n.ends_with(".pvl")).map(str::to_owned).collect(),
    };
    for name in names {
        let data = files.read(&name).expect("level exists").unwrap();
        let level = read_level_geometry(&data).unwrap();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        let (mut uv_min, mut uv_max) = ([f32::MAX; 2], [f32::MIN; 2]);
        let mut polygons = 0;
        let mut light_max = 0.0f32;
        for mesh in &level.meshes {
            polygons += mesh.polygons.len();
            for p in &mesh.positions {
                for a in 0..3 {
                    min[a] = min[a].min(p[a]);
                    max[a] = max[a].max(p[a]);
                }
            }
            for uv in &mesh.uvs {
                for a in 0..2 {
                    uv_min[a] = uv_min[a].min(uv[a]);
                    uv_max[a] = uv_max[a].max(uv[a]);
                }
            }
            for l in &mesh.vertex_light {
                light_max = light_max.max(l[0]).max(l[1]).max(l[2]);
            }
        }
        println!(
            "{name}: {} meshes ({} roots, {} linked, {} weighted), {polygons} polygons, {} materials, end {} of {}",
            level.meshes.len(),
            level.root_count,
            level.meshes.iter().filter(|m| m.link.is_some()).count(),
            level.meshes.iter().filter(|m| m.weighted).count(),
            level.materials.len(),
            level.end,
            data.len()
        );
        println!("  bounds {min:?} .. {max:?}; uv {uv_min:?} .. {uv_max:?}; max light {light_max}");
        if std::env::var("VERBOSE").is_ok() {
            for (i, m) in level.materials.iter().enumerate().take(40) {
                println!(
                    "  material {i}: v{} {:?} opacity {} flags {:#x} anim {} diffuse {:?} emissive {:?} emission {}",
                    m.version,
                    m.textures,
                    m.opacity,
                    m.surface_flags,
                    m.animation_mode,
                    m.diffuse,
                    m.emissive,
                    m.emission
                );
            }
            let m = &level.meshes[0];
            println!(
                "  mesh0 sample normal {:?} light {:?} sun {:?} plane {:?}",
                m.normals[0],
                m.vertex_light[0],
                m.sunlight.as_ref().map(|s| s[0]),
                m.polygons[0].plane
            );
        }
    }
}
