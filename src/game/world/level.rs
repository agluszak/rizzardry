//! The active level: one root entity owning everything level-specific.
//!
//! The root carries [`Level`]; its geometry, collider and navigation are
//! built under it as their assets load. A level the party leaves is set
//! aside, not unloaded: it and everything it holds stay as they were, taking
//! no part in anything, until the party comes back and it wakes. The party is
//! not level-owned and survives travel.

use avian3d::prelude::*;
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::relationship::RelationshipTarget;
use bevy::prelude::*;
use bevy_landmass::NavMesh3d;

use bevy::asset::{LoadState, UntypedAssetId};
use bevy::ecs::system::SystemParam;

use super::navigation::bake::SIZE_CLASSES;
use super::population::MonsterBodies;
use super::scripts::LevelScript;
use super::triggers::spawn_devices;
use super::{navigation, population};
use crate::content::items::LootTables;
use crate::content::level::LevelDefinition;
use crate::content::levels::LevelInfo;
use crate::content::monster::MonsterBody;
use crate::content::navmesh::navmesh_path;
use crate::content::ui::TextTable;
use crate::game::automap::MapNotes;
use crate::game::characters::Character;
use crate::game::combat::Battle;
use crate::game::data::Catalogs;
use crate::game::magic::LearnedSpells;
use crate::game::party::body::{PartyPlacement, PartyStart};
use crate::game::party::roster::Party;
use crate::game::rng::GameRng;
use crate::game::time::GameClock;

/// The root of the level being played, including during travel preparation. A level
/// becomes one only by [`activate_level`]; until then its root is a
/// [`Destination`].
#[derive(Component, Debug, Clone)]
#[require(Transform, MonsterBodies, super::population::Unborn, super::navigation::LevelNavigation)]
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
    battle: Option<Single<&Battle>>,
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
    battle: Option<Single<&Battle>>,
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

/// On a level root: the party has left it. It and everything that belongs to
/// it ([`belonging`]) are [`Disabled`] until the party returns, so no query
/// sees them, and nothing in them runs, ages or is drawn.
#[derive(Component, Debug, Clone, Copy)]
pub struct SetAside {
    /// The game time, in milliseconds, at which the party left.
    pub at_ms: u64,
}

/// A level set aside has woken: the party is back. Whatever time owes a level
/// that was away, from how long that was, is for the rules that care to react
/// to.
#[derive(EntityEvent, Debug, Clone, Copy)]
pub struct Woke {
    #[event_target]
    pub level: Entity,
    pub away_ms: u64,
}

/// What belongs to a level: its root; whatever it owns; whatever hangs under
/// any of these in the hierarchy; and whatever any of these hold (what is
/// stowed in a container or bag, what a body wears or packs, the spells laid
/// on it). It sees what is set aside too.
#[derive(SystemParam)]
pub struct Belongings<'w, 's> {
    holds: Query<
        'w,
        's,
        (
            Option<&'static LevelEntities>,
            Option<&'static Children>,
            Option<&'static crate::game::items::Stows>,
            Option<&'static crate::game::items::Wears>,
            Option<&'static crate::game::items::Pack>,
            Option<&'static crate::game::actors::spells::Spells>,
        ),
        Allow<Disabled>,
    >,
}

impl Belongings<'_, '_> {
    /// Everything that belongs to the level at `root`, the root first.
    pub fn of(&self, root: Entity) -> Vec<Entity> {
        fn held<T: RelationshipTarget>(targets: Option<&T>) -> impl Iterator<Item = Entity> + '_ {
            targets.into_iter().flat_map(|targets| targets.iter())
        }
        let mut found = vec![root];
        let mut next = 0;
        while let Some(&entity) = found.get(next) {
            next += 1;
            let Ok((owned, children, stowed, worn, packed, laid)) = self.holds.get(entity) else {
                continue;
            };
            let more = held(owned)
                .chain(held(children))
                .chain(held(stowed))
                .chain(held(worn))
                .chain(held(packed))
                .chain(held(laid));
            for entity in more {
                if !found.contains(&entity) {
                    found.push(entity);
                }
            }
        }
        found
    }
}

