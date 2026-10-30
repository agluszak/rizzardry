//! Separate immutable gameplay and presentation assets for a level.

use std::ops::RangeInclusive;

use bevy::prelude::*;

use super::items::{Dice, ItemId};
use num_enum::{IntoPrimitive, TryFromPrimitive};

/// One renderable piece of the static level: a compiled octree mesh's
/// polygons that share a material.
#[derive(Clone, Debug)]
pub struct LevelPart {
    pub mesh: Handle<Mesh>,
    pub surface: super::surface::Surface,
    /// The inputs of the mesh's vertex colours, per vertex.
    pub lighting: VertexLighting,
}

/// Retail bakes lighting into vertex colours and redoes it when the time of
/// day changes the sky light: colour = sky x reflectance x sun + fixed.
#[derive(Clone, Debug, Default)]
pub struct VertexLighting {
    /// Light that does not change with the time of day: static lights, the
    /// environment offset and emission.
    pub fixed: Vec<Vec3>,
    /// How exposed each vertex is to the sky (0..1).
    pub sun: Vec<f32>,
    /// The material's ambient reflectance.
    pub reflectance: Vec3,
}

impl VertexLighting {
    pub fn colours(&self, sky: Vec3) -> Vec<[f32; 4]> {
        self.fixed
            .iter()
            .zip(&self.sun)
            .map(|(&fixed, &sun)| {
                let c = (sky * self.reflectance * sun + fixed).min(Vec3::ONE);
                [c.x, c.y, c.z, 1.0]
            })
            .collect()
    }
}

/// How far is seen in a level without fog options: retail's far clip of
/// 75000 units.
pub const DEFAULT_VIEW_DISTANCE: f32 = 150.0;

/// A level's fog and sky light (retail "fog options").
#[derive(Clone, Debug)]
pub struct LevelEnvironment {
    /// Whether the authored level enables distance fog.
    pub fog_enabled: bool,
    /// Fog start and end as fractions of the view distance.
    pub fog_range: (f32, f32),
    /// The far clip, in metres.
    pub view_distance: f32,
    /// Scales the ambient colour; 0 for interiors.
    pub intensity: f32,
    /// Fog and clear colour by time-of-day phase (0 midnight, 128 noon).
    pub fog_colours: Vec<Vec3>,
    /// Sky light by phase, before `intensity`.
    pub ambient_colours: Vec<Vec3>,
}

impl LevelEnvironment {
    pub fn sky_light(&self, phase: u8) -> Vec3 {
        (self.ambient_colours[phase as usize] * self.intensity).min(Vec3::ONE)
    }
}

/// A level's point lights and constant light offset. Level geometry has
/// them baked in; props and actors take them at their positions.
#[derive(Clone, Debug, Default)]
pub struct LevelLighting {
    /// Added to everything's light (the level's environment offset).
    pub offset: Vec3,
    pub lights: Vec<PointLightSource>,
    /// The `SUN` light's position: actors in its line of sight get sky light.
    pub sun: Option<Vec3>,
}

/// A level light with retail's linear falloff and authored activation.
#[derive(Clone, Debug)]
pub struct PointLightSource {
    pub name: String,
    pub runtime: bool,
    pub enabled: bool,
    pub position: Vec3,
    /// Colour times intensity.
    pub colour: Vec3,
    /// Metres.
    pub range: f32,
}

impl PointLightSource {
    /// Light at a point in range: `colour * (1 - d / range)`.
    pub fn at(&self, position: Vec3) -> Vec3 {
        let distance = self.position.distance(position);
        if distance >= self.range || distance == 0.0 {
            return Vec3::ZERO;
        }
        self.colour * (1.0 - distance / self.range)
    }
}

