//! The active level: one root entity owning everything level-specific.
//!
//! The root carries [`Level`]; its geometry, collider and navigation are
//! built under it as their assets load. Despawning the root unloads the
//! level. The party is not level-owned and survives travel.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::NavMesh3d;

use bevy::asset::{LoadState, UntypedAssetId};
use bevy::ecs::system::SystemParam;

use super::identify::{GroupMember, MonsterGroup};
use super::items::WorldItem;
use super::navigation::Walker;
use super::navigation::bake::SIZE_CLASSES;
use super::navigation::links::DoorLink;
use super::population::{Monster, MonsterBodies};
use super::props::{Prop, PropMotion, pose};
use super::scripts::LevelScript;
use super::triggers::{DeviceRecords, LevelState, TriggerMemory, spawn_devices};
use super::{navigation, population};
use crate::content::items::{ItemCatalog, LootTables};
use crate::content::level::LevelGeometry;
use crate::content::levels::LevelInfo;
use crate::content::monster::MonsterBody;
use crate::content::navmesh::navmesh_path;
use crate::content::species::MonsterCatalog;
use crate::content::ui::TextTable;
use crate::game::automap::MapNotes;
use crate::game::data::GameData;
use crate::game::inventory::ItemIds;
use crate::game::party::body::PartyPlacement;
use crate::game::rng::GameRng;

/// The root of the level being played (or, during travel, left). A level
/// becomes one only by [`activate_level`]; until then its root is a
/// [`Destination`].
#[derive(Component, Debug, Clone)]
#[require(Transform, Visibility)]
pub struct Level {
    /// The retail path of its `.pvl`, which identifies it.
    pub path: String,
    pub geometry: Handle<LevelGeometry>,
    /// One navmesh per walker size class (`navigation::bake::SIZE_CLASSES`).
    pub navmeshes: Vec<Handle<NavMesh3d>>,
    /// The level's sky world, drawn behind it; none for levels without one.
    pub sky: Option<Handle<LevelGeometry>>,
    pub script: LevelScript,
    /// The messages its triggers show.
    pub messages: Option<Handle<TextTable>>,
}

impl Level {
    /// The level at a retail `.pvl` path, with its navmeshes and sky.
    pub fn load(path: &str, assets: &AssetServer) -> Self {
        Level {
            path: path.to_owned(),
            geometry: assets.load(format!("retail://{path}")),
            navmeshes: (0..SIZE_CLASSES.len())
                .map(|class| assets.load(navmesh_path(path, class)))
                .collect(),
            sky: LevelInfo::for_path(path)
                .and_then(LevelInfo::sky_path)
                .map(|sky| assets.load(format!("retail://{sky}"))),
            script: LevelInfo::for_path(path)
                .map_or_else(LevelScript::default, |info| LevelScript::for_location(info.location)),
            messages: LevelInfo::for_path(path).map(|info| {
                assets.load(format!("retail://data/messages/{}.msg", info.folder.to_lowercase()))
            }),
        }
    }
}

/// The party is being asked whether to leave for another level's entrance;
/// it waits until the player answers (retail `RequestLevelTransition`).
#[derive(Component, Debug, Clone)]
#[component(immutable)]
pub struct TravelQuestion {
    pub entrance: String,
}

/// Replace the one pending question. The new identity rejects answers from
/// the dismissed dialog, including two asks before presentation has run.
pub fn ask_travel(world: &mut World, level: Entity, entrance: String) -> Option<Entity> {
    if world.get::<Level>(level).is_none() {
        return None;
    }
    let pending: Vec<_> =
        world.query_filtered::<Entity, With<TravelQuestion>>().iter(world).collect();
    for question in pending {
        world.despawn(question);
    }
    Some(world.spawn((TravelQuestion { entrance }, ChildOf(level))).id())
}

/// An answer addresses the particular question that was displayed. It cannot
/// answer a replacement question or a question from a departed level.
#[derive(Message, Debug, Clone, Copy)]
pub struct TravelAnswer {
    pub question: Entity,
    pub accept: bool,
}

/// Resolve questions through gameplay, including when no UI is installed.
/// The first matching answer settles a question; later answers cannot reverse it.
pub fn answer_travel_questions(
    mut commands: Commands,
    mut answers: MessageReader<TravelAnswer>,
    questions: Query<(Entity, &TravelQuestion, &ChildOf)>,
    levels: Query<(), With<Level>>,
    battle: Option<Res<crate::game::combat::Battle>>,
    mut travel: MessageWriter<Travel>,
) {
    let answers: Vec<_> = answers.read().copied().collect();
    for (entity, question, owner) in &questions {
        if !levels.contains(owner.parent()) {
            commands.entity(entity).despawn();
            continue;
        }
        let Some(answer) = answers.iter().find(|answer| answer.question == entity) else {
            continue;
        };
        if answer.accept && battle.is_none() {
            travel.write(Travel { entrance: question.entrance.clone() });
        }
        commands.entity(entity).despawn();
    }
}

