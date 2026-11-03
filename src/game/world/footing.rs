//! Where each body stands, and how bodies see one another: the same
//! components on a party member and a monster, so the rules that ask who
//! reaches or sees whom read them alike. One system places every body from
//! its own sources (a member from the party's body and spells, a monster
//! from its pose, species and vitals), the way `GlobalTransform` follows
//! `Transform`, before anything reads it: at the start of each frame's
//! gameplay and of each fixed step. Physics stays the authority on
//! position; nothing else writes these.

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

/// Where a body stands and how big it is: a monster at its pose, the
/// party's body where physics has it, a member in the party's body, which
/// they share.
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

/// Every body is placed: the party's body where physics has it, and each
/// member in it, as conspicuous as the party's lowest level and its
/// concealment let it be; a monster at its own pose, looking about by its
/// species, as conspicuous as its level.
#[allow(clippy::type_complexity)]
pub fn place_bodies(
    party: Single<(
        Ref<Position>,
        Ref<PartyBody>,
        Ref<PartyEffects>,
        Ref<PartyRoster>,
        &mut Footing,
    )>,
    mut bodies: Query<
        (
            Entity,
            &mut Footing,
            &mut Conspicuous,
            Option<&mut Eyes>,
            Ref<Vitals>,
            AnyOf<(Ref<Character>, (Ref<Monster>, ActorPose, Option<Ref<Walker>>))>,
        ),
        Without<PartyBody>,
    >,
    content: If<Res<Catalogs>>,
    mut departed: RemovedComponents<crate::game::party::roster::PartyMember>,
    mut removed_walkers: RemovedComponents<Walker>,
    mut removed_positions: RemovedComponents<Position>,
    mut removed_rotations: RemovedComponents<avian3d::prelude::Rotation>,
) {
    let (position, body, effects, roster, mut party_at) = party.into_inner();
    let eye = PartyBody::eye(&position);
    let in_party = Footing {
        feet: PartyBody::feet_below(eye),
        eye,
        centre: PartyBody::centre(eye),
        radius: party_body::RADIUS,
        half_height: party_body::RADIUS,
        yaw: body.yaw,
    };
    party_at.set_if_neq(in_party);
    let party_changed = position.is_changed()
        || body.is_changed()
        || effects.is_changed()
        || roster.is_changed()
        || bodies.iter_many(roster.iter()).matched().any(|(.., vitals, (character, _))| {
            vitals.is_changed() || character.is_some_and(|c| c.is_changed())
        });
    let level =
        crate::game::party::lowest_level(bodies.iter_many(roster.iter()).matched().filter_map(
            |(.., vitals, (character, _))| character.map(|c| (c.into_inner(), vitals.into_inner())),
        ));
    let removed: bevy::ecs::entity::EntityHashSet = departed
        .read()
        .chain(removed_walkers.read())
        .chain(removed_positions.read())
        .chain(removed_rotations.read())
        .collect();
    let of_party = Conspicuous { level, concealment: effects.0.concealment() };
    for (entity, mut at, mut seen, eyes, vitals, (character, monster)) in &mut bodies {
        match (character.filter(|_| roster.contains(&entity)), monster) {
            (Some(_), _) => {
                if !(party_changed || at.is_added() || seen.is_added()) {
                    continue;
                }
                at.set_if_neq(in_party);
                seen.set_if_neq(of_party);
            }
            (None, Some((monster, pose, walker))) => {
                if !(at.is_added()
                    || seen.is_added()
                    || eyes.as_ref().is_some_and(|e| e.is_added())
                    || monster.is_changed()
                    || pose.is_changed()
                    || vitals.is_changed()
                    || walker.as_ref().is_some_and(|w| w.is_changed())
                    || content.is_changed()
                    || removed.contains(&entity))
                {
                    continue;
                }
                let kind = &content.species[monster.species];
                let feet = feet(pose.translation(), walker.as_deref());
                let half_height = monster.body.eye_height * 0.5;
                at.set_if_neq(Footing {
                    feet,
                    eye: monster.eye(feet),
                    centre: feet + Vec3::Y * half_height,
                    radius: monster.body.radius,
                    half_height,
                    yaw: party_body::yaw_toward(pose.forward()),
                });
                if let Some(mut eyes) = eyes {
                    eyes.set_if_neq(Eyes {
                        senses: i32::from(kind.attributes.senses),
                        bonus: if kind.level_sight {
                            (u32::from(kind.level) * 5).min(125)
                        } else {
                            0
                        },
                        open: vitals.can_fight() && !blinded(kind, &vitals),
                    });
                }
                seen.set_if_neq(Conspicuous { level: i32::from(kind.level), concealment: 0 });
            }
            (None, None) => {}
        }
    }
}