/// Immutable gameplay content. Loading it never constructs rendering assets.
#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct LevelDefinition {
    pub particles: Vec<super::particles::ParticleConfig>,
    /// Authored underwater camera distortion, changed by trigger calls.
    pub camera_sway: bool,
    pub collision: CollisionMesh,
    pub map_layers: Vec<f32>,
    /// The authored sight limit, independent of any rendering camera.
    pub sight_distance: Option<f32>,
    pub entrances: Vec<Entrance>,
    pub triggers: Vec<TriggerDefinition>,
    pub camera_paths: Vec<CameraPathModel>,
    pub monsters: Vec<MonsterPlacement>,
    pub items: Vec<ItemPlacement>,
    pub props: Vec<PropModel>,
    pub links: Vec<LevelLink>,
    pub encounters: Encounters,
}

/// A level's random encounters as authored (retail `MonGen.cpp`: the
/// `.sts` `MONG` chunk, its row of `LEVELS.DBS` and the encounter tables
/// its generators draw from).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Encounters {
    pub limits: EncounterLimits,
    /// The running budget the level starts with.
    pub budget: i32,
    /// Generation is held off altogether (retail `g_generator_save_flag`).
    pub held: bool,
    /// The chance (%) and interval (seconds) of generators using the
    /// defaults (retail `g_generator_interval_min`,
    /// `g_generator_default_interval`).
    pub default_chance: i16,
    pub default_interval: i16,
    pub generators: Vec<Generator>,
}

/// A level's bounds on random encounters (retail `LEVELS.DBS`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncounterLimits {
    /// How many may be about at once, at most and at least, whatever the
    /// budget.
    pub most: i32,
    pub fewest: i32,
    /// The budget's bounds.
    pub largest_budget: i32,
    pub smallest_budget: i32,
    /// Game seconds that earn the budget one more.
    pub period: i32,
    /// How long an encounter lingers before it may be retired, in units of
    /// twelve game seconds (retail scales it by `g_sight_default`).
    pub lingers: i32,
}

/// A monster generator (retail `MonGen`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Generator {
    pub name: String,
    pub enabled: bool,
    /// Turned off (retail flag bit 0).
    pub disabled: bool,
    /// Uses the level's default chance and interval (bit 3).
    pub uses_defaults: bool,
    /// Generates only for a hostile faction while one of the possessed
    /// story facts holds (bit 5; harassment tables).
    pub harasses: bool,
    /// Its own chance (%) and interval (seconds).
    pub chance: u8,
    pub interval: u16,
    /// Where its encounters come, at the feet.
    pub position: Vec3,
    /// How far its timer had run, as a share of its interval.
    pub progress: f32,
    pub table: Option<EncounterTable>,
}

/// What a generator may call up (retail `EncounterTables.dbs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EncounterTable {
    pub name: String,
    pub entries: Vec<EncounterEntry>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncounterEntry {
    pub species: u32,
    /// How common: retail's classes 3, 7, 20 and 70.
    pub rarity: u8,
    pub time: EncounterTime,
    /// The party level it suits.
    pub challenge: u8,
}

/// When an entry may come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncounterTime {
    Day,
    Night,
    #[default]
    Always,
}

impl LevelDefinition {
    pub fn view_distance(&self) -> f32 {
        self.sight_distance.unwrap_or(DEFAULT_VIEW_DISTANCE)
    }
}

/// Optional presentation content loaded separately from the same retail source.
#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct LevelGeometry {
    pub particles: Vec<super::surface::Surface>,
    pub parts: Vec<LevelPart>,
    pub environment: Option<LevelEnvironment>,
    pub lighting: LevelLighting,
    pub ambient_sounds: Vec<AmbientSoundModel>,
    /// Indexed in the same authored order as `LevelDefinition::props`.
    pub props: Vec<PropVisual>,
}

#[derive(Clone, Debug)]
pub struct PropVisual {
    pub name: String,
    pub face_camera: bool,
    pub parts: Vec<PropVisualPart>,
}

