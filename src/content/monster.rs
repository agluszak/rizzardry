//! Monster models: animation cycles as meshes with per-frame vertex positions.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// A monster species' look and body (retail `.mls` and its `.mon` cycles).
#[derive(Asset, TypePath, Debug)]
pub struct MonsterModel {
    pub scale: f32,
    /// Collision radius in metres, before `scale`.
    pub walk_radius: f32,
    /// Metres per second, before `scale`.
    pub movement_rate: f32,
    /// Cycles by retail name (`IDLE`, `WALK`, `DIE`, ...).
    pub cycles: HashMap<String, MonsterCycleModel>,
}

impl MonsterModel {
    /// A cycle, or IDLE in its place as retail falls back.
    pub fn cycle(&self, name: &str) -> Option<&MonsterCycleModel> {
        self.cycles.get(name).or_else(|| self.cycles.get("IDLE"))
    }
}

/// One cycle: render meshes whose vertices follow the frames.
#[derive(Debug)]
pub struct MonsterCycleModel {
    pub frames_per_second: f32,
    pub parts: Vec<MonsterPart>,
    /// Model-space positions (metres, facing +Z) per frame, per source
    /// vertex.
    pub frames: Vec<Vec<Vec3>>,
}

impl MonsterCycleModel {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// The source-vertex positions at a fractional frame. Retail blends
    /// toward the next frame but never from the last back to the first.
    pub fn positions_at(&self, frame: f32) -> Vec<Vec3> {
        let last = self.frames.len() - 1;
        let frame = frame.clamp(0.0, last as f32);
        let (index, t) = (frame.floor() as usize, frame.fract());
        if index >= last || t == 0.0 {
            return self.frames[index.min(last)].clone();
        }
        self.frames[index].iter().zip(&self.frames[index + 1]).map(|(a, b)| a.lerp(*b, t)).collect()
    }
}

/// One material's faces of a cycle.
#[derive(Debug)]
pub struct MonsterPart {
    /// Positioned at frame 0; instances animate their own copies.
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    /// The source vertex of each render vertex.
    pub source: Vec<u32>,
}
