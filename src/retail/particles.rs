//! `ReadWorldParticles` / `W8LevelParticleRecord`, versions 1 through 4.
use super::{
    pvl::{MaterialRecord, material},
    space::{retail_position, retail_rotation},
};
use crate::{
    bytes::{Bytes, DecodeError},
    content::{UNITS_PER_METRE, particles::*},
};
use bevy::prelude::*;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub config: ParticleConfig,
    pub texture_expiry: bool,
    pub material: MaterialRecord,
}

pub fn read_particle(bytes: &mut Bytes) -> Result<Particle, DecodeError> {
    let version = bytes.u8()?;
    if !(1..=4).contains(&version) {
        return Err(bytes.invalid("unknown particle system version"));
    }
    let name = super::pvl::text(bytes, 64)?.to_uppercase();
    let location = bytes.f32s::<3>()?.map(|v| v * UNITS_PER_METRE);
    let angle = bytes.f32()?;
    let axis = bytes.f32s()?;
    let rotation = retail_rotation(angle, axis);
    bytes.take(12)?; // Positional record, unused by ReadWorldParticles.
    let capacity = bytes.u32()?;
    let spread = Vec3::from(bytes.f32s::<3>()?);
    let has_acceleration = bytes.i32()? != 0;
    let acceleration = Vec3::from(bytes.f32s::<3>()?) * Vec3::new(1.0, 1.0, -1.0);
    let expires_with_texture = bytes.i32()? != 0;
    let bounds_mode = bytes.i32()?;
    let mut origin = Vec3::from(bytes.f32s::<3>()?);
    let radius = bytes.f32()?;
    let extent = Vec3::from(bytes.f32s::<3>()?) * 0.5;
    let lifetime = Duration::from_millis(u64::from(bytes.u32()?));
    let emission_mode = bytes.i32()?;
    let interval = Duration::from_millis(u64::from(bytes.u32()?.max(1)));
    let stops_at_scenery = bytes.i32()? != 0;
    let speed_mode = bytes.i32()?;
    let speed = bytes.f32()?;
    let minimum = bytes.f32()?;
    let maximum = bytes.f32()?;
    let size = bytes.f32()? / UNITS_PER_METRE;
    let flutter_mode = bytes.i32()?;
    let flutter_amplitude = bytes.f32()?;
    let flutter_period = bytes.f32()?;
    let direction_mode = bytes.i32()?;
    let yaw = bytes.f32()?.to_radians();
    let pitch = bytes.f32()?.to_radians();
    let initially_active = bytes.i32()? != 0 || name.starts_with("CLOUD");
    let material = material(bytes, true)?;
    let attachment = if version > 1 { bytes.i16()? } else { -1 };
    let emission_limit = if version > 2 { bytes.u32()? } else { 0 };
    if version > 2 {
        bytes.u8()?;
    } // Sorted rendering follows the material's native alpha mode.
    let frames = if version > 3 { Some((bytes.i32()?, bytes.i32()?)) } else { None };
    let replace_when_full = origin.x != 0.0;
    origin.x = 0.0;
    origin.z = -origin.z;
    let bounds = match bounds_mode {
        1 => ParticleBounds::Box { minimum: origin - extent, maximum: origin + extent },
        2 if radius > 0.0 => ParticleBounds::Sphere { centre: origin, radius },
        _ => ParticleBounds::None,
    };
    let direction = match direction_mode {
        0 => ParticleDirection::Stationary,
        1 => ParticleDirection::Fixed(rotation * Vec3::NEG_Y),
        2 => ParticleDirection::Forward,
        3 => ParticleDirection::Cone { yaw, pitch },
        _ => ParticleDirection::Random,
    };
    let emission = match emission_mode {
        0 => ParticleEmission::None,
        1 => ParticleEmission::Single,
        _ => ParticleEmission::CatchUp {
            minimum: Vec3::new(-spread.x * 0.5, -spread.y * 0.5, -spread.z),
            maximum: Vec3::new(spread.x * 0.5, spread.y * 0.5, 0.0),
        },
    };
    Ok(Particle {
        material,
        texture_expiry: expires_with_texture,
        config: ParticleConfig {
            name,
            transform: Transform::from_translation(retail_position(location))
                .with_rotation(rotation * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            capacity,
            replace_when_full,
            bounds,
            emission,
            interval,
            direction,
            speed: match speed_mode {
                0 => ParticleSpeed::Zero,
                1 => ParticleSpeed::Fixed(speed),
                _ => ParticleSpeed::Random { minimum, maximum },
            },
            acceleration: if has_acceleration { acceleration } else { Vec3::ZERO },
            size,
            lifetime: ParticleLifetime::Timed(lifetime),
            stops_at_scenery,
            flutter: (flutter_mode != 0).then_some((
                flutter_amplitude,
                Duration::from_millis(flutter_period.max(0.0) as u64),
            )),
            initially_active,
            emission_limit,
            attachment: u16::try_from(attachment).ok(),
            frames: frames.and_then(|(start, end)| {
                Some((usize::try_from(start).ok()?, usize::try_from(end).ok()?))
            }),
        },
    })
}
