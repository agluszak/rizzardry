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
use crate::content::level::LevelDefinition;
use crate::content::levels::LevelInfo;
use crate::content::monster::MonsterBody;
use crate::content::navmesh::navmesh_path;
use crate::content::species::MonsterCatalog;
use crate::content::ui::TextTable;
use crate::game::automap::MapNotes;
use crate::game::data::GameData;
use crate::game::inventory::ItemIds;
use crate::game::party::body::{PartyPlacement, PartyStart};
use crate::game::rng::GameRng;

/// The root of the level being played, including during travel preparation. A level
/// becomes one only by [`activate_level`]; until then its root is a
/// [`Destination`].
#[derive(Component, Debug, Clone)]
#[require(Transform)]
pub struct Level {
    /// The retail path of its `.pvl`, which identifies it.
    pub path: String,
    pub geometry: Handle<LevelDefinition>,
    /// One navmesh per walker size class (`navigation::bake::SIZE_CLASSES`).
    pub navmeshes: Vec<Handle<NavMesh3d>>,
    pub script: LevelScript,
    /// The messages its triggers show.
    pub messages: Option<Handle<TextTable>>,
}

impl Level {
    /// The level at a retail `.pvl` path, with its gameplay prerequisites.
    pub fn load(path: &str, assets: &AssetServer) -> Self {
        Level {
            path: path.to_owned(),
            geometry: assets.load(format!("retail://{path}")),
            navmeshes: (0..SIZE_CLASSES.len())
                .map(|class| assets.load(navmesh_path(path, class)))
                .collect(),
            script: LevelInfo::for_path(path)
                .map_or_else(LevelScript::default, |info| LevelScript::for_location(info.location)),
            messages: LevelInfo::for_path(path).map(|info| {
                assets.load(format!("retail://data/messages/{}.msg", info.folder.to_lowercase()))
            }),
        }
    }
}

/// A level as if already activated, without navmeshes or messages. Not a
/// `Default`: a `Level` exists only once [`activate_level`] says so.
#[cfg(test)]
impl Level {
    pub fn test(geometry: Handle<LevelDefinition>) -> Self {
        Level {
            path: "test".into(),
            geometry,
            navmeshes: vec![],
            script: default(),
            messages: None,
        }
    }
}

/// Belongs to a level for its lifetime: its local monsters, items, groups,
/// props, devices and navigation. This, not the transform hierarchy, is what
/// persistence, damage and pickup mean by "in the level"; despawning the
/// level despawns them. Spatial parts keep their own `ChildOf`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[relationship(relationship_target = LevelEntities)]
pub struct LevelOwned(pub Entity);

/// What a level owns, maintained by Bevy from [`LevelOwned`].
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = LevelOwned, linked_spawn)]
pub struct LevelEntities(Vec<Entity>);

