//! A level's static geometry as native Bevy assets.

use bevy::prelude::*;

/// One renderable piece of the static level: a compiled octree mesh's
/// polygons that share a material.
#[derive(Clone, Debug, Reflect)]
pub struct LevelPart {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// Immutable static geometry shared by every visit to a level.
#[derive(Asset, Clone, Debug, Reflect)]
pub struct LevelGeometry {
    pub parts: Vec<LevelPart>,
    pub collision: CollisionMesh,
    pub props: Vec<PropModel>,
    /// Materials whose texture is an animated frame list (`.ifl`).
    pub animated_textures: Vec<TextureAnimation>,
}

/// A placed prop: scenery, a door, a lever, a lift or a billboard.
#[derive(Clone, Debug, Reflect)]
pub struct PropModel {
    pub name: String,
    /// Retail `COLLIDABLE`: the party and monsters collide with its parts.
    pub collidable: bool,
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
