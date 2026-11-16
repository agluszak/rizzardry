//! Composition root: plugins, settings and startup.

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use bevy_enhanced_input::EnhancedInputPlugin;

use crate::game::party::body::PartyStart;
use crate::game::party::roster::Recruit;
use crate::game::time::GameClock;
use crate::game::world::level::{Destination, Level};
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
    /// Skip the menus and start with a party file. Headless runs default to assets/party.ron.
    #[arg(long, value_name = "FILE.ron")]
    pub party: Option<PathBuf>,
    /// Start at this hour of the day (0-24) instead of a new game's noon.
    #[arg(long, value_name = "HOUR")]
    pub hour: Option<f32>,
    /// Run without a window or sound: gameplay only (with the dev feature,
    /// what happens is written to the log, and with --screenshot rendered
    /// offscreen). Each update is one fixed step of game time and they run
    /// as fast as the machine allows, not at the display's pace.
    #[arg(long)]
    pub headless: bool,
    /// Fight continuously (retail's setting): rounds follow each other
    /// without waiting for the player.
    #[arg(long)]
    pub continuous: bool,
    #[cfg(feature = "dev")]
    #[command(flatten)]
    pub dev: crate::dev::Options,
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
    #[cfg(feature = "dev")]
    let screenshot = options.dev.screenshot.is_some();
    #[cfg(not(feature = "dev"))]
    let screenshot = false;
    let quick_start = options.headless || options.party.is_some() || screenshot;
    let party = if quick_start {
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
    let rendered = !options.headless || screenshot;
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
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(FIXED_STEP));
    }
    app.add_plugins(crate::game::GamePlugin);
    // What the player sees and hears: with the window, or offscreen for a
    // headless screenshot. After the gameplay plugins, whose input and
    // messages it uses.
    if rendered {
        app.add_plugins((
            EnhancedInputPlugin,
            crate::retail::RetailPresentationPlugin,
            PresentationPlugin,
        ));
    }
    crate::settings::initialize(
        &mut app,
        crate::settings::SettingsFile::default(),
        options.continuous,
    );
    app.insert_resource(crate::game::session::NewGameDestination {
        path: level.clone(),
        start: start.clone(),
    });
    if let Some(hour) = options.hour {
        app.insert_resource(GameClock::at((hour.rem_euclid(24.0) * 3_600_000.0) as u64));
    }
    #[cfg(feature = "dev")]
    crate::dev::install(&mut app, &options.dev, options.headless);
    // Explicit developer/headless bootstrap: a new campaign with the party
    // file's members. Interactive play commits from the creation flow.
    if quick_start {
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