/// The party leaves a level: all that belongs to it is set aside.
pub fn set_aside(add: On<Add<SetAside>>, belongings: Belongings, mut commands: Commands) {
    for entity in belongings.of(add.entity) {
        commands.entity(entity).insert(Disabled);
    }
}

/// The party returns to a level set aside: all that belongs to it is back,
/// and it is told how long it was away.
pub fn wake(
    remove: On<Remove<SetAside>>,
    aside: Query<&SetAside, Allow<Disabled>>,
    belongings: Belongings,
    clock: Res<GameClock>,
    mut commands: Commands,
) {
    let away_ms = clock.ms - aside.get(remove.entity).expect("it is woken").at_ms;
    for entity in belongings.of(remove.entity) {
        commands.entity(entity).try_remove::<Disabled>();
    }
    commands.trigger(Woke { level: remove.entity, away_ms });
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
    /// Body requests discovered once catalogs and geometry are available,
    /// then retained through loading and transferred to the active level.
    pub bodies: Option<MonsterBodies>,
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
    NewGame(Vec<(Character, LearnedSpells)>),
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
/// level's and every solid prop part's come with the level's data; a part
/// the party walks through (a sensor) only shows the way and may be left
/// without.
fn prepare_collision(geometry: &LevelDefinition) -> Collision {
    let level = super::props::trimesh(&geometry.collision, TrimeshFlags::FIX_INTERNAL_EDGES)
        .expect("a level's collision builds");
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
                        return None;
                    }
                    match super::props::trimesh(&part.collision, TrimeshFlags::empty()) {
                        Ok(collider) => Some(collider),
                        Err(error) if prop.collidable => {
                            panic!("solid prop {} has no collider: {error:?}", prop.name)
                        }
                        Err(error) => {
                            warn!("walk-through prop {} has no collider: {error:?}", prop.name);
                            None
                        }
                    }
                })
                .collect()
        })
        .collect();
    Collision { level, parts }
}

