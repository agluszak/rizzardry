//! Composition root: plugins, settings and startup.

use std::path::PathBuf;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_enhanced_input::EnhancedInputPlugin;

use crate::dev::{
    OverheadView, SayLine, ScreenshotAndExit, overhead_view, say_line, screenshot_and_exit,
};
use crate::game::party::PartyPlugin;
use crate::game::party::body::PartyStart;
use crate::game::party::roster::Recruit;
use crate::game::time::{GameClock, TimePlugin};
use crate::game::world::WorldPlugin;
use crate::game::world::level::{Destination, Level};
use crate::game::world::navigation::asset::NavmeshLoader;
use crate::game::world::navigation::bake::SIZE_CLASSES;
use crate::presentation::PresentationPlugin;
use crate::retail::RetailPlugin;
use crate::retail::files::RetailFiles;
use crate::retail::path::normalize;

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
    /// Initial yaw in degrees, overriding the entrance's.
    #[arg(long, allow_negative_numbers = true)]
    pub yaw: Option<f32>,
    /// Initial pitch in degrees, up positive; retail allows ±45.
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    pub pitch: f32,
    /// Developer: capture the view shortly after entering, then exit.
    #[arg(long, value_name = "FILE.png")]
    pub screenshot: Option<PathBuf>,
    /// Developer: seconds after entering to take the screenshot (default 2).
    #[arg(long, value_name = "SECONDS")]
    pub screenshot_after: Option<f32>,
    /// Skip the menus and start with a party file. Headless runs default to assets/party.ron.
    #[arg(long, value_name = "FILE.ron")]
    pub party: Option<PathBuf>,
    /// Skip the menus and continue a saved game.
    #[arg(long, value_name = "SAVE.ron")]
    pub load: Option<PathBuf>,
    /// Start at this hour of the day (0-24) instead of a new game's noon.
    #[arg(long, value_name = "HOUR")]
    pub hour: Option<f32>,
    /// Run without a window or sound: gameplay only, with what happens
    /// written to the log; with --screenshot, also rendered offscreen. Each
    /// update is one fixed step of game time and they run as fast as the
    /// machine allows, not at the display's pace.
    #[arg(long)]
    pub headless: bool,
    /// Stop after this many seconds of play (game time while a level is
    /// played, not loading).
    #[arg(long, value_name = "SECONDS")]
    pub run_for: Option<f32>,
    /// Fight continuously (retail's setting): rounds follow each other
    /// without waiting for the player.
    #[arg(long)]
    pub continuous: bool,
    /// Developer: seed gameplay's random numbers, for another reproducible
    /// run.
    #[arg(long, value_name = "SEED")]
    pub seed: Option<u64>,
    /// Developer: speak this retail voice line on entry, e.g.
    /// `npc_aletheides_000`.
    #[arg(long, value_name = "LINE")]
    pub say: Option<String>,
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

/// The fixed step: Bevy's default 64 Hz, which movement and physics run on.
const FIXED_STEP: std::time::Duration = std::time::Duration::from_micros(15_625);

fn cpu_mesh_preprocessing(
    mut support: ResMut<bevy::render::batching::gpu_preprocessing::GpuPreprocessingSupport>,
) {
    support.max_supported_mode =
        bevy::render::batching::gpu_preprocessing::GpuPreprocessingMode::None;
}

pub fn run(options: Options) -> AppExit {
    match build(&options) {
        Ok(mut app) => app.run(),
        Err(error) => {
            eprintln!("{error}");
            AppExit::error()
        }
    }
}

