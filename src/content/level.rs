//! A level's static geometry as native Bevy assets.

use bevy::prelude::*;
use num_enum::{IntoPrimitive, TryFromPrimitive};

/// One renderable piece of the static level: a compiled octree mesh's
/// polygons that share a material.
#[derive(Clone, Debug, Reflect)]
pub struct LevelPart {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    /// The inputs of the mesh's vertex colours, per vertex.
    pub lighting: VertexLighting,
}

/// Retail bakes lighting into vertex colours and redoes it when the time of
/// day changes the sky light: colour = sky x reflectance x sun + fixed.
#[derive(Clone, Debug, Default, Reflect)]
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

/// A level's fog and sky light (retail "fog options").
#[derive(Clone, Debug, Reflect)]
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
#[derive(Clone, Debug, Default, Reflect)]
pub struct LevelLighting {
    /// Added to everything's light (the level's environment offset).
    pub offset: Vec3,
    pub lights: Vec<PointLightSource>,
    /// The `SUN` light's position: actors in its line of sight get sky light.
    pub sun: Option<Vec3>,
}

/// A static level light with retail's linear falloff.
#[derive(Clone, Copy, Debug, Reflect)]
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

/// Immutable static geometry shared by every visit to a level.
#[derive(Asset, Clone, Debug, Default, Reflect)]
pub struct LevelGeometry {
    pub parts: Vec<LevelPart>,
    pub collision: CollisionMesh,
    /// Automap ceiling cuts in authored order, top to bottom. Layer zero is uncut.
    pub map_layers: Vec<f32>,
    pub environment: Option<LevelEnvironment>,
    /// Light that reaches props and actors at runtime, besides the sky.
    pub lighting: LevelLighting,
    /// Positioned triggers, where the party can enter.
    pub entrances: Vec<Entrance>,
    /// Every trigger the party or props can run.
    pub triggers: Vec<TriggerDefinition>,
    /// Paths the party's eye can fly along (ladders, climbs, cutscenes).
    pub camera_paths: Vec<CameraPathModel>,
    /// Sounds places make: water, wind, birds.
    pub ambient_sounds: Vec<AmbientSoundModel>,
    /// Monsters and NPCs standing in the level at a new game.
    pub monsters: Vec<MonsterPlacement>,
    pub items: Vec<ItemPlacement>,
    pub props: Vec<PropModel>,
    /// Navigation links between places the navmesh does not join.
    pub links: Vec<LevelLink>,
    /// Materials whose texture is an animated frame list (`.ifl`).
    pub animated_textures: Vec<TextureAnimation>,
}

/// How a walker gets from one end of a link to the other.
/// Its number is its Landmass link kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, IntoPrimitive, TryFromPrimitive)]
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
#[derive(Clone, Copy, Debug, Reflect)]
pub struct LevelLink {
    pub kind: LinkKind,
    pub from: Vec3,
    pub to: Vec3,
}

/// A monster placed in the level (retail `.sts` `MONS`).
#[derive(Clone, Debug, Reflect)]
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
#[derive(Clone, Debug, Reflect)]
pub struct ItemPlacement {
    pub item: crate::content::items::ItemId,
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
#[derive(Clone, Debug, Reflect)]
pub struct AmbientSoundModel {
    /// Retail path under `data/sound`.
    pub path: String,
    pub position: Vec3,
    /// Metres; heard fully within 40% of it, fading to nothing at its edge.
    pub radius: f32,
    pub looping: bool,
    /// Loudness range, 0 to 1.
    pub volume: (f32, f32),
    /// Seconds between plays of a one-shot sound.
    pub interval: (f32, f32),
}

/// A path the party's eye flies along: eye position and view per key.
#[derive(Clone, Debug, Reflect)]
pub struct CameraPathModel {
    pub name: String,
    pub keys: Vec<Transform>,
    pub keys_per_second: f32,
}

/// Where a teleport takes the party.
#[derive(Clone, Debug, Reflect)]
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
#[derive(Clone, Debug, Reflect)]
pub struct Entrance {
    pub name: String,
    /// Retail places the eye 1000 units above the trigger and lets the party
    /// settle.
    pub eye: Vec3,
    pub yaw: f32,
}

/// A placed prop: scenery, a door, a lever, a lift or a billboard.
#[derive(Clone, Debug, Reflect)]
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
    /// The name of the trigger using the prop runs.
    pub trigger: Option<String>,
}

