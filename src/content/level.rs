//! A level's static geometry as native Bevy assets.

use bevy::prelude::*;

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
#[derive(Asset, Clone, Debug, Reflect)]
pub struct LevelGeometry {
    pub parts: Vec<LevelPart>,
    pub collision: CollisionMesh,
    /// Automap ceiling cuts in authored order, top to bottom. Layer zero is uncut.
    pub map_layers: Vec<f32>,
    pub environment: Option<LevelEnvironment>,
    /// Light that reaches props and actors at runtime, besides the sky.
    pub lighting: LevelLighting,
    pub props: Vec<PropModel>,
    /// Navigation links between places the navmesh does not join.
    pub links: Vec<LevelLink>,
    /// Positioned triggers that move the party elsewhere in the level.
    pub teleports: Vec<TeleportTrigger>,
    /// Materials whose texture is an animated frame list (`.ifl`).
    pub animated_textures: Vec<TextureAnimation>,
}

/// How a walker gets from one end of a link to the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum LinkKind {
    /// A teleport pad or portal: arrive at once.
    Teleport,
    /// A ladder or rope: retail flies the party along a camera path.
    Climb,
    /// Walk through an open door.
    Door,
}

impl LinkKind {
    /// Landmass link kind numbers.
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(index: usize) -> Option<Self> {
        [LinkKind::Teleport, LinkKind::Climb, LinkKind::Door].get(index).copied()
    }
}

/// A one-way link between two ground positions.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct LevelLink {
    pub kind: LinkKind,
    pub from: Vec3,
    pub to: Vec3,
}

/// A trigger that teleports the party within the level when it comes within
/// range (retail action 0x34 naming a trigger in the same level).
#[derive(Clone, Debug, Reflect)]
pub struct TeleportTrigger {
    pub name: String,
    pub centre: Vec3,
    /// Fires at distances in `[range.0, range.1)` metres.
    pub range: (f32, f32),
    /// Where the party's eye arrives, and its facing.
    pub destination: Vec3,
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
    /// What using the prop does, from its attached trigger.
    pub interaction: Option<PropInteraction>,
}

/// The attached trigger of a usable prop.
#[derive(Clone, Debug, Reflect)]
pub struct PropInteraction {
    pub trigger: String,
    /// Retail action codes: the first use runs `initial_action`, later uses
    /// `alternate_action` when it is set.
    pub initial_action: u32,
    pub alternate_action: u32,
    pub door: Option<DoorLock>,
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
