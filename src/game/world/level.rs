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
use super::population::{Monster, MonsterBodies};
use super::props::{Prop, PropMotion};
use super::scripts::LevelScript;
use super::triggers::{DeviceRecords, LevelState, TriggerMemory, spawn_devices};
use super::{navigation, population};
use crate::content::items::LootTables;
use crate::content::level::LevelDefinition;
use crate::content::levels::LevelInfo;
use crate::content::monster::MonsterBody;
use crate::content::navmesh::navmesh_path;
use crate::content::ui::TextTable;
use crate::game::automap::MapNotes;
use crate::game::data::Catalogs;
use crate::game::inventory::ItemIds;
use crate::game::party::body::{PartyPlacement, PartyStart};
use crate::game::party::roster::Party;
use crate::game::rng::GameRng;

/// Whether the party is on its way into a level, for operations that hold
/// the whole world.
pub fn entering(world: &mut World) -> bool {
    world.query_filtered::<(), With<Destination>>().iter(world).next().is_some()
}

/// The root of the level being played, including during travel preparation. A level
/// becomes one only by [`activate_level`]; until then its root is a
/// [`Destination`].
#[derive(Component, Debug, Clone)]
#[require(Transform, MonsterBodies)]
pub struct Level {
    /// The retail path of its `.pvl`, which identifies it.
    pub path: String,
    pub definition: Handle<LevelDefinition>,
    /// One navmesh per walker size class (`navigation::bake::SIZE_CLASSES`).
    pub navmeshes: Vec<Handle<NavMesh3d>>,
    pub script: LevelScript,
    /// The messages its triggers show.
    pub messages: Option<Handle<TextTable>>,
}

impl Level {
    /// The level at a retail `.pvl` path, with its gameplay prerequisites.
    pub fn load(path: &str, assets: &AssetServer) -> Self {
        let info = LevelInfo::for_path(path);
        Level {
            path: path.to_owned(),
            definition: assets.load(crate::content::paths::retail(path)),
            navmeshes: (0..SIZE_CLASSES.len())
                .map(|class| assets.load(navmesh_path(path, class)))
                .collect(),
            script: info
                .map_or_else(LevelScript::default, |info| LevelScript::for_location(info.location)),
            messages: info
                .map(|info| assets.load(crate::content::paths::level_messages(info.folder))),
        }
    }
}

