//! Where an actor stands and what it makes out from there
//! (`docs/rules/actors.md`, "Place").
//!
//! Nothing stores a place. A monster stands where its body is; a member
//! stands where the party is. [`Places`] works one out when a rule asks,
//! and what an actor's eyes are worth and how hard it is to make out are
//! likewise worked out from what the actor already is.

use avian3d::prelude::Position;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{Attacks, Attributes, Level};
use crate::content::characters::Attribute;
use crate::game::characters::{Condition, Vitals};
use crate::game::combat::monster::noticing::{Noticing, notices};
use crate::game::party::body::{self as party_body, PartyBody};
use crate::game::party::formation::{self, Row};
use crate::game::party::roster::PartyMember;
use crate::game::world::ActorPose;
use crate::game::world::navigation::{Walker, feet};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;

/// Where an actor stands and how big it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
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

impl Place {
    pub fn forward(&self) -> Vec3 {
        party_body::heading(self.yaw)
    }

    /// How far it is from another, body to body.
    pub fn gap(&self, other: &Place) -> f32 {
        (self.feet.distance(other.feet) - self.radius - other.radius).max(0.0)
    }

    /// The side of it something at `at` is on.
    pub fn quadrant(&self, at: Vec3) -> Row {
        formation::quadrant(self.eye, self.yaw, at)
    }

    /// Whether it has a clear line to another within the far clip,
    /// whichever way that one faces (retail `party_threat.los_flags`).
    pub fn in_view(&self, other: &Place, far: f32, obstacles: &WorldObstacles) -> bool {
        self.gap(other) <= far && obstacles.clear(self.eye, other.eye)
    }

    /// The party's, its eye at `position` facing `yaw`.
    fn of_party(position: &Position, body: &PartyBody) -> Self {
        let eye = PartyBody::eye(position);
        Place {
            feet: PartyBody::feet_below(eye),
            eye,
            centre: PartyBody::centre(eye),
            radius: party_body::RADIUS,
            half_height: party_body::RADIUS,
            yaw: body.yaw,
        }
    }
}

/// Where anyone stands, worked out as it is asked.
#[derive(SystemParam)]
pub struct Places<'w, 's> {
    monsters: Query<'w, 's, (&'static Monster, ActorPose, Option<&'static Walker>)>,
    members: Query<'w, 's, &'static PartyMember>,
    parties: Query<'w, 's, (&'static Position, &'static PartyBody)>,
}

impl Places<'_, '_> {
    /// An actor's place: a member's is their party's, a monster's its
    /// body's. A party's own place can be asked for too. Every actor
    /// stands somewhere; what is no actor is put at the origin.
    pub fn of(&self, actor: Entity) -> Place {
        self.find(actor).unwrap_or_default()
    }

    fn find(&self, actor: Entity) -> Option<Place> {
        let party = self.members.get(actor).map_or(actor, |member| member.0);
        if let Ok((position, body)) = self.parties.get(party) {
            return Some(Place::of_party(position, body));
        }
        let (monster, pose, walker) = self.monsters.get(actor).ok()?;
        let feet = feet(pose.translation(), walker);
        let half_height = monster.body.eye_height * 0.5;
        Some(Place {
            feet,
            eye: monster.eye(feet),
            centre: feet + Vec3::Y * half_height,
            radius: monster.body.radius,
            half_height,
            yaw: party_body::yaw_toward(pose.forward()),
        })
    }

    /// How far two actors are apart, body to body.
    pub fn gap(&self, a: Entity, b: Entity) -> f32 {
        self.of(a).gap(&self.of(b))
    }
}

/// What an actor's eyes are worth (retail `CanMonsterSeeMonster`,
/// `UpdateMonsterSight`): its Senses and what its kind adds to them; shut
/// when it cannot fight, or is blind and does not fight blind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eyes {
    senses: i32,
    bonus: u32,
    open: bool,
}

impl Eyes {
    pub fn of(vitals: &Vitals, attributes: &Attributes, attacks: &Attacks) -> Self {
        let blinded = vitals.conditions.has(Condition::Blind) && attacks.blind_fighting.is_none();
        Eyes {
            senses: attributes[Attribute::Senses] as i32,
            bonus: attacks.keen_sight,
            open: vitals.can_fight() && !blinded,
        }
    }

    /// Whether these eyes, at `from`, make out someone at `to`
    /// ([`notices`]).
    pub fn see(
        &self,
        from: &Place,
        (to, hidden): (&Place, Hidden),
        far: f32,
        obstacles: &WorldObstacles,
    ) -> bool {
        let gap = from.gap(to);
        let forward = from.forward().with_y(0.0);
        notices(&Noticing {
            open: self.open,
            line: self.open && gap <= far && obstacles.clear(from.eye, to.eye),
            senses: self.senses,
            bonus: self.bonus,
            angle: forward.angle_between((to.eye - from.eye).with_y(0.0)),
            gap,
            hider_level: hidden.level,
            concealment: hidden.concealment,
            far,
            searching: false,
        })
    }
}

/// How hard someone is to make out from behind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hidden {
    level: i32,
    concealment: u32,
}

impl Hidden {
    /// A monster hides by its level.
    pub fn alone(level: Level) -> Self {
        Hidden { level: level.0 as i32, concealment: 0 }
    }

    /// The party hides by the lowest level among its `members` who can
    /// fight, and by what Chameleon adds.
    pub fn party<'a>(
        members: impl IntoIterator<Item = (&'a Level, &'a Vitals)>,
        concealment: u32,
    ) -> Self {
        let able = members.into_iter().filter(|(_, vitals)| vitals.can_fight());
        let level = able.map(|(level, _)| level.0 as i32).min().unwrap_or(0);
        Hidden { level, concealment }
    }
}