/// Leave the level for one of another level's entrances, e.g. `MO201`.
#[derive(Message, Debug, Clone)]
pub struct Travel {
    pub entrance: String,
}

/// Prepare another level without changing the live source. Only successful
/// activation captures and retires it; failed preparation has nothing to undo.
pub fn travel(
    mut commands: Commands,
    mut requests: MessageReader<Travel>,
    destinations: Query<Entity, With<Destination>>,
    assets: Res<AssetServer>,
    battle: Option<Res<crate::game::combat::Battle>>,
) {
    let Some(request) = requests.read().last() else { return };
    if battle.is_some() {
        warn!("travel to {} refused: the party is fighting", request.entrance);
        return;
    }
    let Some(info) = LevelInfo::for_entrance(&request.entrance) else {
        warn!("no level has entrance {}", request.entrance);
        return;
    };
    for entity in &destinations {
        commands.entity(entity).despawn();
    }
    let path = info.path();
    let mut destination = Destination::new(Level::load(&path, &assets));
    destination.arrival.entrance = Some(request.entrance.clone());
    commands.spawn((Name::new(path), destination));
}

/// Levels the party has left, by path, as they will be on its return.
/// Not yet saved with the game.
#[derive(Resource, Debug, Default)]
pub struct LevelMemory(pub(super) HashMap<String, LevelRecord>);

impl LevelMemory {
    /// No level is remembered.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A level the party has left (retail keeps it in the level's state image):
/// what its triggers keep, where its props were heading, its monsters (the
/// dead too, so they stay dead), the items lying about, what the party
/// knows of its groups, and its map notes. Activation consumes it.
#[derive(Debug)]
pub struct LevelRecord {
    triggers: TriggerMemory,
    /// By prop definition index.
    prop_frames: HashMap<usize, f32>,
    pub(super) monsters: Vec<KeptMonster>,
    pub(super) items: Vec<(WorldItem, Vec3)>,
    pub(super) groups: Vec<MonsterGroup>,
    notes: MapNotes,
}

/// A monster as the party left it: where its feet were and which way it
/// faced.
#[derive(Clone, Debug)]
pub struct KeptMonster {
    pub group: Option<i32>,
    pub monster: Monster,
    pub feet: Vec3,
    pub yaw: f32,
}

/// What travel reads to remember the level it leaves.
#[derive(SystemParam)]
pub struct LevelContents<'w, 's> {
    devices: DeviceRecords<'w, 's>,
    props: Query<'w, 's, (&'static Prop, &'static PropMotion)>,
    monsters: Query<
        'w,
        's,
        (&'static Monster, super::ActorPose, Option<&'static Walker>, Option<&'static GroupMember>),
    >,
    items: Query<'w, 's, (&'static WorldItem, &'static Transform)>,
    groups: Query<'w, 's, &'static MonsterGroup>,
    notes: Query<'w, 's, &'static MapNotes>,
}

impl LevelContents<'_, '_> {
    fn remember(&self, root: Entity, state: &LevelState, children: &[Entity]) -> LevelRecord {
        let notes = self.notes.get(root).ok();
        LevelRecord {
            triggers: self.devices.remember(root, state),
            prop_frames: children
                .iter()
                .filter_map(|&c| self.props.get(c).ok())
                .map(|(prop, motion)| (prop.definition, motion.target))
                .collect(),
            monsters: children
                .iter()
                .filter_map(|&c| self.monsters.get(c).ok())
                .map(|(monster, at, walker, group)| KeptMonster {
                    group: group.and_then(|m| self.groups.get(m.0).ok()).map(|g| g.authored),
                    monster: monster.clone(),
                    feet: super::navigation::feet(at.translation(), walker),
                    yaw: at.rotation().to_euler(EulerRot::YXZ).0,
                })
                .collect(),
            items: children
                .iter()
                .filter_map(|&c| self.items.get(c).ok())
                .map(|(item, at)| (item.clone(), at.translation))
                .collect(),
            groups: children.iter().filter_map(|&c| self.groups.get(c).ok().copied()).collect(),
            notes: notes.cloned().unwrap_or_default(),
        }
    }
}

/// A level being prepared to be played: the first, or travel's
/// destination. It loads until its gameplay prerequisites exist (geometry
/// and collision, the shared databases, its monsters' body definitions and loaded navmeshes); then [`activate_level`] makes it the [`Level`]. Its
/// sky and other visuals load on their own once it is played.
#[derive(Component, Debug)]
#[require(Transform, Visibility)]
pub struct Destination {
    pub level: Level,
    pub arrival: crate::game::party::body::PartyStart,
    /// It cannot be played: an asset failed or its collision is unusable.
    pub failed: bool,
}

impl Destination {
    pub fn new(level: Level) -> Self {
        Destination { level, arrival: default(), failed: false }
    }
}

/// The level being played, and its geometry.
#[derive(SystemParam)]
pub struct PlayedLevel<'w, 's> {
    levels: Query<'w, 's, (Entity, &'static Level)>,
    geometry: Res<'w, Assets<LevelGeometry>>,
}

impl PlayedLevel<'_, '_> {
    pub fn entity(&self) -> Option<Entity> {
        self.levels.single().ok().map(|(entity, _)| entity)
    }

