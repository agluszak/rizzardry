//! Whom a cast reaches (`docs/rules/casting.md` section 5): the victims of a
//! spell by its kind of target, where it lands, what a backfire turns it on,
//! when a missile arrives.

use bevy::prelude::*;

use crate::content::spells::{SpellDefinition, Targeting};
use crate::game::hits::reach::Field;
use crate::game::magic::may::Refusal;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};

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

/// A cast as it is aimed (retail `PopulateSpellTargetMarkers`): whose, of
/// what kind of target, at what, and how far it reaches.
#[derive(Clone, Copy)]
pub struct Aiming<'a, 'w, 's> {
    pub field: &'a Field<'a, 'w, 's>,
    pub caster: Entity,
    pub targeting: Targeting,
    pub aim: Aim,
    /// The metres the spell reaches.
    pub reach: f32,
}

/// Whom a cast reaches and where it lands.
#[derive(Clone, Debug, PartialEq)]
pub struct Reached {
    pub victims: Vec<Entity>,
    /// The point a blast lands on or a cone fans toward, or the foe's feet;
    /// none for a cast aimed at bodies only.
    pub spot: Option<Vec3>,
}

impl Aiming<'_, '_, '_> {
    /// Where a spell goes astray to (retail `PickBackfireTarget`,
    /// `ScatterSpellPointTarget`): one aimed at a body, a friend or a group
    /// takes another in the fight at random, members and living monsters
    /// alike, never the caster; a cone, a blast or a point spell a spot at
    /// random within its reach of the caster. None when nothing else may be
    /// taken.
    pub fn astray(&self, rng: &mut GameRng) -> Option<Aim> {
        let field = self.field;
        match self.targeting {
            Targeting::Ally | Targeting::Enemy | Targeting::EnemyGroup => {
                let members = field.roster.iter().filter(|&member| member != self.caster);
                let monsters = field.combatants.iter().copied().filter(|&body| {
                    body != self.caster && !field.roster.contains(&body) && self.alive(body)
                });
                let others: Vec<Entity> = members.chain(monsters).collect();
                (!others.is_empty())
                    .then(|| Aim::Body(others[rng.below(others.len() as u32) as usize]))
            }
            Targeting::Cone | Targeting::Radius | Targeting::Point => {
                Some(Aim::Point(self.scatter(rng)))
            }
            _ => None,
        }
    }

    /// A spot at random within the spell's reach of the caster: up to five
    /// tries for one that stands on ground, each stopped short by what is
    /// solid in the way; one too far is drawn back to the reach.
    fn scatter(&self, rng: &mut GameRng) -> Vec3 {
        let origin = self.field.places.of(self.caster).feet;
        let reach = self.reach;
        let obstacles = self.field.obstacles;
        let mut spot = origin;
        for _ in 0..5 {
            let across = |rng: &mut GameRng| (rng.below(2001) as f32 - 1000.0) / 1000.0 * reach;
            let (x, z) = (across(rng), across(rng));
            let up = rng.below(1001) as f32 / 1000.0 * reach;
            let mut point = origin + Vec3::new(x, up, z);
            if let Ok(way) = Dir3::new(point - origin)
                && let Some(hit) = obstacles.first_hit(origin, way, origin.distance(point))
            {
                point = origin + *way * hit.distance;
            }
            spot = point;
            if let Some(ground) = obstacles.ground_below(point, up + reach) {
                spot = ground;
                break;
            }
        }
        if spot.distance(origin) > reach {
            spot = origin + (spot - origin).normalize_or_zero() * reach;
        }
        spot
    }

    fn alive(&self, body: Entity) -> bool {
        self.field.bodies.get(body).is_ok_and(|body| !body.vitals.is_dead())
    }

    /// The side a body is on now.
    fn side(&self, body: Entity) -> Option<Stance> {
        self.field.bodies.get(body).ok().map(|body| body.stance())
    }

    /// How the caster stands toward a body.
    fn toward(&self, body: Entity) -> Stance {
        let bodies = self.field.bodies;
        match (bodies.get(self.caster), bodies.get(body)) {
            (Ok(mine), Ok(their)) if !mine.untargetable => {
                hostility::between(mine.standing(), their.standing())
            }
            _ => Stance::Neutral,
        }
    }

    /// Whether the spell reaches a body from its caster: within its reach,
    /// body to body, along a clear line between their eyes (retail
    /// `CharacterActionReachesTarget`, `SourceActionReachesTarget`).
    fn reaches(&self, body: Entity) -> bool {
        let (from, to) = (self.field.places.of(self.caster), self.field.places.of(body));
        from.gap(&to) <= self.reach && self.field.obstacles.clear(from.eye, to.eye)
    }

    /// The living foes the caster's spell reaches, in the order of the fight.
    fn foes(&self) -> Vec<Entity> {
        self.field
            .everyone()
            .into_iter()
            .filter(|&body| {
                body != self.caster
                    && self.alive(body)
                    && self.toward(body) == Stance::Hostile
                    && self.reaches(body)
            })
            .collect()
    }

    /// The foe an aim takes among those reached: the one aimed at, or for an
    /// automatic aim the one nearest the caster's heading, the nearer of
    /// equals.
    fn aimed_foe(&self) -> Option<Entity> {
        let foes = self.foes();
        let places = self.field.places;
        let from = places.of(self.caster);
        match self.aim {
            Aim::Ahead => ahead(
                from.eye,
                from.forward(),
                f32::INFINITY,
                foes.iter().map(|&foe| {
                    let at = places.of(foe);
                    (foe, at.feet, from.gap(&at))
                }),
            ),
            aim => aim.body().filter(|aimed| foes.contains(aimed)),
        }
    }

    /// Whom the spell reaches by its kind of target, in the order of the
    /// fight: the caster; the friend aimed at, if the caster or a friend, and
    /// reached; every member; the foe aimed at (or nearest ahead) among
    /// those reached; that foe and the rest of its group, reached or not;
    /// every foe reached; everyone in a cone toward the foe; a blast's
    /// victims wait for its power. No target is a refusal.
    pub fn victims(&self) -> Result<Reached, Refusal> {
        let (field, caster) = (self.field, self.caster);
        let feet = |body: Entity| field.places.of(body).feet;
        let foe = || self.aimed_foe().ok_or(Refusal::NoTarget);
        match self.targeting {
            Targeting::Caster => Ok(Reached { victims: vec![caster], spot: None }),
            Targeting::Ally => {
                let everyone = field.everyone();
                self.aim
                    .body()
                    .filter(|&aimed| {
                        everyone.contains(&caster)
                            && everyone.contains(&aimed)
                            && (aimed == caster || self.toward(aimed) == Stance::Friendly)
                            && self.reaches(aimed)
                    })
                    .map(|friend| Reached { victims: vec![friend], spot: None })
                    .ok_or(Refusal::NoTarget)
            }
            Targeting::Party => Ok(Reached { victims: field.roster.iter().collect(), spot: None }),
            Targeting::Enemy => {
                let target = foe()?;
                Ok(Reached { victims: vec![target], spot: Some(feet(target)) })
            }
            Targeting::Radius => Ok(Reached { victims: vec![], spot: Some(feet(foe()?)) }),
            // The aimed foe's whole group, of its side, however far or out of
            // sight (retail `PopulateSpellTargetMarkers`).
            Targeting::EnemyGroup => {
                let first = foe()?;
                let side = self.side(first);
                let everyone = field.everyone();
                let mates = field.group_of(first).into_iter().filter(|&mate| {
                    mate != first
                        && everyone.contains(&mate)
                        && self.alive(mate)
                        && self.side(mate) == side
                });
                Ok(Reached {
                    victims: std::iter::once(first).chain(mates).collect(),
                    spot: Some(feet(first)),
                })
            }
            Targeting::AllEnemies => match self.foes() {
                foes if foes.is_empty() => Err(Refusal::NoTarget),
                victims => Ok(Reached { victims, spot: None }),
            },
            Targeting::Cone => {
                let eye = field.places.of(caster).eye;
                let toward = feet(foe()?) - eye;
                let victims = field
                    .everyone()
                    .into_iter()
                    .filter(|&body| body != caster && self.alive(body) && self.reaches(body))
                    .filter(|&body| in_cone(toward, feet(body) - eye))
                    .collect();
                Ok(Reached { victims, spot: Some(eye + toward) })
            }
            Targeting::Point | Targeting::Item | Targeting::LockOrTrap => Err(Refusal::Unsupported),
        }
    }

    /// Those of the caster's own side a backfire of a wide spell falls on:
    /// the living of the caster's side the caster reaches, themselves among
    /// them.
    fn own_side(&self) -> Vec<Entity> {
        let side = self.side(self.caster);
        self.field
            .everyone()
            .into_iter()
            .filter(|&body| {
                self.alive(body)
                    && self.side(body) == side
                    && (body == self.caster || self.reaches(body))
            })
            .collect()
    }

    /// What a backfire is (retail `PrepareSpellTarget`,
    /// `RedirectBackfiredSpellTarget`): the aimed body becomes the caster of
    /// the cast, as strong as it is, and the caster the victim; a group's
    /// leader casts it and it falls on the caster's whole side, however far;
    /// a cone or a blast falls on the caster's side where the spell was to
    /// go off, at `spot`; the rest are cast as they were but fall on the
    /// caster's own side.
    pub fn backfire(&self, spot: Option<Vec3>) -> Backfire {
        let (field, caster) = (self.field, self.caster);
        let eye_of = |body: Entity| field.places.of(body).eye;
        let eye = eye_of(caster);
        let (owner, from) = match (self.targeting, self.aim) {
            (Targeting::Enemy | Targeting::Ally, Aim::Body(entity)) => (entity, eye_of(entity)),
            (Targeting::EnemyGroup, Aim::Body(entity)) => {
                let leader = field.leader_of(entity);
                (leader, eye_of(leader))
            }
            (Targeting::Radius | Targeting::Cone, _) => (caster, spot.unwrap_or(eye)),
            _ => (caster, eye),
        };
        let falls = match self.targeting {
            Targeting::Caster | Targeting::Ally | Targeting::Enemy => Falls::On(vec![caster]),
            Targeting::Radius => Falls::Blast,
            // Turned back, it takes the caster's whole party or group,
            // however far or out of sight.
            Targeting::EnemyGroup => Falls::On(
                field.group_of(caster).into_iter().filter(|&other| self.alive(other)).collect(),
            ),
            _ => Falls::On(self.own_side()),
        };
        let aim = match (self.targeting, spot) {
            (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
            _ => self.aim,
        };
        Backfire { owner, from, aim, falls }
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

/// What a backfire does with a cast: whom it comes from and where, the aim it
/// keeps, and whom it falls on.
#[derive(Clone, Debug, PartialEq)]
pub struct Backfire {
    pub owner: Entity,
    pub from: Vec3,
    pub aim: Aim,
    pub falls: Falls,
}

/// Whom a backfired cast falls on.
#[derive(Clone, Debug, PartialEq)]
pub enum Falls {
    On(Vec<Entity>),
    /// Those within the blast's radius of the caster, on the side the cast was
    /// not against.
    Blast,
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
