//! Retail world space to Bevy world space.
//!
//! Retail geometry is Y-up and left-handed; Bevy is Y-up and right-handed.
//! Mirroring Z converts between them; triangle winding must then be reversed
//! to keep front faces. Lengths become metres.

use bevy::math::Vec3;

use crate::content::UNITS_PER_METRE;

pub fn retail_position([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(x, y, -z) / UNITS_PER_METRE
}

pub fn retail_direction([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(x, y, -z)
}

/// A retail rotation of `angle` radians about `axis`, in Bevy space.
pub fn retail_rotation(angle: f32, axis: [f32; 3]) -> bevy::math::Quat {
    let axis = retail_direction(axis).normalize_or(Vec3::Y);
    // Mirroring Z reverses the sense of rotation.
    bevy::math::Quat::from_axis_angle(axis, -angle)
}

/// Bevy yaw (about +Y, zero facing -Z) of a retail rotation of `angle`
/// radians about `axis`, applied to retail's forward (+Z).
pub fn retail_yaw(angle: f32, axis: [f32; 3]) -> f32 {
    let forward = retail_rotation(angle, axis) * Vec3::NEG_Z;
    (-forward.x).atan2(-forward.z)
}
