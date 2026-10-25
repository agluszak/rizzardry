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

use super::identify::MonsterGroups;
use super::items::WorldItem;
use super::navigation::asset::navmesh_path;
use super::navigation::bake::SIZE_CLASSES;
use super::population::Monster;
use super::props::{Prop, PropMotion, pose};
use super::scripts::LevelScript;
use super::triggers::{LevelState, TriggerMemory};
use super::{navigation, population};
use crate::content::items::{ItemCatalog, LootTables};
use crate::content::level::LevelGeometry;
use crate::content::levels::LevelInfo;
use crate::content::species::MonsterCatalog;
use crate::content::ui::TextTable;
use crate::game::automap::MapNotes;
use crate::game::data::GameData;
use crate::game::inventory::ItemIds;
use crate::game::party::body::PartyPlacement;
use crate::game::rng::GameRng;
use crate::presentation::monsters::MonsterAppearance;

/// The root of the level the party is in.
#[derive(Component, Debug)]
#[require(Transform, Visibility, LevelPhase)]
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
#[derive(Resource, Debug, Clone)]
pub struct TravelQuestion {
    pub entrance: String,
}

/// Leave the level for one of another level's entrances, e.g. `MO201`.
#[derive(Message, Debug, Clone)]
pub struct Travel {
    pub entrance: String,
}

/// Leave for another level's entrance (retail `TravelToLevel`): what the
/// level holds is remembered, it is despawned, and the destination loads
/// while the party's body waits, suspended, to be placed at the entrance on
/// activation. Work that cannot cross levels ends: a camera-path flight and
/// any pending party move. A fight cannot be left this way; travel asked
/// for during one is refused (retail's rule here is not established).
/// Retail first asks "Enter <level>?" when the trigger says so.
#[allow(clippy::too_many_arguments)]
pub fn travel(
    mut commands: Commands,
    mut requests: MessageReader<Travel>,
    levels: Query<(Entity, &Level, &LevelPhase, Option<&LevelState>, Option<&Children>)>,
    contents: LevelContents,
    mut memory: ResMut<LevelMemory>,
    assets: Res<AssetServer>,
    (battle, mut start, mut pending, party): (
        Option<Res<crate::game::combat::Battle>>,
        ResMut<crate::game::party::body::PartyStart>,
        ResMut<super::triggers::PendingPartyMove>,
        Query<Entity, With<crate::game::party::body::PartyBody>>,
    ),
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
    for (entity, level, phase, state, children) in &levels {
        // Remember the level as the party leaves it; moving props are kept
        // where they were heading.
        if let (LevelPhase::Active, Some(state)) = (phase, state) {
            let children: Vec<Entity> = children.map_or_else(Vec::new, |c| c.to_vec());
            memory.0.insert(level.path.clone(), contents.remember(entity, state, &children));
        }
        commands.entity(entity).despawn();
    }
    pending.0 = None;
    for body in &party {
        commands
            .entity(body)
            .remove::<crate::game::party::walk_in::PathFlight>()
            .insert(RigidBodyDisabled);
    }
    let path = info.path();
    info!("travelling to {path} at {}", request.entrance);
    commands.spawn((Name::new(path.clone()), Level::load(&path, &assets)));
    *start = crate::game::party::body::PartyStart {
        entrance: Some(request.entrance.clone()),
        ..default()
    };
}

/// Levels the party has left, by path, as they will be on its return.
/// Not yet saved with the game.
#[derive(Resource, Debug, Default)]
pub struct LevelMemory(pub(super) HashMap<String, LevelRecord>);

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
    pub(super) groups: MonsterGroups,
    notes: MapNotes,
}

/// A monster as the party left it: where its feet were and which way it
/// faced, and its size.
#[derive(Clone, Debug)]
pub struct KeptMonster {
    pub monster: Monster,
    pub feet: Vec3,
    pub yaw: f32,
    pub size_roll: Option<u32>,
}

/// What travel reads to remember the level it leaves.
#[derive(SystemParam)]
pub struct LevelContents<'w, 's> {
    props: Query<'w, 's, (&'static Prop, &'static PropMotion)>,
    monsters: Query<'w, 's, (&'static Monster, &'static Transform, &'static MonsterAppearance)>,
    items: Query<'w, 's, (&'static WorldItem, &'static Transform)>,
    knowledge: Query<'w, 's, (Option<&'static MonsterGroups>, Option<&'static MapNotes>)>,
}

impl LevelContents<'_, '_> {
    fn remember(&self, root: Entity, state: &LevelState, children: &[Entity]) -> LevelRecord {
        let (groups, notes) = self.knowledge.get(root).unwrap_or_default();
        LevelRecord {
            triggers: state.remember(),
            prop_frames: children
                .iter()
                .filter_map(|&c| self.props.get(c).ok())
                .map(|(prop, motion)| (prop.definition, motion.target))
                .collect(),
            monsters: children
                .iter()
                .filter_map(|&c| self.monsters.get(c).ok())
                .map(|(monster, at, look)| KeptMonster {
                    monster: monster.clone(),
                    feet: at.translation - Vec3::Y * look.float_height,
                    yaw: at.rotation.to_euler(EulerRot::YXZ).0,
                    size_roll: look.size_roll,
                })
                .collect(),
            items: children
                .iter()
                .filter_map(|&c| self.items.get(c).ok())
                .map(|(item, at)| (item.clone(), at.translation))
                .collect(),
            groups: groups.cloned().unwrap_or_default(),
            notes: notes.cloned().unwrap_or_default(),
        }
    }
}

/// Where a level is in its life. It loads until its gameplay prerequisites
/// exist (geometry and collision, the shared databases, its navmeshes
/// loaded or failed), then [`activate_level`] builds its world, places the
/// party and makes it active; only an active level is played. Its sky and
/// other visuals load on their own.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LevelPhase {
    #[default]
    Loading,
    Active,
    /// It cannot be played: an asset failed or its collision is unusable.
    Failed,
}

