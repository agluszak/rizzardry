//! Where each body stands, and how bodies see one another: the same
//! components on a party member and a monster, so the rules that ask who
//! reaches or sees whom read them alike. Each is produced from its body's
//! own sources by one system per kind (a member's from the party's body and
//! spells, a monster's from its pose, species and vitals), the way
//! `GlobalTransform` follows `Transform`, before anything reads it: at the
//! start of each frame's gameplay and of each fixed step. Physics stays the
//! authority on position; nothing else writes these.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::ActorPose;
use super::navigation::{Walker, feet};
use super::obstacles::WorldObstacles;
use super::perception::sight_threshold;
use super::population::{Monster, blinded};
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::magic::enchantments::PartyEffects;
use crate::game::party::body::{self as party_body, PartyBody};
use crate::game::party::formation::{self, Row};
use crate::game::party::roster::PartyRoster;

/// Where a body stands and how big it is: a monster at its pose, a member
/// in the party's body, which they share.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Footing {
    pub feet: Vec3,
    /// Where it looks from.
    pub eye: Vec3,
    /// Where a blow or a missile strikes it.
    pub centre: Vec3,
    pub radius: f32,
    pub half_height: f32,
    /// Which way it faces.
    pub yaw: f32,
}

impl Footing {
    pub fn forward(&self) -> Vec3 {
        party_body::heading(self.yaw)
    }

    /// How far it is from another body, body to body.
    pub fn gap(&self, other: &Footing) -> f32 {
        (self.feet.distance(other.feet) - self.radius - other.radius).max(0.0)
    }

    /// The side of it something at `at` is on.
    pub fn quadrant(&self, at: Vec3) -> Row {
        formation::quadrant(self.eye, self.yaw, at)
    }

    /// Whether the body has a clear line to another within the far clip,
    /// whichever way that one faces (retail `party_threat.los_flags`).
    pub fn in_view(&self, other: &Footing, far: f32, obstacles: &WorldObstacles) -> bool {
        self.gap(other) <= far && obstacles.clear(self.eye, other.eye)
    }
}

/// How a monster looks about it (retail `CanMonsterSeeMonster`,
/// `UpdateMonsterSight`): its species' senses and, for a species whose
/// sight grows with its level, that bonus; shut when it cannot fight or is
/// blinded. Members do not look out this way.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Eyes {
    senses: i32,
    bonus: u32,
    open: bool,
}

/// How hard a body is to make out from behind: a monster by its level, a
/// member by the party's lowest level among those who can act and by its
/// concealment.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Conspicuous {
    level: i32,
    concealment: u32,
}

impl Eyes {
    /// Whether these eyes, at `from`, see a body at `to`: within the far
    /// clip, a clear line to its eyes, and the sight threshold, narrower
    /// from behind by its level and concealment, covering the gap.
    pub fn see(
        &self,
        from: &Footing,
        (to, conspicuous): (&Footing, &Conspicuous),
        far: f32,
        obstacles: &WorldObstacles,
    ) -> bool {
        let distance = from.gap(to);
        if !self.open || distance > far {
            return false;
        }
        let forward = from.forward().with_y(0.0);
        let angle = forward.angle_between((to.eye - from.eye).with_y(0.0));
        let threshold = sight_threshold(
            angle,
            self.senses,
            self.bonus,
            conspicuous.level,
            conspicuous.concealment,
            far,
            distance,
        );
        obstacles.clear(from.eye, to.eye) && threshold >= distance
    }
}

/// The members stand in the party's body, and are as conspicuous as its
/// lowest level and its concealment let it be.
pub fn place_members(
    body: Single<(&Position, &PartyBody)>,
    roster: Single<&PartyRoster>,
    party: Single<&PartyEffects>,
    mut members: Query<(&mut Footing, &mut Conspicuous, &Character, &Vitals)>,
) {
    let (position, body) = *body;
    let eye = PartyBody::eye(position);
    let footing = Footing {
        feet: PartyBody::feet_below(eye),
        eye,
        centre: PartyBody::centre(eye),
        radius: party_body::RADIUS,
        half_height: party_body::RADIUS,
        yaw: body.yaw,
    };
    let level = crate::game::party::lowest_level(
        members.iter_many(roster.iter()).matched().map(|(.., character, vitals)| (character, vitals)),
    );
    let conspicuous = Conspicuous { level, concealment: party.0.concealment() };
    for member in roster.iter() {
        if let Ok((mut at, mut seen, ..)) = members.get_mut(member) {
            at.set_if_neq(footing);
            seen.set_if_neq(conspicuous);
        }
    }
}

/// Monsters stand at their poses, look about by their species, and are as
/// conspicuous as their level.
pub fn place_monsters(
    mut monsters: Query<(
        &mut Footing,
        &mut Eyes,
        &mut Conspicuous,
        &Monster,
        &Vitals,
        ActorPose,
        Option<&Walker>,
    )>,
    content: If<Res<Catalogs>>,
) {
    for (mut at, mut eyes, mut seen, monster, vitals, pose, walker) in &mut monsters {
        let kind = &content.species[monster.species];
        let feet = feet(pose.translation(), walker);
        let half_height = monster.body.eye_height * 0.5;
        at.set_if_neq(Footing {
            feet,
            eye: monster.eye(feet),
            centre: feet + Vec3::Y * half_height,
            radius: monster.body.radius,
            half_height,
            yaw: party_body::yaw_toward(pose.forward()),
        });
        eyes.set_if_neq(Eyes {
            senses: i32::from(kind.attributes.senses),
            bonus: if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 },
            open: vitals.can_fight() && !blinded(kind, vitals),
        });
        seen.set_if_neq(Conspicuous { level: i32::from(kind.level), concealment: 0 });
    }
}
