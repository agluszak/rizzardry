//! Developer conveniences that are not part of the game.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::Stopwatch;

use crate::game::party::body::PartyBody;
use crate::game::world::level::{Departing, Level};

/// Capture the window `after` seconds once the party is in the world, then
/// exit. Used to check rendering from the command line.
#[derive(Resource, Debug, Clone)]
pub struct ScreenshotAndExit {
    pub path: PathBuf,
    /// Until the screenshot, then until it has been saved.
    pub wait: Timer,
    pub taken: bool,
}

impl ScreenshotAndExit {
    pub fn new(path: PathBuf, after: f32) -> Self {
        ScreenshotAndExit { path, wait: Timer::from_seconds(after, TimerMode::Once), taken: false }
    }
}

pub fn screenshot_and_exit(
    mut commands: Commands,
    time: Res<Time>,
    mut request: ResMut<ScreenshotAndExit>,
    party: Query<(), With<PartyBody>>,
    offscreen: Option<Res<Offscreen>>,
    mut exit: MessageWriter<AppExit>,
) {
    if party.is_empty() {
        return;
    }
    if !request.wait.tick(time.delta()).is_finished() {
        return;
    }
    if !request.taken {
        request.taken = true;
        request.wait = Timer::from_seconds(0.5, TimerMode::Once);
        let screenshot = match offscreen {
            Some(offscreen) => Screenshot::image(offscreen.0.clone()),
            None => Screenshot::primary_window(),
        };
        commands.spawn(screenshot).observe(save_to_disk(request.path.clone()));
    } else {
        exit.write(AppExit::Success);
    }
}

/// Rendering without a window: every camera draws into this image, which
/// screenshots capture. Twice retail's 640×480.
#[derive(Resource, Debug)]
pub struct Offscreen(pub Handle<Image>);

pub fn create_offscreen(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = Image::new_target_texture(
        1280,
        960,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    commands.insert_resource(Offscreen(images.add(image)));
}

/// Point each new camera at the offscreen image instead of the window.
pub fn draw_offscreen(
    offscreen: Res<Offscreen>,
    mut cameras: Query<&mut bevy::camera::RenderTarget, Added<Camera>>,
) {
    for mut target in &mut cameras {
        *target = offscreen.0.clone().into();
    }
}

/// Look straight down at the party from `height` metres above it.
#[derive(Resource, Debug, Clone, Copy)]
pub struct OverheadView {
    pub height: f32,
}

pub fn overhead_view(
    view: Res<OverheadView>,
    mut camera: Query<&mut Transform, With<crate::game::party::body::PartyCamera>>,
) {
    if let Ok(mut camera) = camera.single_mut() {
        camera.translation = Vec3::Y * view.height;
        camera.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    }
}

/// Speak a voice line once the party is in the world, and log when it ends.
#[derive(Resource, Debug, Clone)]
pub struct SayLine {
    pub line: String,
    /// The speech playing, and for how long.
    pub started: Option<(Entity, Stopwatch)>,
}

pub fn say_line(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    mut say: ResMut<SayLine>,
    party: Query<(), With<PartyBody>>,
    mut finished: MessageReader<crate::presentation::voice::SpeechFinished>,
) {
    if say.started.is_none() && !party.is_empty() {
        let speech = crate::presentation::voice::speak(&mut commands, &assets, &say.line);
        say.started = Some((speech, Stopwatch::new()));
        info!("speaking {}", say.line);
    }
    let Some((speech, played)) = &mut say.started else { return };
    played.tick(time.delta());
    for done in finished.read() {
        if *speech == done.speech {
            info!("finished {} after {:.2} s", done.line, played.elapsed_secs());
        }
    }
}

/// Stop the game after this many seconds of play (`--run-for`): game time
/// while a level is being played, not loading.
#[derive(Resource, Debug)]
pub struct RunFor(pub Timer);

pub fn exit_after_run(
    time: Res<Time<Virtual>>,
    mut run_for: ResMut<RunFor>,
    playing: Query<(), (With<Level>, Without<Departing>)>,
    mut exit: MessageWriter<AppExit>,
) {
    if !playing.is_empty() {
        run_for.0.tick(time.delta());
    }
    if run_for.0.is_finished() {
        exit.write(AppExit::Success);
    }
}

/// Headless, what happens goes to the log instead of the screen.
pub fn log_news(
    mut combat: MessageReader<crate::game::combat::CombatNews>,
    mut items: MessageReader<crate::game::inventory::ItemNews>,
    mut search: MessageReader<crate::game::world::search::SearchNews>,
    mut identified: MessageReader<crate::game::world::identify::Identified>,
    mut camp: MessageReader<crate::game::camp::CampNews>,
    mut ready: MessageReader<crate::game::world::level::LevelReady>,
) {
    for news in combat.read() {
        info!("combat: {news:?}");
    }
    for news in items.read() {
        info!("items: {news:?}");
    }
    for news in search.read() {
        info!("search: {news:?}");
    }
    for news in identified.read() {
        info!("identified: {news:?}");
    }
    for news in camp.read() {
        info!("camp: {news:?}");
    }
    for news in ready.read() {
        info!("level ready: {news:?}");
    }
}