/// A level as if already activated, without navmeshes or messages. Not a
/// `Default`: a `Level` exists only once [`activate_level`] says so.
#[cfg(test)]
impl Level {
    pub fn test(definition: Handle<LevelDefinition>) -> Self {
        Level {
            path: "test".into(),
            definition,
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
    questions: Query<(Entity, &TravelQuestion)>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    mut travel: MessageWriter<Travel>,
) {
    let answers: Vec<_> = answers.read().copied().collect();
    // A departed level's questions went with it.
    for (entity, question) in &questions {
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

/// Accept travel: from here until the destination commits or fails, the
/// level being left is suspended, not snapshotted. Its questions close and
/// the party's movement intent clears; activation owns the departure.
pub fn travel(
    mut commands: Commands,
    mut requests: MessageReader<Travel>,
    entering: Query<(), With<Destination>>,
    questions: Query<Entity, With<TravelQuestion>>,
    assets: Res<AssetServer>,
    battle: Option<Single<&crate::game::combat::Battle>>,
    camp: Option<Res<crate::game::camp::Camp>>,
    mut movement: Query<&mut crate::game::party::body::PartyMovement>,
) {
    let Some(request) = requests.read().last() else { return };
    if battle.is_some() || camp.is_some() || !entering.is_empty() {
        warn!("travel to {} refused: the party is occupied", request.entrance);
        return;
    }
    let Some(info) = LevelInfo::for_entrance(&request.entrance) else {
        warn!("no level has entrance {}", request.entrance);
        return;
    };
    for question in &questions {
        commands.entity(question).despawn();
    }
    for mut movement in &mut movement {
        *movement = default();
    }
    let path = info.path();
    let start = PartyStart { entrance: Some(request.entrance.clone()), ..default() };
    commands.spawn((
        Name::new(path.clone()),
        Destination::new(Level::load(&path, &assets), start, Entry::Travel),
    ));
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
/// what its triggers keep, its props’ motion, its monsters (the
/// dead too, so they stay dead) and those still unborn, the items lying
/// about, what the party knows of its groups, and its map notes.
/// Activation consumes it.
#[derive(Debug)]
pub struct LevelRecord {
    triggers: TriggerMemory,
    /// By prop definition index.
    prop_motion: HashMap<usize, PropMotion>,
    pub(super) monsters: Vec<KeptMonster>,
    pub(super) unborn: Vec<KeptMonster>,
    pub(super) encounters: super::encounters::EncounterState,
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

impl KeptMonster {
    /// A placed monster met for the first time (retail `LoadMonsterInfo`).
    pub fn placed(
        placed: &crate::content::level::MonsterPlacement,
        kind: &crate::content::species::Species,
        body: &crate::content::monster::MonsterBody,
        rng: &mut crate::game::rng::GameRng,
    ) -> Self {
        Self::rolled(
            (placed.location, placed.species, Some(placed.group)),
            (placed.position, placed.yaw),
            (kind, body),
            rng,
        )
    }

    /// A monster come into being (retail `CreateMonsterInfo`): health and
    /// stamina rolled from its species, then its size. Retail draws
    /// Random(1000) for the size; its left-handed mirror draw, presentation
    /// only, is not taken from gameplay's stream.
    pub fn rolled(
        (location, species, group): (i32, u32, Option<i32>),
        (feet, yaw): (Vec3, f32),
        (kind, body): (&crate::content::species::Species, &crate::content::monster::MonsterBody),
        rng: &mut crate::game::rng::GameRng,
    ) -> Self {
        let max_hp = rng.roll(kind.hit_points).max(1);
        let max_stamina = rng.roll(kind.stamina);
        let body = super::population::ScaledBody::new(body, rng.below(1000));
        KeptMonster {
            monster: Monster {
                enchantments: default(),
                location,
                species,
                hp: max_hp,
                max_hp,
                stamina: max_stamina,

                max_stamina,
                conditions: Default::default(),
                spell_points: 0,
                body,
            },
            group,
            feet,
            yaw,
        }
    }

    /// A monster as it stands now, to keep while its level is away.
    pub fn capture(
        monster: &Monster,
        pose: &super::ActorPoseItem,
        walker: Option<&super::navigation::Walker>,
        group: Option<i32>,
    ) -> Self {
        KeptMonster {
            group,
            monster: monster.clone(),
            feet: super::navigation::feet(pose.translation(), walker),
            yaw: pose.rotation().to_euler(EulerRot::YXZ).0,
        }
    }
}

/// What travel reads to remember the level it leaves.
#[derive(SystemParam)]
pub struct LevelContents<'w, 's> {
    devices: DeviceRecords<'w, 's>,
    lights: Query<'w, 's, &'static super::triggers::LightActivation>,
    props: Query<'w, 's, (&'static Prop, &'static PropMotion)>,
    monsters: Query<
        'w,
        's,
        (&'static Monster, super::ActorPose, Option<&'static Walker>, Option<&'static GroupMember>),
        Without<crate::game::npcs::NpcBody>,
    >,
    items: Query<'w, 's, (&'static WorldItem, &'static Transform)>,
    groups: Query<'w, 's, &'static MonsterGroup>,
    notes: Query<'w, 's, &'static MapNotes>,
    unborn: Query<'w, 's, &'static super::population::Unborn>,
    encounters: Query<'w, 's, &'static super::encounters::EncounterState>,
    clock: Res<'w, crate::game::time::GameClock>,
}

impl LevelContents<'_, '_> {
    fn remember(&self, root: Entity, state: &LevelState, children: &[Entity]) -> LevelRecord {
        let notes = self.notes.get(root).ok();
        LevelRecord {
            triggers: self.devices.remember(
                children,
                state,
                self.lights.get(root).expect("active levels own light state"),
            ),
            prop_motion: self
                .props
                .iter_many(children)
                .matched()
                .map(|(prop, motion)| (prop.definition, *motion))
                .collect(),
            monsters: self
                .monsters
                .iter_many(children)
                .matched()
                .map(|(monster, at, walker, group)| {
                    let group = group.and_then(|m| self.groups.get(m.0).ok()).map(|g| g.authored);
                    KeptMonster::capture(monster, &at, walker, group)
                })
                .collect(),
            unborn: self.unborn.get(root).map_or_else(|_| Vec::new(), |unborn| unborn.0.clone()),
            encounters: super::encounters::EncounterState {
                arrival: Some(super::encounters::Arrival::Returned { left: self.clock.ms / 1000 }),
                ..self.encounters.get(root).cloned().unwrap_or_default()
            },
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

/// An accepted level entry being prepared (retail's please-wait load). While
/// one exists, gameplay and physics are suspended; the level being left, if
/// any, stays intact. It loads until its gameplay prerequisites exist
/// (geometry and collision, the shared databases, its monsters' bodies, its
/// navmeshes); then [`activate_level`] commits the entry in one step and
/// makes it the [`Level`]. If any of them fails, the entry is discarded and
/// play resumes where it was. Its sky and other visuals load on their own
/// once it is played.
#[derive(Component, Debug)]
#[require(Transform)]
pub struct Destination {
    pub level: Level,
    pub start: PartyStart,
    pub entry: Entry,
    /// Its monsters' bodies, held while it loads.
    pub bodies: MonsterBodies,
}

impl Destination {
    pub fn new(level: Level, start: PartyStart, entry: Entry) -> Self {
        Destination { level, start, entry, bodies: default() }
    }
}

/// Why a level is entered. Retail also restores saved games, whose entry
/// effects differ; Rizzardry cannot save yet.
#[derive(Debug)]
pub enum Entry {
    /// A new campaign: the accepted party, which joins only when the level
    /// commits, so a failed first load leaves no campaign behind.
    NewGame(Vec<(crate::game::characters::Character, crate::game::magic::LearnedSpells)>),
    /// The party travels on from the level it is in.
    Travel,
}

/// The definitions of the levels that own entities: what a placed prop,
/// emitter or other authored thing is, read where its level keeps it.
#[derive(bevy::ecs::system::SystemParam)]
pub struct OwnedDefinitions<'w, 's> {
    levels: Query<'w, 's, &'static Level>,
    definitions: Res<'w, Assets<LevelDefinition>>,
}

impl OwnedDefinitions<'_, '_> {
    pub fn of(&self, owner: &LevelOwned) -> Option<&LevelDefinition> {
        self.definitions.get(&self.levels.get(owner.0).ok()?.definition)
    }
}

/// An entry could not be prepared and was discarded.
#[derive(Message, Debug, Clone)]
pub struct LevelEntryFailed {
    pub path: String,
}

/// The level being played, and its geometry.
#[derive(SystemParam)]
pub struct PlayedLevel<'w, 's> {
    levels: Query<'w, 's, (Entity, &'static Level)>,
    definitions: Res<'w, Assets<LevelDefinition>>,
}

impl PlayedLevel<'_, '_> {
    pub fn entity(&self) -> Option<Entity> {
        self.levels.single().ok().map(|(entity, _)| entity)
    }

    pub fn definition(&self) -> Option<&LevelDefinition> {
        self.definitions.get(&self.levels.single().ok()?.1.definition)
    }

    /// A level's definition, by the level.
    pub fn of(&self, level: Entity) -> Option<&LevelDefinition> {
        self.definitions.get(&self.levels.get(level).ok()?.1.definition)
    }

    pub fn view_distance(&self) -> Option<f32> {
        self.definition().map(LevelDefinition::view_distance)
    }
}

/// Run condition: a level is being played, not suspended by an entry in
/// preparation.
pub fn level_active(
    levels: Query<(), With<Level>>,
    entering: Query<(), With<Destination>>,
) -> bool {
    !levels.is_empty() && entering.is_empty()
}

/// Physics stands still while an entry is prepared, like the rest of
/// gameplay (whose systems [`level_active`] gates).
pub fn suspend_physics_for_entry(_: On<Add<Destination>>, mut time: ResMut<Time<Physics>>) {
    time.pause();
}

/// Physics resumes once no entry is being prepared.
pub fn resume_physics_after_entry(
    _: On<Remove<Destination>>,
    entering: Query<(), With<Destination>>,
    mut time: ResMut<Time<Physics>>,
) {
    // The entry going away still counts while it is removed.
    if entering.iter().count() <= 1 {
        time.unpause();
    }
}

/// A level has become active, with the party in it.
#[derive(Message, Debug, Clone, Copy)]
pub struct LevelReady;

/// A level's collision, built and checked before its entry commits: the
/// level's own, and each prop part's by prop and part. Walk-through sprites
/// have none.
struct Collision {
    level: Collider,
    parts: Vec<Vec<Option<Collider>>>,
}

/// Build all of a level's collision once its entry is otherwise ready. The
/// level's and every solid prop part's are mandatory: one that cannot be
/// built fails the entry before anything changes. A part the party walks
/// through (a sensor) only shows the way and may be left without.
fn prepare_collision(geometry: &LevelDefinition) -> Result<Collision, String> {
    let level = super::props::trimesh(&geometry.collision, TrimeshFlags::FIX_INTERNAL_EDGES)
        .map_err(|error| format!("level collider: {error:?}"))?;
    let parts = geometry
        .props
        .iter()
        .map(|prop| {
            prop.parts
                .iter()
                .map(|part| {
                    // Retail's solidity is the collidable flag alone, as the
                    // navmesh bakes it; only a walk-through sprite, which
                    // turns with the view, keeps no fixed shape to point at.
                    if prop.face_camera && !prop.collidable {
                        return Ok(None);
                    }
                    match super::props::trimesh(&part.collision, TrimeshFlags::empty()) {
                        Ok(collider) => Ok(Some(collider)),
                        Err(error) if prop.collidable => {
                            Err(format!("prop {} collider: {error:?}", prop.name))
                        }
                        Err(error) => {
                            warn!("walk-through prop {} has no collider: {error:?}", prop.name);
                            Ok(None)
                        }
                    }
                })
                .collect()
        })
        .collect::<Result<_, String>>()?;
    Ok(Collision { level, parts })
}

/// Build the level's playable world under its root: its collider, its props
/// (moving parts and their colliders) where it left them, its trigger state
/// and map notes. What it looks like is presentation's to attach.
fn build_world(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelDefinition,
    collision: Collision,
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
        collision.level,
        super::obstacles::scenery(),
        remembered.as_deref_mut().map_or_else(MapNotes::default, |r| std::mem::take(&mut r.notes)),
    ));
    for ((index, prop), colliders) in geometry.props.iter().enumerate().zip(collision.parts) {
        let motion = remembered
            .as_ref()
            .and_then(|r| r.prop_motion.get(&index))
            .copied()
            .unwrap_or_default();
        let door = prop.trigger.is_some_and(|index| geometry.triggers[index].door.is_some());
        super::props::build_prop(
            &mut commands.entity(props[index]),
            index,
            prop,
            door,
            motion,
            colliders,
        );
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
    catalogs: Option<Res<'w, Catalogs>>,
    bodies: Res<'w, Assets<MonsterBody>>,
    motion_configs: ResMut<'w, Assets<navigation::WalkerMotionConfig>>,
}

/// Commit an entry once its gameplay prerequisites exist, in one step:
/// capture and retire the level being left, build the destination's world,
/// navigation and population (consuming what was remembered of it), create
/// a new campaign's party, place the party, make the destination the
/// [`Level`] and announce it. Gameplay is suspended until then, so nothing
/// can begin in the meantime. An entry whose geometry, databases or
/// collision fail is discarded and the party plays on where it was.
pub fn activate_level(
    mut commands: Commands,
    mut destinations: Query<(Entity, &mut Destination)>,
    current: Query<(Entity, &Level, &LevelState, Option<&LevelEntities>)>,
    parties: Query<Entity, With<Party>>,
    geometry: Res<Assets<LevelDefinition>>,
    assets: Res<AssetServer>,
    mut shared: SharedData,
    mut npcs: crate::game::npcs::NpcPopulation,
    mut memory: ResMut<LevelMemory>,
    (mut ids, mut rng): (ResMut<ItemIds>, ResMut<GameRng>),
    mut transition: ParamSet<(LevelContents, PartyPlacement)>,
    (mut ready, mut failures): (MessageWriter<LevelReady>, MessageWriter<LevelEntryFailed>),
) {
    for (root, mut destination) in &mut destinations {
        // Polling reads the destination; the commit consumes it.
        let destination = destination.bypass_change_detection();
        let level = &destination.level;
        let mut fail = |why: &str| {
            error!("{}: {why}", level.path);
            failures.write(LevelEntryFailed { path: level.path.clone() });
            commands.entity(root).despawn();
        };
        let bodies = &mut destination.bodies;
        match prepare(level, bodies, &geometry, &shared, &mut npcs, &assets) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(why) => {
                fail(&why);
                continue;
            }
        }
        let geometry = geometry.get(&level.definition).expect("prepared");
        let catalogs = shared.catalogs.as_deref().expect("prepared");
        let (items, loot) = (&catalogs.items, &catalogs.loot);
        let traveller = parties.single().ok();
        if matches!(destination.entry, Entry::Travel) && traveller.is_none() {
            fail("there is no party to travel");
            continue;
        }
        // Built once, now that nothing else is awaited, and before the
        // source is touched.
        let collision = match prepare_collision(geometry) {
            Ok(collision) => collision,
            Err(error) => {
                fail(&error);
                continue;
            }
        };
        if let Entry::NewGame(characters) = &destination.entry
            && let Err(missing) = crate::game::party::roster::check_starting_items(
                characters.iter().map(|(character, _)| character),
                items,
            )
        {
            fail(&missing.to_string());
            continue;
        }
        // Capture the source's state exactly at departure; it has not
        // changed since travel was accepted.
        for (entity, source, state, owned) in &current {
            let owned = owned.map_or(&[][..], LevelEntities::entities);
            memory.0.insert(source.path.clone(), transition.p0().remember(entity, state, owned));
            npcs.depart(&mut commands, entity);
            commands.entity(entity).despawn();
        }
        let mut remembered = memory.0.remove(&level.path);
        build_world(
            &mut commands,
            root,
            geometry,
            collision,
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
            catalogs,
            (&level.path, &mut npcs),
            (&destination.bodies, &shared.bodies),
            (&navigation, &mut shared.motion_configs),
            &mut ids,
            &mut rng,
        );
        let party = match std::mem::replace(&mut destination.entry, Entry::Travel) {
            Entry::Travel => traveller.unwrap(),
            Entry::NewGame(characters) => {
                found_party(&mut commands, characters, items, (&mut ids, &mut rng))
            }
        };
        transition.p1().place(&mut commands, party, geometry, &destination.start);
        navigation::add_party_characters(&mut commands, root, party, &navigation);
        info!("{} is active", level.path);
        let bodies = std::mem::take(&mut destination.bodies);
        commands.entity(root).remove::<Destination>().insert((level.clone(), navigation, bodies));
        ready.write(LevelReady);
        break;
    }
}

/// Whether a destination can be entered: once everything it needs has
/// loaded and checks out, not while something is still loading, or why it
/// never will.
fn prepare(
    level: &Level,
    bodies: &mut MonsterBodies,
    geometry: &Assets<LevelDefinition>,
    shared: &SharedData,
    npcs: &mut crate::game::npcs::NpcPopulation,
    assets: &AssetServer,
) -> Result<bool, String> {
    let failed = |handle: UntypedAssetId| {
        matches!(assets.get_load_state(handle), Some(LoadState::Failed(_)))
    };
    if failed(level.definition.id().untyped())
        || level.navmeshes.iter().any(|h| failed(h.id().untyped()))
    {
        return Err("an asset it needs failed to load".into());
    }
    let (Some(geometry), Some(catalogs)) = (geometry.get(&level.definition), &shared.catalogs)
    else {
        return Ok(false);
    };
    let (items, species) = (&catalogs.items, &catalogs.species);
    population::check(geometry, species, items)?;
    super::triggers::check(geometry)?;
    if !npcs.prepare(catalogs, &geometry.monsters, assets)? {
        return Ok(false);
    }
    let bodies = bodies.for_level(geometry, species, assets);
    if bodies.iter().any(|body| failed(body.id().untyped())) {
        return Err("a monster's body definition failed to load".into());
    }
    Ok(bodies.iter().all(|body| shared.bodies.contains(body))
        && navigation::navmeshes_ready(level, assets))
}

/// A new campaign's party, its members joining complete and in order.
fn found_party(
    commands: &mut Commands,
    characters: Vec<(crate::game::characters::Character, crate::game::magic::LearnedSpells)>,
    items: &crate::content::items::ItemCatalog,
    (ids, rng): (&mut ItemIds, &mut GameRng),
) -> Entity {
    let party = commands.spawn((Name::new("Party"), Party)).id();
    let mut formation = crate::game::party::formation::Formation::default();
    for member in characters {
        crate::game::party::roster::recruit_character(
            commands,
            party,
            &mut formation,
            member,
            items,
            (&mut *ids, &mut *rng),
        );
    }
    commands.entity(party).insert(formation);
    party
}

#[cfg(test)]
mod collision_tests {
    use super::*;
    use crate::content::level::{CollisionMesh, PropModel, PropModelPart};

    fn triangle() -> CollisionMesh {
        CollisionMesh {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Z],
            triangles: vec![[0, 1, 2]],
            walkable: vec![true],
        }
    }

    /// Whether a prop is solid follows its collidable flag, as the navmesh
    /// bakes it, whatever way it is drawn.
    #[test]
    fn solidity_follows_the_collidable_flag_not_the_drawing() {
        let prop = |collidable, face_camera| PropModel {
            name: "PROP".into(),
            collidable,
            face_camera,
            frames_per_second: 10.0,
            parts: vec![PropModelPart {
                transform: Transform::IDENTITY,
                keys: vec![],
                collision: triangle(),
            }],
            wraps: false,
            slots: vec![],
            trigger: None,
        };
        let geometry = LevelDefinition {
            collision: triangle(),
            props: vec![prop(true, true), prop(true, false), prop(false, false), prop(false, true)],
            ..default()
        };
        let collision = prepare_collision(&geometry).unwrap();
        let has = |prop: usize| collision.parts[prop][0].is_some();
        assert!(has(0), "a collidable sprite is solid");
        assert!(has(1));
        assert!(has(2), "a walk-through prop keeps its sensor");
        assert!(!has(3), "a walk-through sprite has no fixed shape");
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
        {
            // A departed level's question goes with it.
            let (mut app, level) = app();
            let question = ask(&mut app, level, "departed");
            app.world_mut().despawn(level);
            app.world_mut().write_message(TravelAnswer { question, accept: true });
            app.update();
            assert!(app.world().get_entity(question).is_err());
            assert!(app.world().resource::<Messages<Travel>>().is_empty());
        }
        let (mut app, level) = app();
        app.world_mut().spawn(crate::game::combat::Battle::default());
        let question = ask(&mut app, level, "fighting");
        app.world_mut().write_message(TravelAnswer { question, accept: true });
        app.update();
        assert!(app.world().get_entity(question).is_err());
        assert!(app.world().resource::<Messages<Travel>>().is_empty());
    }
}