#[derive(Clone, Debug)]
pub struct PropVisualPart {
    /// Used by sky props; level props inherit the live gameplay part's pose.
    pub transform: Transform,
    pub meshes: Vec<LevelPart>,
}

/// How a walker gets from one end of a link to the other.
/// Its number is its Landmass link kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(usize)]
pub enum LinkKind {
    /// A teleport pad or portal: arrive at once.
    Teleport,
    /// A ladder or rope: retail flies the party along a camera path.
    Climb,
    /// Walk through an open door.
    Door,
}

/// A one-way link between two ground positions.
#[derive(Clone, Copy, Debug)]
pub struct LevelLink {
    pub kind: LinkKind,
    pub from: Vec3,
    pub to: Vec3,
}

/// A monster placed in the level (retail `.sts` `MONS`).
#[derive(Clone, Debug)]
pub struct MonsterPlacement {
    /// Unique within the level.
    pub location: i32,
    /// The group it belongs to, by the level's group id.
    pub group: i32,
    /// Its species, by `monsters.dbs` record; the species gives its model.
    pub species: u32,
    /// Feet.
    pub position: Vec3,
    pub yaw: f32,
    /// Waits unborn until something births its group.
    pub unborn: bool,
}

/// An item lying in the level as authored (retail `.sts` `ITEM`).
#[derive(Clone, Debug)]
pub struct ItemPlacement {
    pub item: ItemId,
    /// A stack's count, or charges or uses, by the item's quantity kind.
    pub stack: u8,
    pub uses: u8,
    pub identified: bool,
    /// The item's origin, about a quarter metre above the ground.
    pub position: Vec3,
    /// Unseen until found by searching.
    pub hidden: bool,
}

/// A sound a place makes (retail ambient sound).
#[derive(Clone, Debug)]
pub struct AmbientSoundModel {
    pub name: String,
    /// Retail path under `data/sound`.
    pub path: String,
    pub position: Vec3,
    /// Metres; heard fully within 40% of it, fading to nothing at its edge.
    pub radius: f32,
    pub looping: bool,
    /// Loudness, 0 to 1.
    pub volume: RangeInclusive<f32>,
    /// Seconds between plays of a one-shot sound.
    pub interval: RangeInclusive<f32>,
}

/// A path the party's eye flies along: eye position and view per key.
#[derive(Clone, Debug)]
pub struct CameraPathModel {
    pub name: String,
    pub keys: Vec<Transform>,
    pub keys_per_second: f32,
}

/// Where a teleport takes the party.
#[derive(Clone, Debug, PartialEq)]
pub enum Arrival {
    /// The eye's position and facing in this level.
    Here { eye: Vec3, yaw: f32 },
    /// An entrance trigger of another level, named by its location code and
    /// number, e.g. `MO201`.
    Entrance {
        entrance: String,
        /// Ask "Proceed to <level>?" first.
        ask: bool,
    },
}

/// A positioned trigger the party can enter the level at.
#[derive(Clone, Debug)]
pub struct Entrance {
    pub name: String,
    /// Retail places the eye 1000 units above the trigger and lets the party
    /// settle.
    pub eye: Vec3,
    pub yaw: f32,
}

/// A placed prop: scenery, a door, a lever, a lift or a billboard.
#[derive(Clone, Debug)]
pub struct PropModel {
    pub name: String,
    /// Retail `COLLIDABLE`: the party and monsters collide with its parts.
    pub collidable: bool,
    /// Drawn turned toward the viewer, like a sprite.
    pub face_camera: bool,
    /// Path keys advanced per second when the prop animates.
    pub frames_per_second: f32,
    pub parts: Vec<PropModelPart>,
    /// Each use plays the path from its start; otherwise uses alternate
    /// direction (retail frame method wrap versus ping-pong).
    pub wraps: bool,
    /// Path keys splitting the animation into segments, e.g. a dial's
    /// positions.
    pub slots: Vec<u16>,
    /// The trigger using the prop runs, by index in the level's triggers.
    pub trigger: Option<usize>,
}

