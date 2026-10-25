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
    /// Solid triangles in world space (metres), for building the collider.
    pub collision_vertices: Vec<Vec3>,
    pub collision_triangles: Vec<[u32; 3]>,
    pub bounds: (Vec3, Vec3),
    /// Materials whose texture is an animated frame list (`.ifl`).
    pub animated_textures: Vec<TextureAnimation>,
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
