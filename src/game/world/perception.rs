//! Shared, stateless sight checks. Profiles, effects, physical poses and
//! scenery remain authoritative; no second current-visibility table is kept.
use crate::{
    content::characters::{Attribute, Skill, Trait},
    game::{
        actors::Level,
        characters::{Character, Condition, Vitals},
        combat::monster::noticing::{Noticing, notices},
        world::{
            obstacles::WorldObstacles,
            population::{Camouflage, Monster},
        },
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
    pub fn in_range(self, (monster, vitals): (&Monster, &Vitals), feet: Vec3) -> bool {
        !vitals.is_dead() && party_distance(self.eye, monster, feet) <= self.far
    }
    /// Whether the party makes out a monster ([`notices`]): it hides by its
    /// level and, outside a fight, by its kind's camouflage. Shadow Hound
    /// supplies the full-range override, while blindness, the target's
    /// concealment and solid scenery still apply.
    pub fn sees<'a>(
        self,
        mut members: impl Iterator<
            Item = (&'a Character, &'a Vitals, &'a crate::game::actors::Attributes),
        >,
        shadow_hound: bool,
        (monster, body): (&Monster, &Vitals),
        (level, camouflage): (Level, Option<Camouflage>),
        feet: Vec3,
        in_combat: bool,
        obstacles: &WorldObstacles,
    ) -> bool {
        if body.is_dead() || !in_view((self.eye, self.far), monster, feet, obstacles) {
            return false;
        }
        let angle = if shadow_hound {
            0.0
        } else {
            self.forward.with_y(0.0).angle_between((feet - self.eye).with_y(0.0))
        };
        let concealment = if in_combat { 0 } else { camouflage.map_or(0, |c| u32::from(c.0)) };
        let gap = party_distance(self.eye, monster, feet);
        let level = level.0 as i32;
        members.any(|(profile, vitals, attributes)| {
            notices(&Noticing {
                open: vitals.can_fight()
                    && !(vitals.conditions.has(Condition::Blind)
                        && !profile.has_trait(Trait::EffectiveWhileBlind)),
                line: true,
                senses: attributes[Attribute::Senses] as i32,
                bonus: profile.skill(Skill::Scouting),
                angle,
                gap,
                hider_level: level,
                concealment,
                far: self.far,
                searching: self.searching,
            })
        })
    }
}
