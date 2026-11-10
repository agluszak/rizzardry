//! Developer conveniences that are not part of the game.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::Stopwatch;

use crate::game::party::body::PartyBody;
use crate::game::world::level::Level;

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

/// Ask for the automap until it has opened once.
pub fn open_automap(
    screen: Res<State<crate::presentation::screen::Screen>>,
    mut opened: Local<bool>,
    mut open: MessageWriter<crate::presentation::automap::OpenAutomap>,
) {
    *opened |= *screen.get() == crate::presentation::screen::Screen::Automap;
    if !*opened {
        open.write(crate::presentation::automap::OpenAutomap);
    }
}

/// Ask for the first member's review (their inventory and sheet) until it
/// has opened once, through the request the portrait makes.
pub(crate) fn open_inventory(
    mut opened: Local<bool>,
    roster: Single<&crate::game::party::roster::PartyRoster>,
    mut review: crate::presentation::inventory::OpenReview,
) {
    if !*opened && let Some(&member) = roster.first() {
        *opened = review.open(member);
    }
}

pub fn screenshot_and_exit(
    mut commands: Commands,
    time: Res<Time>,
    mut request: ResMut<ScreenshotAndExit>,
    party: Query<(), With<PartyBody>>,
    offscreen: Option<Res<Offscreen>>,
    dialogue: Option<Res<TalkTo>>,
    mut exit: MessageWriter<AppExit>,
) {
    if party.is_empty() || dialogue.is_some() {
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
    mut camera: Query<&mut Transform, With<crate::presentation::party_view::PartyCamera>>,
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
    playing: Query<(), With<Level>>,
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
    names: crate::presentation::names::ActorNames,
    mut combat: MessageReader<crate::game::combat::CombatNews>,
    mut items: MessageReader<crate::game::items::ItemNews>,
    mut search: MessageReader<crate::game::world::search::SearchNews>,
    mut identified: MessageReader<crate::game::world::identify::Identified>,
    mut camp: MessageReader<crate::game::camp::CampNews>,
    mut ready: MessageReader<crate::game::world::level::LevelReady>,
) {
    for news in combat.read() {
        use crate::game::combat::CombatNews;
        match news {
            CombatNews::Attack { attacker, target, how, swings, surprise } => info!(
                "combat: {:?} attacks {:?}: {how:?}, {swings} swings, {surprise:?}",
                names.name(*attacker),
                names.name(*target)
            ),
            CombatNews::Result { attacker, target, hits, damage } => info!(
                "combat: {:?} -> {:?}: {hits} hits, {damage} damage",
                names.name(*attacker),
                names.name(*target)
            ),
            CombatNews::Killed { actor } => info!("combat: {:?} died", names.name(*actor)),
            _ => info!("combat: {news:?}"),
        }
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

/// A command-line request, not a second conversation or teleport authority.
#[derive(Resource)]
pub struct TalkTo(pub String);

/// Ask to talk to the named NPC once the party is free in its level, and
/// stop with an error if the conversation is refused.
pub fn talk_to_npc(
    If(request): If<Res<TalkTo>>,
    speakers: crate::game::npcs::access::NpcActors,
    freedom: crate::game::freedom::Freedom,
    conversations: Query<(), With<crate::game::npcs::conversation::Conversation>>,
    mut requested: Local<bool>,
    mut begin: MessageWriter<crate::game::npcs::conversation::BeginConversation>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
) {
    let name = &request.0;
    if !conversations.is_empty() {
        commands.remove_resource::<TalkTo>();
        return;
    }
    if *requested {
        error!("cannot start --talk-to {name}: refused; choose a nearby --start position");
        exit.write(AppExit::error());
        commands.remove_resource::<TalkTo>();
        return;
    }
    if speakers.level().is_none() || !freedom.free() {
        return;
    }
    let Some(actor) = speakers.actor_named(name) else {
        error!("unknown NPC for --talk-to: {name}");
        exit.write(AppExit::error());
        commands.remove_resource::<TalkTo>();
        return;
    };
    if speakers.active_body(actor).is_none() {
        return;
    }
    begin.write(crate::game::npcs::conversation::BeginConversation { actor });
    *requested = true;
}
