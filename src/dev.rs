//! Developer conveniences that are not part of the game.

use std::path::PathBuf;
use std::time::Duration;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_enhanced_input::prelude::*;
use bevy_landmass::PointSampleDistance3d;
use bevy_landmass::prelude::*;
use bevy_tnua::builtins::TnuaBuiltinWalkConfig;
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;

use crate::game::party::body::{Interact, PartyBody, Walk};
use crate::game::world::navigation::{LevelNavigation, Walker, WalkerMotion, WalkerMotionConfig};
use crate::presentation::monsters::MonsterAppearance;

/// Capture the window `after` seconds once the party is in the world, then
/// exit. Used to check rendering from the command line.
#[derive(Resource, Debug, Clone)]
pub struct ScreenshotAndExit {
    pub path: PathBuf,
    pub after: f32,
    pub elapsed: f32,
    pub taken: bool,
}

impl ScreenshotAndExit {
    pub fn new(path: PathBuf, after: f32) -> Self {
        ScreenshotAndExit { path, after, elapsed: 0.0, taken: false }
    }
}

pub fn screenshot_and_exit(
    mut commands: Commands,
    time: Res<Time>,
    mut request: ResMut<ScreenshotAndExit>,
    party: Query<(), With<PartyBody>>,
    mut exit: MessageWriter<AppExit>,
) {
    if party.is_empty() {
        return;
    }
    request.elapsed += time.delta_secs();
    if !request.taken && request.elapsed >= request.after {
        request.taken = true;
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(request.path.clone()));
    } else if request.taken && request.elapsed >= request.after + 0.5 {
        exit.write(AppExit::Success);
    }
}

/// Hold "walk forward" for a number of seconds after the party appears and log
/// where it goes: a scripted check of the movement controller.
#[derive(Resource, Debug, Clone)]
pub struct ScriptedWalk {
    pub seconds: f32,
    pub started: bool,
    pub since_log: f32,
}

pub fn scripted_walk(
    mut commands: Commands,
    time: Res<Time>,
    mut script: ResMut<ScriptedWalk>,
    walk: Single<Entity, With<Action<Walk>>>,
    party: Query<&Transform, With<PartyBody>>,
) {
    let Ok(transform) = party.single() else { return };
    if !script.started {
        script.started = true;
        commands.entity(*walk).insert(ActionMock::new(
            TriggerState::Fired,
            Vec2::Y,
            Duration::from_secs_f32(script.seconds),
        ));
    }
    script.since_log += time.delta_secs();
    if script.since_log >= 0.5 {
        script.since_log = 0.0;
        info!("party at {:.2}", transform.translation);
    }
}

/// Spawn stand-in monsters that chase the party across the navmesh: an
/// end-to-end check of Rerecast, Landmass, Tnua and Avian together.
#[derive(Resource, Debug, Clone)]
pub struct TestAgents {
    pub count: usize,
    /// A retail monster species for them to look like, e.g. `savant_trooper`.
    pub monster: Option<String>,
    /// Body radius in metres; picks the navmesh size class.
    pub radius: f32,
    pub spawned: bool,
    pub since_log: f32,
    pub frames: u32,
}

const TEST_AGENT_HALF_LENGTH: f32 = 0.25;
const TEST_AGENT_SPEED: f32 = 3.0;

#[derive(Component)]
pub struct TestAgent;

pub fn spawn_test_agents(
    mut commands: Commands,
    mut test: ResMut<TestAgents>,
    navigation: Query<&LevelNavigation>,
    archipelagos: Query<&Archipelago3d>,
    party: Query<(Entity, &Transform), With<PartyBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut configs: ResMut<Assets<WalkerMotionConfig>>,
    assets: Res<AssetServer>,
) {
    if test.spawned {
        return;
    }
    let (Ok(navigation), Ok((party, party_transform))) = (navigation.single(), party.single())
    else {
        return;
    };
    let radius = test.radius;
    // Keeps the capsule's bottom at the step height.
    let float_height = crate::game::world::STEP_HEIGHT + radius + TEST_AGENT_HALF_LENGTH;
    let Ok(archipelago) = archipelagos.get(navigation.archipelago_for(radius)) else {
        return;
    };
    // Probe rings around the party; keep the points that land on the navmesh.
    let centre = party_transform.translation;
    let wide = PointSampleDistance3d {
        horizontal_distance: 3.0,
        distance_above: 5.0,
        distance_below: 10.0,
        vertical_preference_ratio: 1.0,
        animation_link_max_vertical_distance: 1.0,
    };
    let mut points = Vec::new();
    for ring in [12.0, 20.0, 30.0, 45.0] {
        for step in 0..16 {
            let angle = step as f32 / 16.0 * std::f32::consts::TAU;
            let probe = centre + Vec3::new(angle.cos(), 0.0, angle.sin()) * ring;
            if let Ok(sampled) = archipelago.sample_point(probe, &wide) {
                points.push(sampled.point());
            }
        }
    }
    if points.is_empty() {
        // The island may not be registered with the archipelago yet.
        return;
    }
    test.spawned = true;
    let mesh = meshes.add(Capsule3d::new(radius, 2.0 * TEST_AGENT_HALF_LENGTH));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.2, 0.3),
        unlit: true,
        ..default()
    });
    let config = configs.add(WalkerMotionConfig {
        basis: TnuaBuiltinWalkConfig {
            speed: TEST_AGENT_SPEED,
            float_height,
            max_slope: 50f32.to_radians(),
            ..default()
        },
    });
    let model =
        test.monster.as_ref().map(|name| assets.load(format!("retail://data/monsters/{name}.mls")));
    let stride = (points.len() / test.count.max(1)).max(1);
    for &feet in points.iter().step_by(stride).take(test.count) {
        let mut agent = commands.spawn((
            Name::new("Test agent"),
            TestAgent,
            Transform::from_translation(feet + Vec3::Y * float_height),
            RigidBody::Dynamic,
            Collider::capsule(radius, 2.0 * TEST_AGENT_HALF_LENGTH),
            LockedAxes::ROTATION_LOCKED,
            TnuaController::<WalkerMotion>::default(),
            TnuaConfig::<WalkerMotion>(config.clone()),
            TnuaAvian3dSensorShape(Collider::cylinder(radius * 0.9, 0.0)),
            Walker { speed: TEST_AGENT_SPEED, float_height },
            Agent3dBundle {
                agent: default(),
                settings: AgentSettings {
                    radius,
                    desired_speed: TEST_AGENT_SPEED * 0.8,
                    max_speed: TEST_AGENT_SPEED,
                },
                archipelago_ref: ArchipelagoRef3d::new(navigation.archipelago_for(radius)),
            },
            AgentTarget3d::Entity(party),
            Velocity3d::default(),
        ));
        match &model {
            Some(model) => {
                agent.insert(MonsterAppearance { model: model.clone(), float_height });
            }
            None => {
                agent.insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
            }
        }
    }
    info!(
        "spawned {} test agents from {} navmesh points",
        test.count.min(points.len()),
        points.len()
    );
}

