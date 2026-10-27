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

/// A static level light with retail's linear falloff.
#[derive(Clone, Copy, Debug)]
pub struct PointLightSource {
    pub position: Vec3,
    /// Colour times intensity.
    pub colour: Vec3,
    /// Metres.
    pub range: f32,
}

impl LevelLighting {
    /// Light at a point from the offset and every light in range:
    /// `colour * (1 - d / range) * facing`, with facing 1 when `normal` is
    /// none.
    pub fn at(&self, position: Vec3, normal: Option<Vec3>) -> Vec3 {
        let mut light = self.offset;
        for source in &self.lights {
            let to_light = source.position - position;
            let distance = to_light.length();
            if distance >= source.range || distance == 0.0 {
                continue;
            }
            let facing = normal.map_or(1.0, |n| n.dot(to_light / distance).max(0.0));
            light += source.colour * (1.0 - distance / source.range) * facing;
        }
        light
    }
}

/// Immutable gameplay content. Loading it never constructs rendering assets.
#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct LevelDefinition {
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
}

impl LevelDefinition {
    pub fn view_distance(&self) -> f32 {
        self.sight_distance.unwrap_or(DEFAULT_VIEW_DISTANCE)
    }
}

/// Optional presentation content loaded separately from the same retail source.
#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct LevelGeometry {
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

/// A case-insensitive location-variable name, canonicalized at the content boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
    /// Run the recipients (0xf).
    Relay,
    /// Send the party elsewhere (0x34, and 0x4b, whose random choice is
    /// not ported).
    Teleport(Arrival),
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

/// A fountain restoring each member (retail actions 0x25 and 0x26).
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

/// A door's lock as authored: whether it starts locked, and the key that
/// unlocks it.
#[derive(Clone, Copy, Debug)]
pub struct DoorLock {
    pub locked: bool,
    pub key: Option<ItemId>,
}

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