    pub fn geometry(&self) -> Option<&LevelGeometry> {
        self.geometry.get(&self.levels.single().ok()?.1.geometry)
    }

    pub fn view_distance(&self) -> Option<f32> {
        self.geometry().map(LevelGeometry::view_distance)
    }
}

/// Run condition: a level is being played.
pub fn level_active(levels: Query<(), With<Level>>) -> bool {
    !levels.is_empty()
}

/// One rigid part of a placed prop, with its animation poses.
#[derive(Component, Debug)]
pub struct PropPart {
    /// Its place among its prop's parts (`PropModel::parts`).
    pub part: usize,
    /// Poses along the part's path, starting with the rest pose; empty if
    /// the part does not move.
    pub keys: Vec<Transform>,
}

/// A level has become active, with the party in it.
#[derive(Message, Debug, Clone, Copy)]
pub struct LevelReady {
    pub level: Entity,
}

/// The level's collider, or why it has none.
fn level_collider(geometry: &LevelGeometry) -> Result<Collider, String> {
    Collider::try_trimesh_with_config(
        geometry.collision.vertices.clone(),
        geometry.collision.triangles.clone(),
        TrimeshFlags::MERGE_DUPLICATE_VERTICES
            | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES
            | TrimeshFlags::FIX_INTERNAL_EDGES,
    )
    .map_err(|error| format!("level collider: {error:?}"))
}

/// Build the level's playable world under its root: its collider, its props
/// (moving parts and their colliders) where it left them, its trigger state
/// and map notes. What it looks like is presentation's to attach.
fn build_world(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelGeometry,
    collider: Collider,
    script: super::scripts::LevelScript,
    tables: &LootTables,
    mut remembered: Option<&mut LevelRecord>,
) {
    let props: Vec<_> = geometry.props.iter().map(|_| commands.spawn(ChildOf(root)).id()).collect();
    spawn_devices(
        commands,
        root,
        geometry,
        &props,
        script,
        tables,
        remembered.as_deref_mut().map(|r| std::mem::take(&mut r.triggers)),
    );
    commands.entity(root).insert((
        RigidBody::Static,
        collider,
        remembered.as_deref_mut().map_or_else(MapNotes::default, |r| std::mem::take(&mut r.notes)),
    ));
    for (index, prop) in geometry.props.iter().enumerate() {
        let frame =
            remembered.as_ref().and_then(|r| r.prop_frames.get(&index)).copied().unwrap_or(0.0);
        let last_frame =
            prop.parts.iter().map(|p| p.keys.len().saturating_sub(1)).max().unwrap_or(0);
        let door = prop.trigger.is_some_and(|index| geometry.triggers[index].door.is_some());
        let centre = prop.parts.iter().map(|part| part.transform.translation).sum::<Vec3>()
            / prop.parts.len().max(1) as f32;
        // A door's doorway, from its closed shape (each part at its
        // first pose), for walkers' links while it is open.
        let doorway = door
            .then(|| {
                DoorLink::around(prop.parts.iter().flat_map(|part| {
                    let closed = part.keys.first().copied().unwrap_or(part.transform);
                    part.collision.vertices.iter().map(move |&v| closed.transform_point(v))
                }))
            })
            .flatten();
        let mut prop_entity = commands.entity(props[index]);
        prop_entity.insert((
            Name::new(prop.name.clone()),
            Prop {
                definition: index,
                centre,
                door,
                frames_per_second: prop.frames_per_second,
                last_frame,
                wraps: prop.wraps,
                slots: prop.slots.clone(),
            },
            PropMotion { frame, target: frame },
            Transform::default(),
            Visibility::default(),
        ));
        if let Some(doorway) = doorway {
            prop_entity.insert(doorway);
        }
        prop_entity.with_children(|prop_entity| {
            for (part_index, part) in prop.parts.iter().enumerate() {
                let at =
                    if part.keys.is_empty() { part.transform } else { pose(&part.keys, frame) };
                let mut part_entity = prop_entity.spawn((
                    PropPart { part: part_index, keys: part.keys.clone() },
                    at,
                    Visibility::default(),
                ));
                // Sprites turn with the view and do not collide.
                if !prop.face_camera {
                    match Collider::try_trimesh_with_config(
                        part.collision.vertices.clone(),
                        part.collision.triangles.clone(),
                        TrimeshFlags::MERGE_DUPLICATE_VERTICES
                            | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES,
                    ) {
                        Ok(collider) => {
                            // Parts with a path can move (doors, lifts).
                            let body = if part.keys.is_empty() {
                                RigidBody::Static
                            } else {
                                RigidBody::Kinematic
                            };
                            part_entity.insert((body, collider));
                            // Props the party walks through can still be used.
                            if !prop.collidable {
                                part_entity.insert(Sensor);
                            }
                        }
                        Err(error) => warn!("prop {} collider: {error:?}", prop.name),
                    }
                }
            }
        });
    }
    info!(
        "level built: {} parts, {} props, {} collision triangles",
        geometry.parts.len(),
        geometry.props.len(),
        geometry.collision.triangles.len()
    );
}

/// The databases every level's gameplay needs.
#[derive(SystemParam)]
pub struct SharedData<'w> {
    data: Option<Res<'w, GameData>>,
    items: Res<'w, Assets<ItemCatalog>>,
    loot: Res<'w, Assets<LootTables>>,
    species: Res<'w, Assets<MonsterCatalog>>,
    bodies: Res<'w, Assets<MonsterBody>>,
    monster_bodies: ResMut<'w, MonsterBodies>,
    motion_configs: ResMut<'w, Assets<navigation::WalkerMotionConfig>>,
}

/// Make a destination playable once its gameplay prerequisites exist, in
/// one step: build its world, navigation and population (consuming what was
/// remembered of it), make it the [`Level`], place the party, let the outgoing level go, and announce it. A destination whose
/// geometry, databases or collision fail is marked failed, and the party
/// plays on where it was.
pub fn activate_level(
    mut commands: Commands,
    mut destinations: Query<(Entity, &mut Destination)>,
    sources: Query<(Entity, &Level, Option<&LevelState>, Option<&Children>)>,
    battle: Option<Res<crate::game::combat::Battle>>,
    mut pending: ResMut<super::triggers::PendingPartyMove>,
    geometry: Res<Assets<LevelGeometry>>,
    assets: Res<AssetServer>,
    mut shared: SharedData,
    mut memory: ResMut<LevelMemory>,
    (mut ids, mut rng): (ResMut<ItemIds>, ResMut<GameRng>),
    mut transition: ParamSet<(LevelContents, PartyPlacement)>,
    mut ready: MessageWriter<LevelReady>,
) {
    // There is one accepted destination, not a sequence to replay in one frame.
    let Ok((root, mut destination)) = destinations.single_mut() else { return };
    let destination = &mut *destination;
    if destination.failed {
        return;
    }
    // A fight may have begun while assets loaded. Do not let preparation
    // grant permission to escape it; a fresh travel request is required.
    if battle.is_some() {
        commands.entity(root).despawn();
        return;
    }
    let level = &destination.level;
    let failed = |handle: UntypedAssetId| {
        matches!(assets.get_load_state(handle), Some(LoadState::Failed(_)))
    };
    let Some(data) = shared.data.as_ref() else { return };
    if failed(level.geometry.id().untyped())
        || level.navmeshes.iter().any(|h| failed(h.id().untyped()))
        || failed(data.items.id().untyped())
        || failed(data.loot.id().untyped())
        || failed(data.species.id().untyped())
    {
        error!("{}: an asset it needs failed to load", level.path);
        destination.failed = true;
        return;
    }
    let (Some(geometry), Some(items), Some(loot), Some(species)) = (
        geometry.get(&level.geometry),
        shared.items.get(&data.items),
        shared.loot.get(&data.loot),
        shared.species.get(&data.species),
    ) else {
        return;
    };
    let bodies = shared.monster_bodies.for_level(geometry, species, &assets);
    if bodies.iter().any(|body| failed(body.id().untyped())) {
        error!("{}: a monster's body definition failed to load", level.path);
        destination.failed = true;
        return;
    }
    if !bodies.iter().all(|body| shared.bodies.contains(body))
        || !navigation::navmeshes_ready(level, &assets)
    {
        return;
    }
    let collider = match level_collider(geometry) {
        Ok(collider) => collider,
        Err(error) => {
            error!("{}: {error}", level.path);
            destination.failed = true;
            return;
        }
    };
    for (entity, source, state, children) in &sources {
        if let Some(state) = state {
            let children = children.map_or(&[][..], |c| &c[..]);
            memory.0.insert(source.path.clone(), transition.p0().remember(entity, state, children));
        }
    }
    // Capture first, including same-level travel, then consume the record.
    let mut remembered = memory.0.remove(&level.path);
    build_world(&mut commands, root, geometry, collider, level.script, loot, remembered.as_mut());
    let navigation = navigation::install(&mut commands, root, level, geometry);
    population::populate(
        &mut commands,
        root,
        geometry,
        remembered,
        (species, items),
        (&shared.monster_bodies.0, &shared.bodies),
        (&navigation, &mut shared.motion_configs),
        &mut ids,
        &mut rng,
    );
    pending.0 = None;
    transition.p1().place(&mut commands, geometry, &destination.arrival);
    for (left, ..) in &sources {
        commands.entity(left).despawn();
    }
    info!("{} is active", level.path);
    commands.entity(root).remove::<Destination>().insert((level.clone(), navigation));
    ready.write(LevelReady { level: root });
}

#[cfg(test)]
mod travel_question_tests {
    use super::*;

