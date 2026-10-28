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

pub fn screenshot_and_exit(
    mut commands: Commands,
    time: Res<Time>,
    mut request: ResMut<ScreenshotAndExit>,
    party: Query<(), With<PartyBody>>,
    offscreen: Option<Res<Offscreen>>,
    dialogue: Option<Res<TalkTo>>,
    question: Option<Res<AskAbout>>,
    mut exit: MessageWriter<AppExit>,
) {
    if party.is_empty() || dialogue.is_some() || question.is_some() {
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
    mut items: MessageReader<crate::game::inventory::ItemNews>,
    mut search: MessageReader<crate::game::world::search::SearchNews>,
    mut identified: MessageReader<crate::game::world::identify::Identified>,
    mut camp: MessageReader<crate::game::camp::CampNews>,
    mut ready: MessageReader<crate::game::world::level::LevelReady>,
) {
    for news in combat.read() {
        use crate::game::combat::CombatNews;
        match news {
            CombatNews::Attack { attacker, target, how, swings } => info!(
                "combat: {:?} attacks {:?}: {how:?}, {swings} swings",
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

pub fn talk_to_npc(world: &mut World) {
    use crate::content::npcs::NpcCatalog;
    use crate::game::npcs::{Npc, NpcAssets, NpcBodies, conversation};
    let Some(request) = world.get_resource::<TalkTo>() else { return };
    let name = request.0.clone();
    if world.query::<&Level>().iter(world).next().is_none() {
        return;
    }
    let assets = world.resource::<NpcAssets>();
    let Some(catalog) = world.resource::<Assets<NpcCatalog>>().get(&assets.catalog) else { return };
    let definition = catalog.0.iter().position(|npc| npc.source_name.eq_ignore_ascii_case(&name));
    let Some(definition) = definition else {
        error!("unknown NPC for --talk-to: {name}");
        world.write_message(AppExit::error());
        world.remove_resource::<TalkTo>();
        return;
    };
    let actor = world
        .query::<(Entity, &Npc, &NpcBodies)>()
        .iter(world)
        .find(|(_, npc, bodies)| npc.definition.0 as usize == definition && !bodies.is_empty())
        .map(|(entity, _, _)| entity);
    let Some(actor) = actor else { return };
    match conversation::begin(world, actor) {
        Ok(_) => {
            world.remove_resource::<TalkTo>();
        }
        Err(crate::game::npcs::trade::TradeError::Busy) => {}
        Err(error) => {
            error!("cannot start --talk-to {name}: {error:?}; choose a nearby --start position");
            world.write_message(AppExit::error());
            world.remove_resource::<TalkTo>();
        }
    }
}

/// A developer-supplied topic goes through the same request as the UI; no
/// fabricated dialogue state, payment bypass or forced quote selection.
#[derive(Resource)]
pub struct AskAbout(pub String);
pub(crate) fn ask_about(
    mut commands: Commands,
    question: Option<Res<AskAbout>>,
    mut inputs: Query<
        &mut bevy::text::EditableText,
        With<crate::presentation::conversation::TopicInput>,
    >,
    sessions: Query<(Entity, &crate::game::npcs::conversation::Conversation)>,
    mut requests: MessageWriter<crate::game::npcs::conversation::ConversationRequest>,
) {
    let Some(question) = question else { return };
    let Ok((conversation, state)) = sessions.single() else { return };
    if state.ready() {
        info!("asking NPC about {}", question.0);
        for mut input in &mut inputs {
            input.clear();
            input.queue_edit(bevy::text::TextEdit::Insert(question.0.clone().into()));
        }
        requests.write(crate::game::npcs::conversation::ConversationRequest::Topic {
            conversation,
            revision: state.revision,
            text: question.0.clone(),
        });
        commands.remove_resource::<AskAbout>();
    }
}