impl PropModel {
    /// The highest path key of any part.
    pub fn last_frame(&self) -> usize {
        self.parts.iter().map(|p| p.keys.len().saturating_sub(1)).max().unwrap_or(0)
    }

    /// The middle of its parts at rest, where its sounds play.
    pub fn centre(&self) -> Vec3 {
        self.parts.iter().map(|part| part.transform.translation).sum::<Vec3>()
            / self.parts.len().max(1) as f32
    }
}

/// A case-insensitive location-variable name, canonicalized at the content boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VariableName(String);

impl From<&str> for VariableName {
    fn from(name: &str) -> Self {
        Self(name.to_uppercase())
    }
}

impl VariableName {
    pub fn numbered(&self, position: usize) -> Self {
        Self(format!("{}{position}", self.0))
    }
}

/// A trigger: what happens when it runs, from the player using it, another
/// trigger naming it as a recipient, or a level script.
#[derive(Clone, Debug)]
pub struct TriggerDefinition {
    pub name: String,
    /// The player may use it; recipients run whatever this says.
    pub usable: bool,
    /// It runs at most once.
    pub once: bool,
    /// Run the recipients when the action commits.
    pub fire_linked: bool,
    /// The first use runs `initial`; later uses run `alternate` if there is
    /// one, back and forth when `alternate_toggles`. When a required state
    /// is unset, `fallback` runs instead, or nothing.
    pub initial: TriggerUse,
    pub alternate: Option<TriggerUse>,
    pub fallback: Option<TriggerUse>,
    pub alternate_toggles: bool,
    /// Triggers to run, in order, by index in the level's triggers (retail
    /// looks each name up when it runs; names no trigger has are dropped).
    pub recipients: Vec<usize>,
    pub sound: Option<TriggerSound>,
    /// Commits play the sound (retail `ANIMATE_ACTION`); otherwise only
    /// actions that play it do.
    pub sound_on_commit: bool,
    /// Location variables that must all be set.
    pub required_states: Vec<VariableName>,
    /// The location variable a commit changes.
    pub state_change: Option<(VariableName, StateChange)>,
    /// The value its location variable starts with in a new game.
    pub initial_state: Option<bool>,
    /// The distance band, in metres, from which the party can use it.
    pub range: (f32, f32),
    pub door: Option<DoorLock>,
    /// A lock to pick or a trap to disarm before it runs.
    pub lock: Option<Lock>,
    /// A trigger standing at a place rather than on a prop: it runs when the
    /// party's eye comes within `range` of it.
    pub place: Option<Vec3>,
    /// The loot table a container draws its contents from, by name (the
    /// tables are their own asset; a level is played only if it exists).
    pub loot_table: Option<String>,
}

/// One of a trigger's actions, and the level message its commit shows.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerUse {
    pub action: TriggerAction,
    pub message: Option<i32>,
}

