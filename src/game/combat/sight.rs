//! What monsters see of the party, and what the reinforcement and
//! leaving rules read of each monster.

use super::*;

/// What monsters see of the party (after retail `UpdateMonsterSight`).
pub(super) struct Sight<'a, 'w, 's> {
    pub(super) eye: Vec3,
    pub(super) far: f32,
    /// The party's lowest level among those who can act, which weakens
    /// sight from behind.
    pub(super) lowest_level: i32,
    pub(super) concealment: u32,
    pub(super) obstacles: &'a WorldObstacles<'w, 's>,
}

impl Sight<'_, '_, '_> {
    /// Whether a monster that can still fight sees the party: within the
    /// far clip, a clear line from its eyes past the level's static
    /// geometry, and its sight threshold covering the distance.
    pub(super) fn sees(
        &self,
        monster: &Monster,
        vitals: &Vitals,
        at: &ActorPoseItem,
        walker: Option<&Walker>,
        kind: &Species,
    ) -> bool {
        let distance = party_distance(self.eye, monster, feet(at.translation(), walker));
        self.sees_point(
            ((monster, vitals), at, walker, kind),
            self.eye,
            distance,
            self.lowest_level,
            self.concealment,
        )
    }

    /// A monster as the reinforcement and leaving rules read it.
    pub(super) fn seen(
        &self,
        entity: Entity,
        ((monster, vitals), at, walker, kind): (
            (&Monster, &Vitals),
            &ActorPoseItem,
            Option<&Walker>,
            &Species,
        ),
    ) -> Seen {
        let at_feet = feet(at.translation(), walker);
        Seen {
            entity,
            at: at_feet,
            from_party: party_distance(self.eye, monster, at_feet),
            sees: self.sees(monster, vitals, at, walker, kind),
            in_view: self.in_view(monster, at, walker),
        }
    }

    /// Whether the party has a clear line to a monster
    /// ([`crate::game::world::perception::in_view`]), whichever way the
    /// monster faces and however keen it is.
    pub(super) fn in_view(
        &self,
        monster: &Monster,
        at: &ActorPoseItem,
        walker: Option<&Walker>,
    ) -> bool {
        crate::game::world::perception::in_view(
            (self.eye, self.far),
            monster,
            feet(at.translation(), walker),
            self.obstacles,
        )
    }

    /// Whether a monster sees another (retail `CanMonsterSeeMonster`):
    /// the same rule, weakened from behind by the other's level.
    pub(super) fn sees_monster(
        &self,
        observer: ((&Monster, &Vitals), &ActorPoseItem, Option<&Walker>, &Species),
        target: (&Monster, &ActorPoseItem, Option<&Walker>, &Species),
    ) -> bool {
        let (body, at, walker, kind) = target;
        let ((from, _), ..) = observer;
        let target_feet = feet(at.translation(), walker);
        let observer_feet = feet(observer.1.translation(), observer.2);
        let distance = from.gap(observer_feet, body, target_feet);
        let eye = body.eye(target_feet);
        self.sees_point(observer, eye, distance, i32::from(kind.level), 0)
    }

    pub(super) fn sees_point(
        &self,
        ((monster, vitals), at, walker, kind): (
            (&Monster, &Vitals),
            &ActorPoseItem,
            Option<&Walker>,
            &Species,
        ),
        target: Vec3,
        distance: f32,
        penalty: i32,
        concealment: u32,
    ) -> bool {
        let feet = feet(at.translation(), walker);
        self.sees_along(
            monster,
            vitals,
            kind,
            feet,
            at.forward(),
            target,
            distance,
            penalty,
            concealment,
        )
    }

    /// Whether a monster standing at `feet` and facing `forward` sees
    /// another (retail `CanMonsterSeeMonster`), from plain values.
    pub(super) fn sees_from(
        &self,
        monster: &Monster,
        vitals: &Vitals,
        kind: &Species,
        feet: Vec3,
        forward: Vec3,
        target: &Monster,
        target_feet: Vec3,
        target_kind: &Species,
    ) -> bool {
        let distance = monster.gap(feet, target, target_feet);
        let eye = target.eye(target_feet);
        self.sees_along(
            monster,
            vitals,
            kind,
            feet,
            forward,
            eye,
            distance,
            i32::from(target_kind.level),
            0,
        )
    }

    /// Whether a monster that can still fight sees `target` `distance`
    /// away: within the far clip, not blinded, a clear line from its eyes,
    /// and its sight threshold covering the distance.
    fn sees_along(
        &self,
        monster: &Monster,
        vitals: &Vitals,
        kind: &Species,
        feet: Vec3,
        forward: Vec3,
        target: Vec3,
        distance: f32,
        penalty: i32,
        concealment: u32,
    ) -> bool {
        let eye = monster.eye(feet);
        if !vitals.can_fight() || distance > self.far || blinded(kind, vitals) {
            return false;
        }
        let clear = self.obstacles.clear(eye, target);
        let forward = forward.with_y(0.0);
        let angle = forward.angle_between((target - eye).with_y(0.0));
        let senses = i32::from(kind.attributes.senses);
        let bonus = if kind.level_sight { (u32::from(kind.level) * 5).min(125) } else { 0 };
        clear
            && sight_threshold(angle, senses, bonus, penalty, concealment, self.far, distance)
                >= distance
    }
}

/// How far from the party a neutral member may stand and still reinforce a
/// fight (retail `GetMonsterEngagementRange`, 25000 world units).
pub(super) const ENGAGEMENT_RANGE: f32 = 50.0;

/// What a group's leader reaches at least, beyond the engagement range, to
/// join a running fight (retail `g_monster_engagement_range_floor`).
pub(super) const ENGAGEMENT_FLOOR: f32 = 15.0;

/// How far out of its way a neutral member may stand from a hostile's line
/// to the party and still be between them (retail
/// `g_reinforcement_distance_slack`).
pub(super) const REINFORCEMENT_SLACK: f32 = 1.15;

/// A monster as the reinforcement and leaving rules read it: where its
/// feet are, how far it is from the party, whether it sees the party
/// ([`Sight::sees`]) and whether the party has a line to it
/// ([`Sight::in_view`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Seen {
    pub(super) entity: Entity,
    pub(super) at: Vec3,
    pub(super) from_party: f32,
    pub(super) sees: bool,
    pub(super) in_view: bool,
}

/// Whether a neutral group stands with a fight (retail
/// `MonsterGroupHasReinforcement`): one of its members in the party's view,
/// within the engagement range, stands roughly between the party and a
/// hostile fighter also in the party's view ([`Seen::in_view`], retail
/// `party_threat.los_flags[1]`). A camouflaged member does not count.
pub(super) fn reinforces(members: &[(Seen, bool)], hostiles: &[Seen]) -> bool {
    members
        .iter()
        .filter(|(member, camouflaged)| {
            !camouflaged && member.in_view && member.from_party <= ENGAGEMENT_RANGE
        })
        .any(|(member, _)| {
            hostiles.iter().filter(|other| other.entity != member.entity && other.in_view).any(
                |other| {
                    let between = member.at.distance(other.at);
                    between + member.from_party < other.from_party * REINFORCEMENT_SLACK
                        && (member.from_party < other.from_party || between < other.from_party)
                },
            )
        })
}