/// Build the level's playable world under its root: its collider, its props
/// (moving parts and their colliders), its trigger state and map notes. What
/// it looks like is presentation's to attach.
fn build_world(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelDefinition,
    collision: Collision,
    script: super::scripts::LevelScript,
    tables: &LootTables,
) {
    let props: Vec<_> =
        geometry.props.iter().map(|_| commands.spawn(LevelOwned(root)).id()).collect();
    spawn_devices(commands, root, geometry, &props, script, tables);
    commands.entity(root).insert((
        RigidBody::Static,
        collision.level,
        super::obstacles::scenery(),
        MapNotes::default(),
    ));
    for ((index, prop), colliders) in geometry.props.iter().enumerate().zip(collision.parts) {
        let door = prop.trigger.is_some_and(|index| geometry.triggers[index].door.is_some());
        super::props::build_prop(&mut commands.entity(props[index]), index, prop, door, colliders);
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

/// Commit an entry once its gameplay prerequisites exist, in one step: set
/// aside the level being left, make the destination the [`Level`] (waking it
/// if the party has been there, otherwise building its world, navigation and
/// population from its definition), create a new campaign's party, place the
/// party and announce it. Gameplay is suspended until then, so nothing can
/// begin in the meantime. An entry whose geometry, databases or collision
/// fail is discarded and the party plays on where it was.
pub fn activate_level(
    mut commands: Commands,
    mut destinations: Query<(Entity, &mut Destination)>,
    played: Query<Entity, With<Level>>,
    built: Query<(Entity, &Level, Has<SetAside>), Allow<Disabled>>,
    parties: Query<Entity, With<Party>>,
    geometry: Res<Assets<LevelDefinition>>,
    assets: Res<AssetServer>,
    mut shared: SharedData,
    mut npcs: crate::game::npcs::NpcPopulation,
    mut rng: ResMut<GameRng>,
    mut placement: PartyPlacement,
    clock: Res<GameClock>,
    mut ready: MessageWriter<LevelReady>,
) {
    for (root, mut destination) in &mut destinations {
        // Polling reads the destination; the commit consumes it.
        let destination = destination.bypass_change_detection();
        // A level the party has been to is not loaded or built again: it is
        // the played level already, or it is set aside and wakes.
        let known = built.iter().find(|(_, level, _)| level.path == destination.level.path);
        if known.is_none()
            && !prepare(
                &destination.level,
                &mut destination.bodies,
                &geometry,
                &shared,
                &mut npcs,
                &assets,
            )
        {
            continue;
        }
        let catalogs = shared.catalogs.as_deref().expect("prepared");
        let party = match std::mem::replace(&mut destination.entry, Entry::Travel) {
            Entry::Travel => parties.single().expect("a party travels"),
            Entry::NewGame(characters) => {
                found_party(&mut commands, characters, catalogs, &mut rng)
            }
        };
        for left in played.iter().filter(|&left| known.is_none_or(|(level, ..)| level != left)) {
            commands.entity(left).insert(SetAside { at_ms: clock.ms });
        }
        if let Some((level, definition, asleep)) = known.map(|(e, l, a)| (e, &l.definition, a)) {
            if asleep {
                commands.entity(level).remove::<SetAside>();
            }
            let geometry = geometry.get(definition).expect("a built level keeps its definition");
            placement.place(&mut commands, party, geometry, &destination.start);
            info!("{} is active again", destination.level.path);
            commands.entity(root).despawn();
            ready.write(LevelReady);
            break;
        }
        let level = &destination.level;
        let geometry = geometry.get(&level.definition).expect("prepared");
        // Built once, now that nothing else is awaited.
        let collision = prepare_collision(geometry);
        build_world(&mut commands, root, geometry, collision, level.script, &catalogs.loot);
        let navigation = navigation::install(&mut commands, root, level, geometry);
        population::populate(
            &mut commands,
            root,
            geometry,
            catalogs,
            (&level.path, &mut npcs),
            (destination.bodies.as_ref().unwrap(), &shared.bodies),
            (&navigation, &mut shared.motion_configs),
            &mut rng,
        );
        placement.place(&mut commands, party, geometry, &destination.start);
        navigation::add_party_characters(&mut commands, root, party, &navigation);
        info!("{} is active", level.path);
        let bodies = destination.bodies.take().unwrap();
        commands.entity(root).remove::<Destination>().insert((level.clone(), navigation, bodies));
        ready.write(LevelReady);
        break;
    }
}

/// Whether everything a destination needs has loaded. Its files come with
/// the game: one that fails to load is a broken installation.
fn prepare(
    level: &Level,
    bodies: &mut Option<MonsterBodies>,
    geometry: &Assets<LevelDefinition>,
    shared: &SharedData,
    npcs: &mut crate::game::npcs::NpcPopulation,
    assets: &AssetServer,
) -> bool {
    let failed = |handle: UntypedAssetId| {
        matches!(assets.get_load_state(handle), Some(LoadState::Failed(_)))
    };
    assert!(
        !failed(level.definition.id().untyped())
            && !level.navmeshes.iter().any(|h| failed(h.id().untyped())),
        "{}: a file it needs failed to load",
        level.path
    );
    let (Some(geometry), Some(catalogs)) = (geometry.get(&level.definition), &shared.catalogs)
    else {
        return false;
    };
    let species = &catalogs.species;
    if !npcs.prepare(catalogs, geometry, assets) {
        return false;
    }
    let bodies = bodies.get_or_insert_with(|| MonsterBodies::for_level(geometry, species, assets));
    assert!(
        !bodies.0.values().any(|body| failed(body.id().untyped())),
        "{}: a monster's body definition failed to load",
        level.path
    );
    bodies.0.values().all(|body| shared.bodies.contains(body))
        && navigation::navmeshes_ready(level, assets)
}

/// A new campaign's party, its members joining complete and in order.
fn found_party(
    commands: &mut Commands,
    characters: Vec<(Character, LearnedSpells)>,
    content: &crate::game::data::Catalogs,
    rng: &mut GameRng,
) -> Entity {
    let party = commands.spawn((Name::new("Party"), Party)).id();
    for member in characters {
        crate::game::party::roster::recruit_character(commands, party, member, content, rng);
    }
    party
}
