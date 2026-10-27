//! Bake and cache every retail level's navmeshes ahead of play, as the game
//! would on entering each level, so no first entry waits for a bake:
//!
//! cargo run --release --example navmesh_prebake -- --retail <dir>

use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;

use rizzardry::content::navmesh::navmesh_input_path;
use rizzardry::game::world::navigation::asset::{CACHE, NavmeshLoader};
use rizzardry::game::world::navigation::bake::SIZE_CLASSES;
use rizzardry::retail::files::RetailFiles;
use rizzardry::retail::source::navmesh_input_bytes;

#[derive(clap::Parser)]
#[command(about = "Bake and cache every retail level's navmeshes ahead of play.")]
struct Args {
    /// The Wizardry 8 installation directory.
    #[arg(long, env = "RIZZARDRY_RETAIL")]
    retail: std::path::PathBuf,
}

fn main() {
    let args = Args::parse();
    let files = RetailFiles::open(&args.retail).expect("a Wizardry 8 installation");
    let loader = NavmeshLoader {
        cache: bevy::asset::io::file::FileAssetReader::get_base_path().join(CACHE),
    };
    let work: Vec<(String, usize)> = files
        .names()
        .filter(|name| name.ends_with(".pvl"))
        .flat_map(|level| (0..SIZE_CLASSES.len()).map(move |class| (level.to_owned(), class)))
        .collect();
    let (next, failed) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while let Some((level, class)) = work.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let path = navmesh_input_path(level, *class);
                    let result = navmesh_input_bytes(&files, level, *class as u8)
                        .ok_or_else(|| "no collision".to_owned())
                        .and_then(|input| input.map_err(|e| e.to_string()))
                        .and_then(|input| {
                            loader.navmesh(path.as_ref(), &input).map_err(|e| e.to_string())
                        });
                    match result {
                        Ok(_) => println!("{path}"),
                        Err(error) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            println!("{path}: FAILED: {error}");
                        }
                    }
                }
            });
        }
    });
    let failed = failed.into_inner();
    println!("{} of {} navmeshes cached", work.len() - failed, work.len());
    if failed > 0 {
        std::process::exit(1);
    }
}