/// Run condition: a level is being played.
pub fn level_active(levels: Query<&LevelPhase>) -> bool {
    levels.iter().any(|phase| *phase == LevelPhase::Active)
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
    level: &Level,
    geometry: &LevelGeometry,
    collider: Collider,
    remembered: Option<&LevelRecord>,
) {
    let mut root = commands.entity(root);
    root.insert((
        RigidBody::Static,
        collider,
        remembered.map_or_else(
            || LevelState::new(geometry, level.script),
            |r| LevelState::returning(geometry, level.script, &r.triggers),
        ),
        remembered.map_or_else(MapNotes::default, |r| r.notes.clone()),
    ));
    {
        root.with_children(|parent| {
            for (index, prop) in geometry.props.iter().enumerate() {
                let frame =
                    remembered.and_then(|r| r.prop_frames.get(&index)).copied().unwrap_or(0.0);
                let last_frame =
                    prop.parts.iter().map(|p| p.keys.len().saturating_sub(1)).max().unwrap_or(0);
                parent
                    .spawn((
                        Name::new(prop.name.clone()),
                        Prop {
                            definition: index,
                            trigger: prop.trigger.clone(),
                            door: prop.trigger.as_ref().is_some_and(|name| {
                                geometry
                                    .triggers
                                    .iter()
                                    .any(|t| &t.name == name && t.door.is_some())
                            }),
                            frames_per_second: prop.frames_per_second,
                            last_frame,
                            wraps: prop.wraps,
                            slots: prop.slots.clone(),
                        },
                        PropMotion { frame, target: frame },
                        Transform::default(),
                        Visibility::default(),
                    ))
                    .with_children(|prop_entity| {
                        for (part_index, part) in prop.parts.iter().enumerate() {
                            let at = if part.keys.is_empty() {
                                part.transform
                            } else {
                                pose(&part.keys, frame)
                            };
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
        });
        info!(
            "level built: {} parts, {} props, {} collision triangles",
            geometry.parts.len(),
            geometry.props.len(),
            geometry.collision.triangles.len()
        );
    }
}

/// The databases every level's gameplay needs.
#[derive(SystemParam)]
pub struct SharedData<'w> {
    data: Option<Res<'w, GameData>>,
    items: Res<'w, Assets<ItemCatalog>>,
    loot: Res<'w, Assets<LootTables>>,
    species: Res<'w, Assets<MonsterCatalog>>,
}

/// Make a loading level playable once its gameplay prerequisites exist, in
/// one step: build its world, navigation and population (consuming what was
/// remembered of it), place the party and resume its body, and announce it.
/// A level whose geometry, databases or collision fail is marked failed.
#[allow(clippy::too_many_arguments)]
pub fn activate_level(
    mut commands: Commands,
    mut levels: Query<(Entity, &Level, &mut LevelPhase)>,
    geometry: Res<Assets<LevelGeometry>>,
    assets: Res<AssetServer>,
    shared: SharedData,
    mut memory: ResMut<LevelMemory>,
    (mut ids, mut rng): (ResMut<ItemIds>, ResMut<GameRng>),
    mut party: PartyPlacement,
    mut ready: MessageWriter<LevelReady>,
) {
    for (root, level, mut phase) in &mut levels {
        if *phase != LevelPhase::Loading {
            continue;
        }
        let failed = |handle: UntypedAssetId| {
            matches!(assets.get_load_state(handle), Some(LoadState::Failed(_)))
        };
        let Some(data) = shared.data.as_ref() else { continue };
        if failed(level.geometry.id().untyped())
            || failed(data.items.id().untyped())
            || failed(data.loot.id().untyped())
            || failed(data.species.id().untyped())
        {
            error!("{}: an asset it needs failed to load", level.path);
            *phase = LevelPhase::Failed;
            continue;
        }
        let (Some(geometry), Some(items), Some(_), Some(species)) = (
            geometry.get(&level.geometry),
            shared.items.get(&data.items),
            shared.loot.get(&data.loot),
            shared.species.get(&data.species),
        ) else {
            continue;
        };
        if !navigation::navmeshes_settled(level, &assets) {
            continue;
        }
        let collider = match level_collider(geometry) {
            Ok(collider) => collider,
            Err(error) => {
                error!("{}: {error}", level.path);
                *phase = LevelPhase::Failed;
                continue;
            }
        };
        let remembered = memory.0.remove(&level.path);
        build_world(&mut commands, root, level, geometry, collider, remembered.as_ref());
        navigation::install(&mut commands, root, level, geometry, &assets);
        population::populate(
            &mut commands,
            root,
            geometry,
            remembered.as_ref(),
            (species, items),
            &assets,
            &mut ids,
            &mut rng,
        );
        party.place(&mut commands, geometry);
        *phase = LevelPhase::Active;
        info!("{} is active", level.path);
        ready.write(LevelReady { level: root });
    }
}
