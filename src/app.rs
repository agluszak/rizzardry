//! Composition root: plugins, settings and startup.

use std::path::PathBuf;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_enhanced_input::EnhancedInputPlugin;

use crate::content::levels::{DEFAULT_SKY, LevelInfo};
use crate::dev::{FacePropOnEntry, face_prop_on_entry};
use crate::dev::{
    OverheadView, ScreenshotAndExit, ScriptedInteract, ScriptedWalk, TestAgents, log_test_agents,
    overhead_view, screenshot_and_exit, scripted_interact, scripted_walk, spawn_test_agents,
};
use crate::game::party::PartyPlugin;
use crate::game::party::body::PartyStart;
use crate::game::time::{GameClock, TimePlugin};
use crate::game::world::WorldPlugin;
use crate::game::world::level::Level;
use crate::game::world::navigation::asset::navmesh_path;
use crate::game::world::navigation::bake::SIZE_CLASSES;
use crate::presentation::PresentationPlugin;
use crate::retail::RetailPlugin;
use crate::retail::bytes::Bytes;
use crate::retail::files::RetailFiles;
use crate::retail::path::normalize;
use crate::retail::pvl::read_level_geometry;
use crate::retail::sections::read_sections;
use crate::retail::space::{retail_position, retail_yaw};
use crate::retail::trigger::TriggerKind;

pub const DEFAULT_LEVEL: &str = "levels/monastery/monastery1.pvl";
/// A new game enters the monastery at entrance 0 (location code `MO1`).
pub const DEFAULT_ENTRANCE: &str = "MO100";

/// Rizzardry: a Rust and Bevy rewrite of Wizardry 8.
#[derive(Debug, Clone, clap::Parser)]
#[command(version, about)]
pub struct Options {
    /// The Wizardry 8 installation directory.
    #[arg(long, env = "RIZZARDRY_RETAIL")]
    pub retail: PathBuf,
    /// The level to enter, as a retail path.
    #[arg(long, default_value = DEFAULT_LEVEL)]
    pub level: String,
    /// The entrance trigger to start at. Defaults to the new-game entrance
    /// for the default level.
    #[arg(long)]
    pub entrance: Option<String>,
    /// An explicit eye position in metres, overriding the entrance.
    #[arg(long, value_name = "X,Y,Z", value_parser = parse_vec3)]
    pub start: Option<Vec3>,
    /// Initial yaw in degrees, with --start.
    #[arg(long, default_value_t = 0.0)]
    pub yaw: f32,
    /// Developer: capture the view shortly after entering, then exit.
    #[arg(long, value_name = "FILE.png")]
    pub screenshot: Option<PathBuf>,
    /// Developer: seconds after entering to take the screenshot (default: 2
    /// plus any scripted walk).
    #[arg(long, value_name = "SECONDS")]
    pub screenshot_after: Option<f32>,
    /// Developer: hold "walk forward" for this many seconds after entering.
    #[arg(long, value_name = "SECONDS")]
    pub walk: Option<f32>,
    /// Developer: once the level is built, stand 2 m in front of this prop.
    #[arg(long, value_name = "NAME")]
    pub face_prop: Option<String>,
    /// Start at this hour of the day (0-24) instead of a new game's noon.
    #[arg(long, value_name = "HOUR")]
    pub hour: Option<f32>,
    /// Developer: press "interact" once, this many seconds after entering.
    #[arg(long, value_name = "SECONDS")]
    pub interact_at: Option<f32>,
    /// Developer: spawn this many stand-in monsters that chase the party.
    #[arg(long, value_name = "COUNT", default_value_t = 0)]
    pub test_agents: usize,
    /// Developer: the stand-in monsters' radius in metres.
    #[arg(long, value_name = "METRES", default_value_t = 0.35)]
    pub test_agent_radius: f32,
    /// Developer: draw the navmesh, agent paths and avoidance.
    #[arg(long)]
    pub nav_debug: bool,
    /// Developer: view the party from this many metres straight above.
    #[arg(long, value_name = "METRES")]
    pub overhead: Option<f32>,
}

fn parse_vec3(text: &str) -> Result<Vec3, String> {
    let values: Vec<f32> =
        text.split(',').map(str::parse).collect::<Result<_, _>>().map_err(|e| format!("{e}"))?;
    match values[..] {
        [x, y, z] => Ok(Vec3::new(x, y, z)),
        _ => Err("expected x,y,z".into()),
    }
}

impl Options {
    /// The entrance to use: the explicit one, or the new-game entrance when
    /// starting the default level without an explicit position.
    fn entrance(&self, level: &str) -> Option<&str> {
        match (&self.entrance, self.start) {
            (Some(entrance), _) => Some(entrance),
            (None, None) if level == DEFAULT_LEVEL => Some(DEFAULT_ENTRANCE),
            _ => None,
        }
    }
}

