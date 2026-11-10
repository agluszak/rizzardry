//! Who a burst reaches: those within a radius of a point, in sight of where
//! it went off, and on the side it is against.

use bevy::prelude::*;

use super::effects::BodyReadOnly;
use crate::game::actors::Places;
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::Stance;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;

/// Whom a burst is against.
#[derive(Clone, Copy, Debug)]
pub enum Recipients {
    /// Those who stand against this side.
    Opponents(Stance),
    Everyone,
}

impl Recipients {
    pub fn admits(self, side: Stance) -> bool {
        match self {
            Self::Opponents(source) => source.opposes(side),
            Self::Everyone => true,
        }
    }

    /// Turned back: against the other side.
    pub fn reversed(self) -> Self {
        match self {
            Self::Opponents(Stance::Hostile) => Self::Opponents(Stance::Friendly),
            Self::Opponents(Stance::Friendly) => Self::Opponents(Stance::Hostile),
            other => other,
        }
    }
}

/// Who is about and where they stand, in the order of the fight: the party
/// by seat, then the combatants as they joined, then anyone else.
#[derive(Clone, Copy)]
pub struct Field<'a, 'w, 's> {
    pub bodies: &'a Query<'w, 's, BodyReadOnly>,
    pub places: &'a Places<'w, 's>,
    pub obstacles: &'a WorldObstacles<'w, 's>,
    pub roster: &'a PartyRoster,
    pub combatants: &'a [Entity],
    pub memberships: &'a Query<'w, 's, &'static GroupMember>,
    pub groups: &'a Query<'w, 's, &'static GroupMembers>,
}

impl Field<'_, '_, '_> {
    /// When a body joined the fight; those who have not come last.
    pub fn joined(&self, body: Entity) -> usize {
        self.combatants.iter().position(|&c| c == body).unwrap_or(usize::MAX)
    }

    /// Whether magic may take a body at all.
    fn targetable(&self, body: Entity) -> bool {
        self.bodies.get(body).is_ok_and(|body| !body.untargetable)
    }

    /// The monster group a body is of, as magic sees it: those of the group
    /// it may take. A body of no group, or one magic never takes, is alone.
    fn monster_group(&self, body: Entity) -> Vec<Entity> {
        self.memberships
            .get(body)
            .ok()
            .filter(|_| self.targetable(body))
            .and_then(|member| self.groups.get(member.0).ok())
            .map_or_else(
                || vec![body],
                |members| members.iter().filter(|&mate| self.targetable(mate)).collect(),
            )
    }

    /// A body's group: the party for a member, its monster group for a
    /// monster, itself for anyone else.
    pub fn group_of(&self, body: Entity) -> Vec<Entity> {
        if self.roster.contains(&body) {
            self.roster.iter().collect()
        } else {
            self.monster_group(body)
        }
    }

    /// The first of a monster's group to have joined the fight, whom a group
    /// spell turned back comes from.
    pub fn leader_of(&self, body: Entity) -> Entity {
        self.monster_group(body).into_iter().min_by_key(|&mate| self.joined(mate)).unwrap_or(body)
    }

    /// Everyone a spell may take, in the order of the fight: the party by
    /// seat, the combatants as they joined, then the rest of their monster
    /// groups, fighting or not; none that magic never targets.
    pub fn everyone(&self) -> Vec<Entity> {
        let mut all: Vec<Entity> =
            crate::game::combat::participants(self.roster, self.combatants).collect();
        let mut at = 0;
        while at < all.len() {
            if !self.roster.contains(&all[at]) {
                for mate in self.monster_group(all[at]) {
                    if !all.contains(&mate) {
                        all.push(mate);
                    }
                }
            }
            at += 1;
        }
        all.retain(|&body| self.targetable(body));
        all
    }

    /// Those a burst takes within `radius` of `centre`: the living it is
    /// against whose body reaches within the radius, with a clear line from
    /// `origin`, where it went off, in the order of the fight. Magic takes
    /// neither one it never targets nor a character without a seat.
    pub fn within(
        &self,
        centre: Vec3,
        origin: Vec3,
        radius: f32,
        recipients: Recipients,
    ) -> Vec<Entity> {
        if radius <= 0.0 {
            return vec![];
        }
        let mut taken: Vec<((u8, usize), Entity)> = self
            .bodies
            .iter()
            .filter(|body| !body.untargetable && (body.member || body.character.is_none()))
            .filter(|body| !body.vitals.is_dead() && recipients.admits(body.stance()))
            .filter_map(|body| {
                let at = self.places.of(body.entity);
                let near = at.centre.distance(centre) - at.radius <= radius;
                let order = match self.roster.get_index_of(&body.entity) {
                    Some(seat) => (0, seat),
                    None => match self.combatants.iter().position(|&c| c == body.entity) {
                        Some(joined) => (1, joined),
                        None => (2, 0),
                    },
                };
                (near && self.obstacles.clear(origin, at.centre)).then_some((order, body.entity))
            })
            .collect();
        taken.sort_by_key(|&(order, _)| order);
        taken.into_iter().map(|(_, body)| body).collect()
    }
}
