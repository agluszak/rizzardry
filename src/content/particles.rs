//! Authored emitter definitions; activation belongs to the played level.
use bevy::prelude::*;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum ParticleBounds {
    None,
    Box { minimum: Vec3, maximum: Vec3 },
    Sphere { centre: Vec3, radius: f32 },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleEmission {
    None,
    Single,
    CatchUp { minimum: Vec3, maximum: Vec3 },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleDirection {
    Stationary,
    Fixed(Vec3),
    Forward,
    Cone { yaw: f32, pitch: f32 },
    Random,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleSpeed {
    Zero,
    Fixed(f32),
    Random { minimum: f32, maximum: f32 },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleLifetime {
    Timed(Duration),
    TextureLoop,
    TextureOnce(Duration),
}
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleConfig {
    pub name: String,
    pub transform: Transform,
    pub capacity: u32,
    pub replace_when_full: bool,
    pub bounds: ParticleBounds,
    pub emission: ParticleEmission,
    pub interval: Duration,
    pub direction: ParticleDirection,
    pub speed: ParticleSpeed,
    pub acceleration: Vec3,
    pub size: f32,
    pub lifetime: ParticleLifetime,
    pub stops_at_scenery: bool,
    pub flutter: Option<(f32, Duration)>,
    pub initially_active: bool,
    pub emission_limit: u32,
    pub attachment: Option<u16>,
    pub frames: Option<(usize, usize)>,
}
