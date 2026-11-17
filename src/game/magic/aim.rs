//! Whom magic takes in (`docs/rules/casting.md` section 5 and "How this is
//! built"): an area about a place, against a side, answered by
//! [`Reaching`] as it is asked. Who may be taken is what the bodies are: the
//! party's members and the monsters, living (the party, all of it), never a
//! monster magic never targets; in the original's order, the members by
//! seat, then the monsters as they were placed.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::reach::{Aim, ahead, in_cone};
use crate::game::actors::{Place, Places};
use crate::game::characters::Vitals;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::world::hostility::{self, Stance, Standing};
use crate::game::world::identify::{GroupMember, GroupMembers, Leads};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Untargetable};

/// What a monster's proposed aim is at.
#[derive(Component, Clone, Copy, Debug)]
pub struct At(pub Aim);

/// Whom an area is against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Against {
    /// Those whose it is stands hostile toward.
    FoesOf,
    /// Those of the side whose it is stands on, itself among them.
    FellowsOf,
    /// Those who stand against this side.
    Opponents(Stance),
    Everyone,
}

impl Against {
    /// Against the other side: a backfire or a reflection.
    pub fn reversed(self) -> Self {
        match self {
            Self::FoesOf => Self::FellowsOf,
            Self::FellowsOf => Self::FoesOf,
            Self::Opponents(Stance::Hostile) => Self::Opponents(Stance::Friendly),
            Self::Opponents(Stance::Friendly) => Self::Opponents(Stance::Hostile),
            other => other,
        }
    }

    /// Against the opponents of a side, its own if `backfired`; everyone if
    /// it comes from no side.
    pub fn opposing(side: Option<Stance>, backfired: bool) -> Self {
        match side {
            Some(side) if backfired => Self::Opponents(side).reversed(),
            Some(side) => Self::Opponents(side),
            None => Self::Everyone,
        }
    }
}

/// An area: whom an aim takes in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Area {
    /// Whose it is.
    Caster,
    /// The body aimed at, whatever it is.
    One(Aim),
    /// The body aimed at, if it is whose the aim is or a friend, within reach.
    Friend { at: Aim, reach: f32 },
    /// Every member of the party, the dead too.
    Party,
    /// The body aimed at and the rest of its group of its side, living,
    /// reached or not: the party for a member, its monster group for a
    /// monster.
    Group(Entity),
    /// Every living body it is against that whose it is reaches.
    Reached { reach: f32 },
    /// Every living body it is against but whose it is, inside the cone from
    /// `origin` toward `spot` (retail `TargetInRangeAndArcs`): its body
    /// within the reach, both bodies' radii taken off and the rest rounded
    /// down, a clear line to its eye, its feet within the arcs.
    Cone { origin: Vec3, spot: Vec3, reach: f32 },
    /// Every living body it is against whose body reaches within the radius
    /// of `spot`, with a clear line from `origin` (retail
    /// `CollectMonstersWithinRadius`).
    Blast { spot: Vec3, origin: Vec3, radius: f32 },
}

/// Whether a body at `from` reaches one at `to` within `reach`: body to
/// body, along a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `SourceActionReachesTarget`).
pub fn reaches(from: &Place, to: &Place, reach: f32, obstacles: &WorldObstacles) -> bool {
    from.gap(to) <= reach && obstacles.clear(from.eye, to.eye)
}

/// Who stands where, as magic sees them: whom an area takes in, the foe an
/// aim picks, a group's leader.
#[derive(SystemParam)]
pub struct Reaching<'w, 's> {
    #[allow(clippy::type_complexity)]
    bodies: Query<
        'w,
        's,
        (Entity, &'static Vitals, &'static Standing),
        (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>),
    >,
    stands: Query<'w, 's, (&'static Standing, Has<Untargetable>)>,
    roster: Single<'w, 's, &'static PartyRoster>,
    monsters: Query<'w, 's, &'static Monster>,
    members: Query<'w, 's, &'static GroupMember>,
    groups: Query<'w, 's, &'static GroupMembers>,
    leads: Query<'w, 's, (), With<Leads>>,
    places: Places<'w, 's>,
    obstacles: WorldObstacles<'w, 's>,
}

impl Reaching<'_, '_> {
    /// Where a body stands.
    pub fn place(&self, body: Entity) -> Place {
        self.places.of(body)
    }

    /// The leader of a body's monster group, or the body itself.
    pub fn leader(&self, body: Entity) -> Entity {
        self.members
            .get(body)
            .and_then(|member| self.groups.get(member.0))
            .ok()
            .and_then(|group| group.iter().find(|&mate| self.leads.contains(mate)))
            .unwrap_or(body)
    }

    /// The party's members, by seat.
    pub fn party(&self) -> impl Iterator<Item = Entity> + '_ {
        self.roster.iter()
    }