/// Retail trigger action codes and their parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerAction {
    /// Commit, doing nothing else (0).
    Nothing,
    /// Known editor placeholders: retail returns without committing an action.
    NoRuntimeAction,
    /// Open a door (1).
    OpenDoor,
    /// Open or close a door (2, and 0x2c, which retail also unlocks with a
    /// key the party carries).
    ToggleDoor,
    /// Play the prop's animation (3 toggles, 0x32 starts, 0x33 stops); a
    /// container also opens.
    Animate(Animation),
    /// Fly the party along the named camera path (9).
    CameraPath(usize),
    /// A camera path flight naming a path the level does not have (its
    /// name, empty when it names none): broken content, which refuses the
    /// level.
    MissingCameraPath(String),
    /// Run the recipients (0xf).
    Relay(i32),
    /// Distort or restore the camera according to the signed call argument (0x10).
    CameraSway,
    /// Enable, disable or toggle the linked devices (0x2d..0x2f).
    SetDevices(StateChange),
    Lights {
        sources: Vec<(String, bool)>,
        change: StateChange,
    },
    Particles {
        sources: Vec<usize>,
        change: StateChange,
    },
    AmbientSounds {
        names: Vec<String>,
        change: StateChange,
    },
    /// Send the party elsewhere (0x34).
    Teleport(Arrival),
    /// Choose one authored destination when the action executes (0x4b).
    RandomTeleport(Vec<Arrival>),
    /// Hurt everyone in the party by their own roll (0x24).
    HurtParty(Dice),
    /// Hurt everyone in the party every half second while it stays (0x23).
    DamageZone,
    Fountain(Fountain),
    Shake {
        seconds: f32,
        intensity: f32,
        fade_out: bool,
    },
    /// Advance a dial to its next position (0x40).
    Dial,
    /// Play the trigger's sound, without committing (0x36).
    Sound,
    /// A retail action not ported yet, or one missing what it needs (a
    /// teleport without a destination): running it does nothing.
    Unsupported(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Animation {
    Toggle,
    Start,
    Stop,
}

/// A fountain restoring each member (retail actions 0x25..0x27 and 0x2b).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fountain {
    pub restores: Restores,
    pub amount: u32,
    /// Uses before it runs dry; endless when none.
    pub uses: Option<u32>,
    /// Game milliseconds after it is first used that it refills.
    pub refills_after: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restores {
    Stamina,
    Health,
    Magic,
    /// Heal All at the authored power, then remove conditions.
    Restoration,
}

#[derive(Clone, Debug)]
pub struct TriggerSound {
    /// Retail path under `data/sound`.
    pub path: String,
    /// 0 to 1.
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateChange {
    Set,
    Clear,
    Toggle,
}

impl StateChange {
    pub fn apply(self, current: bool) -> bool {
        match self {
            Self::Set => true,
            Self::Clear => false,
            Self::Toggle => !current,
        }
    }
}

/// A door's lock as authored: whether it starts locked, and the key that
/// unlocks it.
#[derive(Clone, Copy, Debug)]
pub struct DoorLock {
    pub locked: bool,
    pub key: Option<ItemId>,
}

/// What stands between the party and a device as authored (retail
/// `W8LockState`): a lock to pick, or a trap to disarm. Keyed locks are
/// [`DoorLock`]s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// Its difficulty is the editor's 0 to 10 grade; it has 2 to 8 tumblers.
    Pickable { difficulty: u8 },
    /// `kind` (below [`TRAP_TYPES`]) indexes retail's trap tables; `None` rolls one from the
    /// difficulty when the party first meets it.
    Trap { difficulty: u8, kind: Option<u8> },
}

/// The trap types retail's trap tables describe.
pub const TRAP_TYPES: usize = 15;

/// One rigid part of a prop, in its rest pose.
#[derive(Clone, Debug)]
pub struct PropModelPart {
    pub transform: Transform,
    /// The part's poses along its animation path, starting with the rest
    /// pose; empty for parts that do not move.
    pub keys: Vec<Transform>,
    /// Part-local collision triangles.
    pub collision: CollisionMesh,
}

/// A level's solid collision triangles in world space (metres), with outward
/// winding. Only vertices some triangle uses are kept.
#[derive(Clone, Debug, Default)]
pub struct CollisionMesh {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Per triangle: whether walking monsters may path across it.
    pub walkable: Vec<bool>,
}

impl CollisionMesh {
    /// Append another mesh's triangles, placed by `transform`.
    pub fn append(&mut self, other: &CollisionMesh, transform: &Transform, walkable: bool) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices.iter().map(|&v| transform.transform_point(v)));
        self.triangles.extend(other.triangles.iter().map(|t| t.map(|v| base + v)));
        self.walkable.extend(std::iter::repeat_n(walkable, other.triangles.len()));
    }
}