/// The game, composed from its options: the window and presentation, or
/// headless; a new game entering the chosen level. Scenario tests build it
/// the same way.
pub fn build(options: &Options) -> Result<App, String> {
    let files = RetailFiles::open(&options.retail)
        .map(Arc::new)
        .map_err(|error| format!("cannot read the retail installation: {error}"))?;
    // Retail names are case-insensitive; use the normalized name throughout.
    let level = match normalize(&options.level) {
        Ok(level) if files.contains(&level) => level,
        _ => return Err(format!("{} is not in the retail installation", options.level)),
    };
    let quick_start = options.headless
        || options.party.is_some()
        || options.screenshot.is_some()
        || options.load.is_some();
    let new_game = quick_start && options.load.is_none();
    let party = if new_game {
        let path = options.party.clone().unwrap_or_else(|| "assets/party.ron".into());
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read the party {}: {error}", path.display()))?;
        ron::from_str::<Vec<Recruit>>(&text).map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };
    let start = PartyStart {
        entrance: options.entrance(&level).map(str::to_owned),
        eye: options.start,
        yaw: options.yaw.map(f32::to_radians),
        pitch: options.pitch.to_radians(),
    };

    let mut app = App::new();
    app.add_plugins(RetailPlugin { files, navmesh_classes: SIZE_CLASSES.len() });
    let asset_plugin = AssetPlugin::default();
    let rendered = !options.headless || options.screenshot.is_some();
    if rendered {
        // Bevy #25966: GPU instance-buffer construction can retain a missing
        // morph descriptor after a hidden cycle becomes visible. CPU instance
        // construction resolves the current descriptor; deformation stays on GPU.
        let plugins = DefaultPlugins
            .set(asset_plugin)
            .set(bevy::pbr::PbrPlugin { use_gpu_instance_buffer_builder: false, ..default() });
        if options.headless {
            app.add_plugins(
                plugins
                    .set(WindowPlugin {
                        primary_window: None,
                        exit_condition: bevy::window::ExitCondition::DontExit,
                        ..default()
                    })
                    .disable::<bevy::winit::WinitPlugin>()
                    .set(bevy::audio::AudioPlugin {
                        global_volume: bevy::audio::GlobalVolume::new(bevy::audio::Volume::SILENT),
                        ..default()
                    }),
            )
            .add_systems(Startup, crate::dev::create_offscreen)
            .add_systems(
                First,
                crate::dev::draw_offscreen.run_if(resource_exists::<crate::dev::Offscreen>),
            );
        } else {
            app.add_plugins(plugins.set(WindowPlugin {
                primary_window: Some(Window { title: "Rizzardry".into(), ..default() }),
                ..default()
            }));
        }
        use bevy::render::{
            RenderApp, RenderStartup, batching::gpu_preprocessing::GpuPreprocessingSupport,
            init_gpu_resource,
        };
        // Keep phase batching consistent with CPU instance construction, also
        // after device recovery. Otherwise opaque meshes enter undrawn GPU bins.
        app.sub_app_mut(RenderApp).add_systems(
            RenderStartup,
            cpu_mesh_preprocessing.after(init_gpu_resource::<GpuPreprocessingSupport>),
        );
        app.insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.07)));
    } else {
        // Gameplay has no renderer, window, image/mesh assets, UI or audio.
        app.add_plugins((
            MinimalPlugins,
            bevy::log::LogPlugin::default(),
            asset_plugin,
            TransformPlugin,
            bevy::state::app::StatesPlugin,
        ));
    }
    if options.headless {
        if rendered {
            app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::ZERO));
        }
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(FIXED_STEP))
            .add_systems(Update, crate::dev::log_news);
    }
    // Shared campaign services are composed here, not owned by world loading.
    app.init_resource::<crate::game::rng::GameRng>()
        .init_resource::<crate::game::inventory::ItemIds>()
        .init_resource::<crate::game::factions::Factions>()
        .init_resource::<crate::game::difficulty::Difficulty>()
        .init_state::<crate::game::screen::Screen>()
        .add_message::<crate::game::inventory::ItemNews>()
        .add_systems(Startup, crate::game::data::load_game_data)
        .configure_sets(
            Update,
            (
                crate::game::GameplaySet::Equipment,
                crate::game::GameplaySet::LevelActivation,
                crate::game::GameplaySet::Combat,
            )
                .chain(),
        );
    app.add_plugins((
        PhysicsPlugins::default(),
        WorldPlugin,
        PartyPlugin,
        TimePlugin,
        crate::game::combat::CombatPlugin,
        crate::game::save::SavePlugin,
        crate::game::magic::MagicPlugin,
    ));
    // Navmeshes: retail decodes a level's collision, navigation bakes it on
    // request and caches the result.
    app.register_asset_loader(NavmeshLoader {
        cache: bevy::asset::io::file::FileAssetReader::get_base_path()
            .join(crate::game::world::navigation::asset::CACHE),
        decode: crate::retail::navmesh::decode_input,
    });
    // What the player sees and hears: with the window, or offscreen for a
    // headless screenshot. After the gameplay plugins, whose input and
    // messages it uses.
    if !options.headless || options.screenshot.is_some() {
        app.add_plugins((
            EnhancedInputPlugin,
            crate::retail::RetailPresentationPlugin,
            PresentationPlugin,
        ));
    }
    if options.continuous {
        app.insert_resource(crate::game::combat::CombatSettings { continuous: true });
    }
    if let Some(seconds) = options.run_for {
        app.insert_resource(crate::dev::RunFor(Timer::from_seconds(seconds, TimerMode::Once)))
            .add_systems(Update, crate::dev::exit_after_run);
    }
    app.insert_state(if quick_start {
        crate::game::session::Session::Playing
    } else {
        crate::game::session::Session::MainMenu
    });
    app.insert_resource(crate::game::session::NewGameDestination {
        path: level.clone(),
        start: start.clone(),
    });
    if let Some(path) = options.screenshot.clone() {
        let after = options.screenshot_after.unwrap_or(2.0);
        app.insert_resource(ScreenshotAndExit::new(path, after))
            .add_systems(Update, screenshot_and_exit);
    }
    if let Some(line) = options.say.clone() {
        app.insert_resource(SayLine { line, started: None }).add_systems(Update, say_line);
    }
    if let Some(seed) = options.seed {
        app.insert_resource(crate::game::rng::GameRng::seeded(seed));
    }
    if let Some(hour) = options.hour {
        app.insert_resource(GameClock::at((hour.rem_euclid(24.0) * 3_600_000.0) as u64));
    }
    if options.nav_debug {
        app.add_plugins(bevy_landmass::debug::Landmass3dDebugPlugin::default());
    }
    if let Some(height) = options.overhead {
        app.insert_resource(OverheadView { height }).add_systems(
            PostUpdate,
            overhead_view
                .before(crate::presentation::sky::follow_with_sky_camera)
                .before(TransformSystems::Propagate),
        );
    }
    // Explicit developer/headless bootstrap: a saved game, or a new campaign
    // with the party file's members. Interactive play commits from the
    // creation flow.
    if let Some(path) = options.load.clone() {
        app.add_systems(Startup, move |mut loads: MessageWriter<crate::game::save::LoadGame>| {
            loads.write(crate::game::save::LoadGame(path.clone()));
        });
    } else if new_game {
        app.add_systems(Startup, move |mut commands: Commands, assets: Res<AssetServer>| {
            let characters: Vec<_> = party.iter().map(Recruit::character).collect();
            let first = Destination::new(
                Level::load(&level, &assets),
                start.clone(),
                crate::game::world::level::Entry::NewGame(
                    characters.into_iter().map(|c| (c, default())).collect(),
                ),
            );
            commands.spawn((Name::new(level.clone()), first));
        });
    }
    Ok(app)
}