    /// Whether `against`, from `whose`, takes in `body` standing so.
    fn admits(
        &self,
        against: Against,
        whose: Option<Entity>,
        body: Entity,
        theirs: Standing,
    ) -> bool {
        let mine = whose.and_then(|whose| self.stands.get(whose).ok());
        match (against, mine) {
            (Against::Opponents(side), _) => side.opposes(theirs.stance),
            (Against::Everyone, _) => true,
            (Against::FoesOf, Some((&mine, false))) => {
                hostility::between(mine, theirs) == Stance::Hostile
            }
            (Against::FellowsOf, Some((mine, _))) => {
                Some(body) == whose || mine.stance == theirs.stance
            }
            (Against::FoesOf | Against::FellowsOf, _) => false,
        }
    }

    /// Those living, takable and `against` would take in, by `within`.
    fn living(
        &self,
        whose: Option<Entity>,
        against: Against,
        within: impl Fn(Entity) -> bool,
    ) -> Vec<Entity> {
        let mut taken: Vec<Entity> = self
            .bodies
            .iter()
            .filter(|&(body, vitals, &theirs)| {
                !vitals.is_dead() && self.admits(against, whose, body, theirs)
            })
            .map(|(body, ..)| body)
            .filter(|&body| within(body))
            .collect();
        taken.sort_by_key(|&body| self.order_of(body));
        taken
    }

    /// Where a body stands among those magic takes.
    fn order_of(&self, body: Entity) -> (u8, i64) {
        match self.roster.get_index_of(&body) {
            Some(seat) => (0, seat as i64),
            None => (1, self.monsters.get(body).map_or(i64::MAX, |m| i64::from(m.location))),
        }
    }

    /// The foe an aim from `whose` picks among the living foes it reaches:
    /// the one aimed at, or for an automatic aim the one nearest its heading,
    /// the nearer of equals.
    pub fn foe(&self, whose: Entity, at: Aim, reach: f32) -> Option<Entity> {
        let from = self.place(whose);
        let foes = self.living(Some(whose), Against::FoesOf, |body| {
            body != whose && reaches(&from, &self.place(body), reach, &self.obstacles)
        });
        match at {
            Aim::Ahead => ahead(
                from.eye,
                from.forward(),
                f32::INFINITY,
                foes.iter().map(|&foe| {
                    let there = self.place(foe);
                    (foe, there.feet, from.gap(&there))
                }),
            ),
            at => at.body().filter(|aimed| foes.contains(aimed)),
        }
    }

    /// Whom `area`, `whose` it is, takes in, against `against`.
    pub fn takes(&self, area: Area, whose: Option<Entity>, against: Against) -> Vec<Entity> {
        let near = |reach: f32| {
            move |body: Entity| match whose {
                Some(whose) => {
                    body == whose
                        || reaches(&self.place(whose), &self.place(body), reach, &self.obstacles)
                }
                None => false,
            }
        };
        match area {
            Area::Caster => whose.into_iter().collect(),
            Area::One(at) => at.body().into_iter().collect(),
            Area::Party => self.roster.iter().collect(),
            Area::Friend { at, reach } => {
                let friend = at.body().filter(|&aimed| {
                    let Some(whose) = whose else { return false };
                    let (Ok((&mine, _)), Ok((&theirs, never))) =
                        (self.stands.get(whose), self.stands.get(aimed))
                    else {
                        return false;
                    };
                    !never
                        && (aimed == whose || hostility::between(mine, theirs) == Stance::Friendly)
                        && near(reach)(aimed)
                });
                friend.into_iter().collect()
            }
            Area::Group(first) => {
                let side = self.stands.get(first).map(|(standing, _)| standing.stance).ok();
                let group: Vec<Entity> = match self.roster.contains(&first) {
                    true => self.roster.iter().collect(),
                    false => self
                        .members
                        .get(first)
                        .and_then(|member| self.groups.get(member.0))
                        .map_or_else(|_| vec![first], |members| members.iter().collect()),
                };
                let mut mates = self.living(whose, Against::Everyone, |mate| {
                    mate != first
                        && group.contains(&mate)
                        && self.stands.get(mate).is_ok_and(|(s, _)| Some(s.stance) == side)
                });
                mates.insert(0, first);
                mates
            }
            Area::Reached { reach } => self.living(whose, against, near(reach)),
            Area::Cone { origin, spot, reach } => {
                let apex = whose.map_or(0.0, |whose| self.place(whose).radius);
                self.living(whose, against, |body| {
                    let there = self.place(body);
                    Some(body) != whose
                        && (there.centre.distance(origin) - there.radius - apex).floor() <= reach
                        && self.obstacles.clear(origin, there.eye)
                        && in_cone(spot - origin, there.feet - origin)
                })
            }
            Area::Blast { spot, origin, radius } if radius > 0.0 => {
                self.living(whose, against, |body| {
                    let there = self.place(body);
                    there.centre.distance(spot) - there.radius <= radius
                        && self.obstacles.clear(origin, there.centre)
                })
            }
            Area::Blast { .. } => Vec::new(),
        }
    }
}