    fn app() -> (App, Entity) {
        let mut app = App::new();
        app.add_message::<TravelAnswer>()
            .add_message::<Travel>()
            .add_systems(Update, answer_travel_questions);
        let level = app
            .world_mut()
            .spawn(Level {
                path: "question-test".into(),
                geometry: default(),
                navmeshes: vec![],
                sky: None,
                script: default(),
                messages: None,
            })
            .id();
        (app, level)
    }

    fn ask(app: &mut App, level: Entity, entrance: &str) -> Entity {
        ask_travel(app.world_mut(), level, entrance.into()).unwrap()
    }

    #[test]
    fn questions_complete_headlessly_and_answers_cannot_retarget_a_new_question() {
        let (mut app, level) = app();
        let old = ask(&mut app, level, "first");
        app.world_mut().write_message(TravelAnswer { question: old, accept: false });
        app.update();
        assert!(app.world().get_entity(old).is_err());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
        let current = ask(&mut app, level, "second");
        app.world_mut().write_message(TravelAnswer { question: old, accept: true });
        app.update();
        assert!(app.world().get_entity(current).is_ok());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
        app.world_mut().write_message(TravelAnswer { question: current, accept: true });
        app.world_mut().write_message(TravelAnswer { question: current, accept: false });
        app.update();
        assert!(app.world().get_entity(current).is_err());
        let travel: Vec<_> = app.world_mut().resource_mut::<Messages<Travel>>().drain().collect();
        assert_eq!(travel.len(), 1);
        assert_eq!(travel[0].entrance, "second");
    }