pub fn run(options: Options) -> AppExit {
    let files = match RetailFiles::open(&options.retail) {
        Ok(files) => Arc::new(files),
        Err(error) => {
            eprintln!("cannot read the retail installation: {error}");
            return AppExit::error();
        }
    };
    // Retail names are case-insensitive; use the normalized name throughout.
    let level = match normalize(&options.level) {
        Ok(level) if files.contains(&level) => level,
        _ => {
            eprintln!("{} is not in the retail installation", options.level);
            return AppExit::error();
        }
    };

    // Retail falls back to the default sky when a level's own is missing.
    let sky = LevelInfo::for_path(&level)
        .and_then(LevelInfo::sky_path)
        .map(|sky| if files.contains(&sky) { sky } else { DEFAULT_SKY.to_owned() });

    let mut start = PartyStart { eye: options.start, yaw: options.yaw.to_radians() };
    if let (None, Some(entrance)) = (options.start, options.entrance(&level)) {
        match entrance_placement(&files, &level, entrance) {
            Some(placement) => {
                info!("entering {level} at {entrance}: {placement:?}");
                start = placement
            }
            None => warn!(
                "entrance {entrance} is missing or not positioned; using the retail fallback position"
            ),
        }
    }

    let mut app = App::new();
    app.add_plugins(RetailPlugin {
        files,
        navmesh_cache: "imported_assets/navmesh".into(),
        navmesh_classes: SIZE_CLASSES.len(),
    })
    .add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window { title: "Rizzardry".into(), ..default() }),
                ..default()
            })
            // Navmeshes are baked once by the asset processor and cached.
            .set(AssetPlugin { mode: AssetMode::Processed, ..default() }),
    )
    .add_plugins((
        EnhancedInputPlugin,
        PhysicsPlugins::default(),
        WorldPlugin,
        PartyPlugin,
        PresentationPlugin,
        TimePlugin,
    ))
    .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.07)));
    app.insert_resource(start);
    if let Some(path) = options.screenshot.clone() {
        let after = options.screenshot_after.unwrap_or(2.0 + options.walk.unwrap_or(0.0));
        app.insert_resource(ScreenshotAndExit::new(path, after))
            .add_systems(Update, screenshot_and_exit);
    }
    if let Some(hour) = options.hour {
        app.insert_resource(GameClock { ms: (hour.rem_euclid(24.0) * 3_600_000.0) as u64 });
    }
    if let Some(name) = options.face_prop.clone() {
        app.insert_resource(FacePropOnEntry { name, done: false })
            .add_systems(Update, face_prop_on_entry);
    }
    if let Some(after) = options.interact_at {
        app.insert_resource(ScriptedInteract { after, elapsed: 0.0, done: false })
            .add_systems(Update, scripted_interact);
    }
    if options.nav_debug {
        app.add_plugins(bevy_landmass::debug::Landmass3dDebugPlugin::default());
    }
    if let Some(height) = options.overhead {
        app.insert_resource(OverheadView { height })
            .add_systems(PostUpdate, overhead_view.before(TransformSystems::Propagate));
    }
    if options.test_agents > 0 {
        app.insert_resource(TestAgents {
            count: options.test_agents,
            radius: options.test_agent_radius,
            spawned: false,
            since_log: 0.0,
            frames: 0,
        })
        .add_systems(Update, (spawn_test_agents, log_test_agents));
    }
    if let Some(seconds) = options.walk {
        app.insert_resource(ScriptedWalk { seconds, started: false, since_log: 0.0 })
            .add_systems(Update, scripted_walk);
    }
    app.add_systems(Startup, move |mut commands: Commands, assets: Res<AssetServer>| {
        commands.spawn((
            Name::new(level.clone()),
            Level {
                geometry: assets.load(format!("retail://{level}")),
                navmeshes: (0..SIZE_CLASSES.len())
                    .map(|class| assets.load(navmesh_path(&level, class)))
                    .collect(),
                sky: sky.as_ref().map(|sky| assets.load(format!("retail://{sky}"))),
            },
        ));
    });
    app.run()
}

/// Retail places the party's eye 1000 units above the ground under a
/// positioned entrance trigger, facing along the trigger's rotation. Ground
/// probing is not implemented: the eye starts 1000 above the trigger and the
/// party settles under gravity.
fn entrance_placement(files: &RetailFiles, level: &str, entrance: &str) -> Option<PartyStart> {
    let data = files.read(level)?.ok()?;
    let start = read_level_geometry(&data).ok()?.end;
    let sections = read_sections(&mut Bytes::new(&data[start..])).ok()?;
    let TriggerKind::Item(trigger) = &sections.trigger(entrance)?.kind else { return None };
    let placement = trigger.placement?;
    let [x, y, z] = placement.position;
    Some(PartyStart {
        eye: Some(retail_position([x, y + 1000.0, z])),
        yaw: retail_yaw(placement.angle, placement.axis),
    })
}