impl LevelEntities {
    pub fn entities(&self) -> &[Entity] {
        &self.0
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
    world.get::<Level>(level)?;
    let pending: Vec<_> =
        world.query_filtered::<Entity, With<TravelQuestion>>().iter(world).collect();
    for question in pending {
        world.despawn(question);
    }
    Some(world.spawn((TravelQuestion { entrance }, LevelOwned(level))).id())
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
    questions: Query<(Entity, &TravelQuestion, &LevelOwned)>,
    levels: Query<(), With<Level>>,
    battle: Option<Res<crate::game::combat::Battle>>,
    mut travel: MessageWriter<Travel>,
) {
    let answers: Vec<_> = answers.read().copied().collect();
    for (entity, question, owner) in &questions {
        if !levels.contains(owner.0) {
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

/// Prepare a destination without freezing or snapshotting the source world.
/// Activation owns the departure commit; loading failure changes no live state.
pub fn travel(
    mut commands: Commands,
    mut requests: MessageReader<Travel>,
    destinations: Query<Entity, With<Destination>>,
    assets: Res<AssetServer>,
    battle: Option<Res<crate::game::combat::Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
) {
    let Some(request) = requests.read().last() else { return };
    if battle.is_some() || camp.is_some() {
        warn!("travel to {} refused: the party is occupied", request.entrance);
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
    let start = PartyStart { entrance: Some(request.entrance.clone()), ..default() };
    commands.spawn((Name::new(path.clone()), Destination::new(Level::load(&path, &assets), start)));
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
            triggers: self.devices.remember(children, state),
            prop_frames: self
                .props
                .iter_many(children)
                .matched()
                .map(|(prop, motion)| (prop.definition, motion.target))
                .collect(),
            monsters: self
                .monsters
                .iter_many(children)
                .matched()
                .map(|(monster, at, walker, group)| KeptMonster {
                    group: group.and_then(|m| self.groups.get(m.0).ok()).map(|g| g.authored),
                    monster: monster.clone(),
                    feet: super::navigation::feet(at.translation(), walker),
                    yaw: at.rotation().to_euler(EulerRot::YXZ).0,
                })
                .collect(),
            items: self
                .items
                .iter_many(children)
                .matched()
                .map(|(item, at)| (item.clone(), at.translation))
                .collect(),
            groups: self.groups.iter_many(children).matched().copied().collect(),
            notes: notes.cloned().unwrap_or_default(),
        }
    }
}

/// A level being prepared to be played: the first, or travel's
/// destination. It loads until its gameplay prerequisites exist (geometry
/// and collision, the shared databases, its monsters' models, its navmeshes
/// loaded or failed); then [`activate_level`] makes it the [`Level`]. Its
/// sky and other visuals load on their own once it is played.
#[derive(Component, Debug)]
#[require(Transform)]
pub struct Destination {
    pub level: Level,
    pub start: PartyStart,
    /// It cannot be played: an asset failed or its collision is unusable.
    pub failed: bool,
}

impl Destination {
    pub fn new(level: Level, start: PartyStart) -> Self {
        Destination { level, start, failed: false }
    }
}

/// The level being played, and its geometry.
#[derive(SystemParam)]
pub struct PlayedLevel<'w, 's> {
    levels: Query<'w, 's, (Entity, &'static Level)>,
    geometry: Res<'w, Assets<LevelDefinition>>,
}

impl PlayedLevel<'_, '_> {
    pub fn entity(&self) -> Option<Entity> {
        self.levels.single().ok().map(|(entity, _)| entity)
    }

    pub fn geometry(&self) -> Option<&LevelDefinition> {
        self.geometry.get(&self.levels.single().ok()?.1.geometry)
    }

    pub fn view_distance(&self) -> Option<f32> {
        self.geometry().map(LevelDefinition::view_distance)
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
fn level_collider(geometry: &LevelDefinition) -> Result<Collider, String> {
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
    geometry: &LevelDefinition,
    collider: Collider,
    script: super::scripts::LevelScript,
    tables: &LootTables,
    mut remembered: Option<&mut LevelRecord>,
) {
    let props: Vec<_> =
        geometry.props.iter().map(|_| commands.spawn(LevelOwned(root)).id()).collect();
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
        ));
        if let Some(doorway) = doorway {
            prop_entity.insert(doorway);
        }
        prop_entity.with_children(|prop_entity| {
            for (part_index, part) in prop.parts.iter().enumerate() {
                let at =
                    if part.keys.is_empty() { part.transform } else { pose(&part.keys, frame) };
                let mut part_entity =
                    prop_entity.spawn((PropPart { part: part_index, keys: part.keys.clone() }, at));
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
        "level built: {} props, {} collision triangles",
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
/// remembered of it), make it the [`Level`], place the party and resume its
/// body, let the departed level go, and announce it. A destination whose
/// geometry, databases or collision fail is marked failed, and the party
/// plays on where it was.
pub fn activate_level(
    mut commands: Commands,
    mut destinations: Query<(Entity, &mut Destination)>,
    current: Query<(Entity, &Level, &LevelState, Option<&LevelEntities>)>,
    battle: Option<Res<crate::game::combat::Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    geometry: Res<Assets<LevelDefinition>>,
    assets: Res<AssetServer>,
    mut shared: SharedData,
    mut memory: ResMut<LevelMemory>,
    (mut ids, mut rng): (ResMut<ItemIds>, ResMut<GameRng>),
    mut transition: ParamSet<(LevelContents, PartyPlacement)>,
    mut ready: MessageWriter<LevelReady>,
) {
    for (root, mut destination) in &mut destinations {
        let destination = &mut *destination;
        if destination.failed {
            continue;
        }
        let level = &destination.level;
        let failed = |handle: UntypedAssetId| {
            matches!(assets.get_load_state(handle), Some(LoadState::Failed(_)))
        };
        let Some(data) = shared.data.as_ref() else { continue };
        if failed(level.geometry.id().untyped())
            || level.navmeshes.iter().any(|h| failed(h.id().untyped()))
            || failed(data.items.id().untyped())
            || failed(data.loot.id().untyped())
            || failed(data.species.id().untyped())
        {
            error!("{}: an asset it needs failed to load", level.path);
            destination.failed = true;
            continue;
        }
        let (Some(geometry), Some(items), Some(loot), Some(species)) = (
            geometry.get(&level.geometry),
            shared.items.get(&data.items),
            shared.loot.get(&data.loot),
            shared.species.get(&data.species),
        ) else {
            continue;
        };
        let bodies = shared.monster_bodies.for_level(geometry, species, &assets);
        if bodies.iter().any(|body| failed(body.id().untyped())) {
            error!("{}: a monster's body definition failed to load", level.path);
            destination.failed = true;
            continue;
        }
        if !bodies.iter().all(|body| shared.bodies.contains(body))
            || !navigation::navmeshes_ready(level, &assets)
        {
            continue;
        }
        // Revalidate at commit: combat or camping may have begun while loading.
        if battle.is_some() || camp.is_some() {
            continue;
        }
        let collider = match level_collider(geometry) {
            Ok(collider) => collider,
            Err(error) => {
                error!("{}: {error}", level.path);
                destination.failed = true;
                continue;
            }
        };
        // One authority until now: capture the source's latest state exactly at
        // departure, including changes made while the destination was loading.
        for (entity, source, state, children) in &current {
            let children = children.map_or(&[][..], LevelEntities::entities);
            memory.0.insert(source.path.clone(), transition.p0().remember(entity, state, children));
            commands.entity(entity).despawn();
        }
        let mut remembered = memory.0.remove(&level.path);
        build_world(
            &mut commands,
            root,
            geometry,
            collider,
            level.script,
            loot,
            remembered.as_mut(),
        );
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
        let party = transition.p1().place(&mut commands, geometry, &destination.start);
        navigation::add_party_characters(&mut commands, root, party, &navigation);
        info!("{} is active", level.path);
        commands.entity(root).remove::<Destination>().insert((level.clone(), navigation));
        ready.write(LevelReady { level: root });
        break;
    }
}

#[cfg(test)]
mod travel_question_tests {
    use super::*;

    fn app() -> (App, Entity) {
        let mut app = App::new();
        app.add_message::<TravelAnswer>()
            .add_message::<Travel>()
            .add_systems(Update, answer_travel_questions);
        let level = app.world_mut().spawn(Level::test(default())).id();
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
        // Asking again replaces an unanswered question.
        let replaced = ask(&mut app, level, "replaced");
        let current = ask(&mut app, level, "second");
        assert!(app.world().get_entity(replaced).is_err());
        assert_eq!(app.world_mut().query::<&TravelQuestion>().iter(app.world()).count(), 1);
        app.world_mut().write_message(TravelAnswer { question: replaced, accept: true });
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
    fn departed_questions_and_answers_during_a_battle_cannot_start_travel() {
        let (mut app, level) = app();
        let question = ask(&mut app, level, "departed");
        let source = app.world_mut().entity_mut(level).take::<Level>().unwrap();
        app.world_mut().write_message(TravelAnswer { question, accept: true });
        app.update();
        assert!(app.world().get_entity(question).is_err());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
        app.world_mut().entity_mut(level).insert(source);
        app.insert_resource(crate::game::combat::Battle::default());
        let question = ask(&mut app, level, "fighting");
        app.world_mut().write_message(TravelAnswer { question, accept: true });
        app.update();
        assert!(app.world().get_entity(question).is_err());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
    }
}