pub fn log_test_agents(
    time: Res<Time>,
    mut test: ResMut<TestAgents>,
    party: Query<&Transform, With<PartyBody>>,
    agents: Query<(&Transform, Option<&AgentState>), With<TestAgent>>,
) {
    test.since_log += time.delta_secs();
    test.frames += 1;
    if test.since_log < 1.0 || agents.is_empty() {
        return;
    }
    let fps = test.frames as f32 / test.since_log;
    (test.since_log, test.frames) = (0.0, 0);
    let Ok(party) = party.single() else { return };
    let mut distances: Vec<f32> =
        agents.iter().map(|(t, _)| t.translation.distance(party.translation)).collect();
    distances.sort_by(f32::total_cmp);
    let mut states: std::collections::BTreeMap<String, usize> = default();
    for (_, state) in &agents {
        *states.entry(state.map_or("none".into(), |s| format!("{s:?}"))).or_default() += 1;
    }
    let arrived = distances.iter().filter(|&&d| d < 2.0).count();
    info!(
        "{fps:.0} fps; {arrived}/{} agents within 2 m; median {:.1} m; states {states:?}",
        distances.len(),
        distances[distances.len() / 2]
    );
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

/// Press "interact" once, this many seconds after the party appears.
#[derive(Resource, Debug, Clone)]
pub struct ScriptedInteract {
    pub after: f32,
    pub elapsed: f32,
    pub done: bool,
}

pub fn scripted_interact(
    mut commands: Commands,
    time: Res<Time>,
    mut script: ResMut<ScriptedInteract>,
    interact: Single<Entity, With<Action<Interact>>>,
    party: Query<(), With<PartyBody>>,
) {
    if script.done || party.is_empty() {
        return;
    }
    script.elapsed += time.delta_secs();
    if script.elapsed >= script.after {
        script.done = true;
        commands.entity(*interact).insert(ActionMock::once(TriggerState::Fired, true));
    }
}

/// Put the party 2 m in front of a named prop, facing it, once its collider
/// exists. Used to test props without walking to them.
#[derive(Resource, Debug, Clone)]
pub struct FacePropOnEntry {
    pub name: String,
    pub done: bool,
}

pub fn face_prop_on_entry(
    mut request: ResMut<FacePropOnEntry>,
    props: Query<(&Name, &Children), With<crate::game::world::props::Prop>>,
    bounds: Query<&ColliderAabb>,
    mut party: Query<
        (&mut PartyBody, &mut Position, &mut Transform),
        Without<crate::game::world::props::Prop>,
    >,
) {
    if request.done {
        return;
    }
    let Ok((mut body, mut position, mut transform)) = party.single_mut() else { return };
    let Some((_, children)) =
        props.iter().find(|(n, _)| n.as_str().eq_ignore_ascii_case(&request.name))
    else {
        return;
    };
    let Some(aabb) = children.iter().find_map(|c| bounds.get(c).ok()) else { return };
    if aabb.min.x > aabb.max.x {
        return; // not computed yet
    }
    request.done = true;
    let (min, max) = (aabb.min, aabb.max);
    let centre = (min + max) / 2.0;
    let size = max - min;
    // Stand off the prop across its thinner horizontal axis.
    let across = if size.x < size.z { Vec3::X } else { Vec3::Z };
    let eye = (centre + across * 2.0).with_y(min.y + crate::game::party::body::EYE_HEIGHT);
    // Teleport: Avian takes a changed Transform as the new position.
    position.0 = eye - Vec3::Y * crate::game::party::body::EYE_ABOVE_CENTRE;
    transform.translation = position.0;
    let forward = -across;
    body.yaw = (-forward.x).atan2(-forward.z);
    info!("placed the party at {eye:.2} facing prop {} ({min:.2}..{max:.2})", request.name);
}

/// Speak a voice line once the party is in the world, and log when it ends.
#[derive(Resource, Debug, Clone)]
pub struct SayLine {
    pub line: String,
    pub started: Option<(Entity, f32)>,
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
        say.started = Some((speech, time.elapsed_secs()));
        info!("speaking {}", say.line);
    }
    for done in finished.read() {
        if let Some((speech, started)) = say.started
            && speech == done.speech
        {
            info!("finished {} after {:.2} s", done.line, time.elapsed_secs() - started);
        }
    }
}
