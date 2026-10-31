//! Shared, stateless sight checks. Profiles, effects, physical poses and
//! scenery remain authoritative; no second current-visibility table is kept.
use crate::{
    content::{
        characters::{Attribute, Skill},
        species::Species,
    },
    game::{
        characters::{Character, Condition, Profession, Vitals},
        world::{obstacles::WorldObstacles, population::Monster},
    },
};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub struct View {
    pub eye: Vec3,
    pub forward: Vec3,
    pub far: f32,
    pub searching: bool,
}
/// How far a monster standing at `feet` is from the party (retail
/// `GetPointDistanceToPlayer`): from its feet to the party's point, less
/// both bodies' radii (its fight radius, the party's 1 m), never below 0.
/// Reach and sight compare against this.
pub fn party_distance(eye: Vec3, monster: &Monster, feet: Vec3) -> f32 {
    (feet.distance(crate::game::party::body::PartyBody::feet_below(eye))
        - monster.body.radius
        - crate::game::party::body::RADIUS)
        .max(0.0)
}

/// Whether the party has a clear line to a monster (retail
/// `GetPlayerToMonsterSightFlags`): within the far clip, nothing solid
/// between the party's eye and the monster's. The party's line of sight,
/// whichever way it faces.
pub fn in_view(
    (eye, far): (Vec3, f32),
    monster: &Monster,
    feet: Vec3,
    obstacles: &WorldObstacles,
) -> bool {
    party_distance(eye, monster, feet) <= far && obstacles.clear(eye, monster.eye(feet))
}

impl View {
    pub fn in_range(self, monster: &Monster, feet: Vec3) -> bool {
        !monster.is_dead() && party_distance(self.eye, monster, feet) <= self.far
    }
    /// Retail party-side sight. Shadow Hound supplies the full-range override,
    /// while blindness, the target's concealment and solid scenery still apply.
    pub fn sees<'a>(
        self,
        mut members: impl Iterator<Item = (&'a Character, &'a Vitals)>,
        shadow_hound: bool,
        monster: &Monster,
        species: &Species,
        feet: Vec3,
        in_combat: bool,
        obstacles: &WorldObstacles,
    ) -> bool {
        if monster.is_dead() || !in_view((self.eye, self.far), monster, feet, obstacles) {
            return false;
        }
        let distance = party_distance(self.eye, monster, feet);
        let angle = if shadow_hound {
            0.0
        } else {
            self.forward.with_y(0.0).angle_between((feet - self.eye).with_y(0.0))
        };
        let concealment = if in_combat { 0 } else { u32::from(species.camouflage) };
        members.any(|(profile, vitals)| {
            if !vitals.can_fight()
                || vitals.conditions.has(Condition::Blind) && profile.profession != Profession::Monk
            {
                return false;
            }
            let stats = profile.stats(&vitals.enchantments);
            sight_threshold(
                angle,
                stats.attribute(Attribute::Senses) as i32,
                stats.skill(Skill::Scouting),
                i32::from(species.level),
                concealment,
                self.far,
                distance,
            ) * if self.searching { 0.6667 } else { 1.0 }
                >= distance
        })
    }
}

/// Retail `ComputeSightThreshold`: how far an observer facing away by
/// `angle` (radians) sees, given its senses, a ranged bonus percent and the
/// penalty source (the party's lowest level, doubled); within 45 degrees, or
/// at no distance, the whole `far` clip.
pub(crate) fn sight_threshold(
    angle: f32,
    senses: i32,
    bonus: u32,
    penalty: i32,
    concealment: u32,
    far: f32,
    distance: f32,
) -> f32 {
    let mut percent = if angle < std::f32::consts::FRAC_PI_4 || distance == 0.0 {
        100
    } else {
        ((senses - penalty * 2) as f32 * ((far - distance) / distance)) as i32
    };
    percent -= concealment as i32 * 10;
    if bonus != 0 {
        percent = (bonus as i32 + 100) * percent / 100;
    }
    percent.clamp(2, 100) as f32 * far * 0.01
}