    #[test]
    fn asking_twice_replaces_the_question_before_presentation() {
        let (mut app, level) = app();
        let old = ask(&mut app, level, "first");
        let current = ask(&mut app, level, "second");
        assert!(app.world().get_entity(old).is_err());
        assert_eq!(app.world_mut().query::<&TravelQuestion>().iter(app.world()).count(), 1);
        app.world_mut().write_message(TravelAnswer { question: old, accept: true });
        app.update();
        assert!(app.world().get_entity(current).is_ok());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
        app.world_mut().write_message(TravelAnswer { question: current, accept: true });
        app.update();
        let travel: Vec<_> = app.world_mut().resource_mut::<Messages<Travel>>().drain().collect();
        assert_eq!(travel.len(), 1);
        assert_eq!(travel[0].entrance, "second");
    }

    #[test]
    fn departed_questions_and_answers_during_a_battle_cannot_start_travel() {
        let (mut app, level) = app();
        let question = ask(&mut app, level, "departed");
        let live = app.world_mut().entity_mut(level).take::<Level>().unwrap();
        app.world_mut().write_message(TravelAnswer { question, accept: true });
        app.update();
        assert!(app.world().get_entity(question).is_err());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
        app.world_mut().entity_mut(level).insert(live);
        app.insert_resource(crate::game::combat::Battle::default());
        let question = ask(&mut app, level, "fighting");
        app.world_mut().write_message(TravelAnswer { question, accept: true });
        app.update();
        assert!(app.world().get_entity(question).is_err());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
    }
}