/// A trigger: what happens when it runs, from the player using it, another
/// trigger naming it as a recipient, or a level script.
#[derive(Clone, Debug, Reflect)]
pub struct TriggerDefinition {
    pub name: String,
    /// The player may use it; recipients run whatever this says.
    pub usable: bool,
    /// It runs at most once.
    pub once: bool,
    /// Run the recipients when the action commits.
    pub fire_linked: bool,
    /// Retail action codes. The first use runs `initial`; later uses run
    /// `alternate` if set, back and forth when `alternate_toggles`. When a
    /// required state is unset, `fallback` runs instead (0: nothing).
    pub initial: u32,
    pub alternate: u32,
    pub fallback: u32,
    pub alternate_toggles: bool,
    /// Triggers to run, in order.
    pub recipients: Vec<String>,
    /// Action parameters; their meaning depends on the action.
    pub data: [i32; 3],
    pub sound: Option<TriggerSound>,
    /// Commits play the sound (retail `ANIMATE_ACTION`); otherwise only
    /// actions that play it do.
    pub sound_on_commit: bool,
    /// Show level message `data[use]` on commit.
    pub shows_message: bool,
    /// Location variables that must all be set.
    pub required_states: Vec<String>,
    /// The location variable a commit changes.
    pub state_change: Option<(String, StateChange)>,
    /// The value its location variable starts with in a new game.
    pub initial_state: Option<bool>,
    /// The distance band, in metres, from which the party can use it.
    pub range: (f32, f32),
    pub door: Option<DoorLock>,
    /// A trigger standing at a place rather than on a prop: it runs when the
    /// party's eye comes within `range` of it.
    pub place: Option<Vec3>,
    /// Where a teleport takes the party.
    pub arrival: Option<Arrival>,
    /// The loot table a container draws its contents from.
    pub loot_table: Option<String>,
}

#[derive(Clone, Debug, Reflect)]
pub struct TriggerSound {
    /// Retail path under `data/sound`.
    pub path: String,
    /// 0 to 1.
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum StateChange {
    Set,
    Clear,
    Toggle,
}

/// A door's lock state.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct DoorLock {
    pub locked: bool,
    /// The key item that unlocks it.
    pub key: Option<u16>,
}

/// One rigid part of a prop, in its rest pose.
#[derive(Clone, Debug, Reflect)]
pub struct PropModelPart {
    pub transform: Transform,
    /// The part's poses along its animation path, starting with the rest
    /// pose; empty for parts that do not move.
    pub keys: Vec<Transform>,
    pub meshes: Vec<LevelPart>,
    /// Part-local collision triangles.
    pub collision: CollisionMesh,
}

/// A level's solid collision triangles in world space (metres), with outward
/// winding. Only vertices some triangle uses are kept.
#[derive(Clone, Debug, Default, Reflect)]
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

/// How an animated texture advances (retail `W8TextureAnimationMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum Playback {
    Loop,
    PingPong,
    Once,
    /// Never advances by itself; driven by game state (e.g. the sky by time
    /// of day).
    Manual,
}

/// A material showing one frame of a frame list at a time.
#[derive(Clone, Debug, Reflect)]
pub struct TextureAnimation {
    pub material: Handle<StandardMaterial>,
    pub frames: Vec<Handle<Image>>,
    pub playback: Playback,
    pub frames_per_second: f32,
    pub start_frame: usize,
}

impl TextureAnimation {
    /// The frame shown `seconds` after the animation started (retail
    /// `Texture::UpdateFrame`).
    pub fn frame_at(&self, seconds: f32) -> usize {
        let count = self.frames.len();
        if count == 0 {
            return 0;
        }
        let steps = (seconds.max(0.0) * self.frames_per_second) as usize;
        let start = self.start_frame.min(count - 1);
        match self.playback {
            Playback::Manual => start,
            Playback::Loop => (start + steps) % count,
            Playback::Once => (start + steps).min(count - 1),
            Playback::PingPong => {
                let step = (start + steps) % count;
                if ((start + steps) / count).is_multiple_of(2) { step } else { count - 1 - step }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(playback: Playback) -> TextureAnimation {
        TextureAnimation {
            material: default(),
            frames: vec![default(); 4],
            playback,
            frames_per_second: 2.0,
            start_frame: 0,
        }
    }

    #[test]
    fn frames_follow_the_retail_playback_modes() {
        let at = |p, s| animation(p).frame_at(s);
        assert_eq!([0.0, 0.5, 1.9, 2.0].map(|s| at(Playback::Loop, s)), [0, 1, 3, 0]);
        assert_eq!([2.0, 2.5, 3.5].map(|s| at(Playback::PingPong, s)), [3, 2, 0]);
        assert_eq!(at(Playback::Once, 10.0), 3);
        assert_eq!(at(Playback::Manual, 10.0), 0);
    }
}
