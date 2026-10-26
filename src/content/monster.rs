//! Monster models: animation cycles as meshes with per-frame vertex positions.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// A monster species' visual cycles. Gameplay dimensions load independently.
#[derive(Asset, TypePath, Debug)]
pub struct MonsterModel {
    /// Cycles by retail name (`IDLE`, `WALK`, `DIE`, ...).
    pub cycles: HashMap<String, MonsterCycleModel>,
}

/// A species' body measurements (`.mls` keywords). Lengths are metres and
/// speeds metres per second before the monster's scale, as retail's
/// navigator scales them (`Navigator::SetScale`).
#[derive(Asset, TypePath, Clone, Copy, Debug, PartialEq)]
pub struct MonsterBody {
    /// The scale without a range, or the middle of the range.
    pub scale: f32,
    /// Each monster's scale is rolled within this (retail
    /// `RandomizeAppearanceAndMotion`).
    pub scale_range: Option<(f32, f32)>,
    /// The body's radius outside combat (`walkradius`).
    pub walk_radius: f32,
    /// The body's radius in combat, and what reach is measured from
    /// (`fightradius`, retail's alternate radius).
    pub fight_radius: f32,
    /// Its eyes and where it is aimed at, above its feet (`targetheight`).
    pub target_height: f32,
    pub movement_rate: f32,
}

impl Default for MonsterBody {
    /// Retail's navigator defaults: 500 world units, one metre.
    fn default() -> Self {
        MonsterBody {
            scale: 1.0,
            scale_range: None,
            walk_radius: 1.0,
            fight_radius: 1.0,
            target_height: 1.0,
            movement_rate: 3.0,
        }
    }
}

impl MonsterBody {
    /// One monster's scale from its roll below 1000.
    pub fn scale_for(&self, roll: u32) -> f32 {
        match self.scale_range {
            Some((low, high)) => (high - low) * roll as f32 / 1000.0 + low,
            None => self.scale,
        }
    }
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
    /// The material's ambient reflectance, which scales sky light.
    pub ambient: Vec3,
    /// Light the material gives off itself.
    pub emission: f32,
    /// The source vertex of each render vertex.
    pub source: Vec<u32>,
}
