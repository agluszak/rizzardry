//! Whom a cast reaches (`docs/rules/casting.md` section 5): the victims of a
//! spell by its kind of target, where it lands, what a backfire turns it on,
//! when a missile arrives.

use bevy::prelude::*;

use crate::content::spells::SpellDefinition;

/// How fast spell missiles fly: retail's `energy_blast.mls` velocity of 30,
/// advanced per two-millisecond step (`Missile.cpp`), comes to 15 m/s.
pub const MISSILE_SPEED: f32 = 15.0;

/// A cone is inside when the heading and the elevation toward it each differ
/// from those toward the aimed spot by less than this (retail
/// `TargetInRangeAndArcs`).
const CONE_HALF_ARC: f32 = std::f32::consts::FRAC_PI_4;

/// Whom the caster aimed at; group, area, party and self spells widen or
/// replace it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Point(Vec3),
    /// A body, a member's or a monster's.
    Body(Entity),
    /// Whoever stands nearest the caster's heading within the spell's
    /// reach when it is cast (retail's automatic targeting), the nearer of
    /// equals.
    Ahead,
    /// The device attempt under way (its lock or trap), by the attempt.
    Device(Entity),
    /// The selected item, retained across any delayed request.
    Item(Entity),
}

impl Aim {
    /// The body aimed at, if one was.
    pub fn body(self) -> Option<Entity> {
        match self {
            Aim::Body(entity) => Some(entity),
            _ => None,
        }
    }
}

/// Select an available target by angle to the heading, then distance.
pub fn ahead(
    eye: Vec3,
    forward: Vec3,
    reach: f32,
    candidates: impl IntoIterator<Item = (Entity, Vec3, f32)>,
) -> Option<Entity> {
    candidates
        .into_iter()
        .filter(|(_, _, distance)| *distance <= reach)
        .map(|(entity, at, distance)| {
            (entity, (at - eye).with_y(0.0).angle_between(forward.with_y(0.0)), distance)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(entity, ..)| entity)
}

/// Whether `target` lies within a cone's arcs along `aim`, both directions
/// from its apex (retail `TargetInRangeAndArcs`, which tests heading and
/// elevation independently against a 45 degree half arc). Range is the
/// caller's.
pub fn in_cone(aim: Vec3, target: Vec3) -> bool {
    let heading = |v: Vec3| v.x.atan2(v.z);
    let elevation = |v: Vec3| v.y.atan2(v.x.hypot(v.z));
    let within = |a: f32, b: f32| {
        let difference = a - b;
        difference.sin().atan2(difference.cos()).abs() < CONE_HALF_ARC
    };
    within(heading(aim), heading(target)) && within(elevation(aim), elevation(target))
}

/// The metres a blast spreads from where it lands: the spell's radius and its
/// radius per power times the power.
pub fn blast_radius(spell: &SpellDefinition, power: u32) -> f32 {
    spell.radius + spell.radius_per_power * power as f32
}

/// The seconds a missile takes from where the cast set out to where it lands.
pub fn flight(from: Vec3, to: Vec3) -> f32 {
    to.distance(from) / MISSILE_SPEED
}

/// What a utility spell takes as its target (`docs/rules/casting.md` section
/// 8): nothing, a spot, an item, a body, or the attempt under way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Marked {
    Nothing,
    Spot(Vec3),
    Item(Entity),
    Body(Entity),
    Attempt(Entity),
}
