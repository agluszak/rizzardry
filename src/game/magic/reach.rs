//! Whom a cast reaches (`docs/rules/casting.md` section 5): the victims of a
//! spell by its kind of target, where it lands, what a backfire turns it on,
//! when a missile arrives. Each a function of the facts it is about and
//! nothing else.

use bevy::prelude::*;

use crate::content::spells::{SpellDefinition, Targeting, Working};
use crate::game::hits::reach::{Recipients, Struck, within};
use crate::game::magic::may::Refusal;
use crate::game::world::hostility::Stance;

/// How fast spell missiles fly: retail's `energy_blast.mls` velocity of 30,
/// advanced per two-millisecond step (`Missile.cpp`), comes to 15 m/s.
pub const MISSILE_SPEED: f32 = 15.0;

/// Whether a body is reached: within the spell's reach of the caster, edge to
/// edge, and a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `SourceActionReachesTarget`).
pub fn reached(gap: f32, reach: f32, clear: bool) -> bool {
    gap <= reach && clear
}

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

/// A body in the fight as the aim sees it, in the order of the fight: the
/// party's members by seat, then the monsters as they joined it.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub id: Entity,
    /// One of the party's members.
    pub member: bool,
    pub dead: bool,
    /// The side it is on now.
    pub stance: Stance,
    /// How the caster stands toward it.
    pub toward: Stance,
    /// Within the spell's reach of the caster, edge to edge, with a clear line
    /// between their eyes.
    pub reached: bool,
    /// The gap between its body and the caster's.
    pub gap: f32,
    pub feet: Vec3,
    pub eye: Vec3,
    /// The monster group it is in, as an index of [`Aiming::groups`].
    pub group: Option<usize>,
    /// When it joined the fight; those who have not come last.
    pub joined: usize,
}

/// What aiming a spell weighs (retail `PopulateSpellTargetMarkers`).
#[derive(Clone, Copy, Debug)]
pub struct Aiming<'a> {
    pub targeting: Targeting,
    pub caster: Entity,
    pub aim: Aim,
    /// Where the caster looks from, and which way they face.
    pub eye: Vec3,
    pub forward: Vec3,
    /// Everyone a spell may take, the caster among them.
    pub bodies: &'a [Target],
    /// The party, in its seats.
    pub party: &'a [Entity],
    /// The monster groups, each in the order of its members.
    pub groups: &'a [Vec<Entity>],
}

/// Whom a cast reaches and where it lands.
#[derive(Clone, Debug, PartialEq)]
pub struct Reached {
    pub victims: Vec<Entity>,
    /// The point a blast lands on or a cone fans toward, or the foe's feet;
    /// none for a cast aimed at bodies only.
    pub spot: Option<Vec3>,
}

impl Aiming<'_> {
    fn find(&self, id: Entity) -> Option<&Target> {
        self.bodies.iter().find(|body| body.id == id)
    }

    /// A body's group, as retail takes it: the party for a member, its monster
    /// group for a monster, itself for anyone else.
    fn group_of(&self, id: Entity) -> Vec<Entity> {
        match self.find(id) {
            Some(body) if body.member => self.party.to_vec(),
            Some(Target { group: Some(group), .. }) => self.groups[*group].clone(),
            _ => vec![id],
        }
    }

    /// The first of a monster's group to have joined the fight, whom a group
    /// spell turned back comes from.
    fn leader_of(&self, id: Entity) -> Entity {
        let Some(Target { group: Some(group), .. }) = self.find(id) else { return id };
        self.groups[*group]
            .iter()
            .copied()
            .min_by_key(|&mate| self.find(mate).map_or(usize::MAX, |body| body.joined))
            .unwrap_or(id)
    }

    /// The living foes the caster's spell reaches, in the order of the fight.
    fn foes(&self) -> Vec<&Target> {
        self.bodies
            .iter()
            .filter(|body| {
                body.id != self.caster
                    && !body.dead
                    && body.toward == Stance::Hostile
                    && body.reached
            })
            .collect()
    }

    /// The foe an aim takes among those reached: the one aimed at, or for an
    /// automatic aim the one nearest the caster's heading, the nearer of
    /// equals.
    fn aimed_foe(&self) -> Option<Entity> {
        let foes = self.foes();
        match self.aim {
            Aim::Ahead => ahead(
                self.eye,
                self.forward,
                f32::INFINITY,
                foes.iter().map(|foe| (foe.id, foe.feet, foe.gap)),
            ),
            aim => aim.body().filter(|aimed| foes.iter().any(|foe| foe.id == *aimed)),
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

/// Whom a spell reaches by its kind of target, in the order of the fight:
/// the caster; the friend aimed at, if the caster or a friend, and reached;
/// every member; the foe aimed at (or nearest ahead) among those reached; that
/// foe and the rest of its group, reached or not; every foe reached; everyone
/// in a cone toward the foe; a blast's victims wait for its power. No target is
/// a refusal.
pub fn victims(facts: &Aiming) -> Result<Reached, Refusal> {
    let caster = facts.caster;
    let feet = |id: Entity| facts.find(id).map_or(Vec3::ZERO, |body| body.feet);
    match facts.targeting {
        Targeting::Caster => Ok(Reached { victims: vec![caster], spot: None }),
        Targeting::Ally => {
            let friend = facts.aim.body().filter(|&aimed| {
                facts.find(aimed).is_some_and(|to| {
                    facts.find(caster).is_some()
                        && (aimed == caster || to.toward == Stance::Friendly)
                        && to.reached
                })
            });
            friend
                .map(|friend| Reached { victims: vec![friend], spot: None })
                .ok_or(Refusal::NoTarget)
        }
        Targeting::Party => Ok(Reached { victims: facts.party.to_vec(), spot: None }),
        Targeting::Enemy => {
            let target = facts.aimed_foe().ok_or(Refusal::NoTarget)?;
            Ok(Reached { victims: vec![target], spot: Some(feet(target)) })
        }
        Targeting::Radius => {
            let target = facts.aimed_foe().ok_or(Refusal::NoTarget)?;
            Ok(Reached { victims: vec![], spot: Some(feet(target)) })
        }
        // The aimed foe's whole group, of its side, however far or out of sight
        // (retail `PopulateSpellTargetMarkers`, `Magic.cpp:3714-3733`).
        Targeting::EnemyGroup => {
            let first = facts.aimed_foe().ok_or(Refusal::NoTarget)?;
            let side = facts.find(first).map(|body| body.stance);
            let mates = facts.group_of(first).into_iter().filter(|&mate| {
                mate != first
                    && facts.find(mate).is_some_and(|mate| !mate.dead && Some(mate.stance) == side)
            });
            Ok(Reached {
                victims: std::iter::once(first).chain(mates).collect(),
                spot: Some(feet(first)),
            })
        }
        Targeting::AllEnemies => {
            let victims: Vec<Entity> = facts.foes().iter().map(|foe| foe.id).collect();
            if victims.is_empty() {
                Err(Refusal::NoTarget)
            } else {
                Ok(Reached { victims, spot: None })
            }
        }
        Targeting::Cone => {
            let target = facts.aimed_foe().ok_or(Refusal::NoTarget)?;
            let toward = feet(target) - facts.eye;
            let victims = facts
                .bodies
                .iter()
                .filter(|body| body.id != caster && !body.dead && body.reached)
                .filter(|body| in_cone(toward, body.feet - facts.eye))
                .map(|body| body.id)
                .collect();
            Ok(Reached { victims, spot: Some(facts.eye + toward) })
        }
        Targeting::Point | Targeting::Item | Targeting::LockOrTrap => Err(Refusal::Unsupported),
    }
}

/// The metres a blast spreads from where it lands: the spell's radius and its
/// radius per power times the power.
pub fn blast_radius(spell: &SpellDefinition, power: u32) -> f32 {
    spell.radius + spell.radius_per_power * power as f32
}

/// Those a blast takes within `radius` of `centre`: the living bodies, on the
/// side it is against, whose body reaches within the radius, with a clear line
/// from where it went off, in the order of the fight.
pub fn blasted(
    centre: Vec3,
    radius: f32,
    recipients: Recipients,
    bodies: &[Struck],
) -> Vec<Entity> {
    within(centre, radius, recipients, bodies.iter().copied())
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

/// What a backfire is (retail `PrepareSpellTarget`,
/// `RedirectBackfiredSpellTarget`): the aimed body becomes the caster of the
/// cast, as strong as it is, and the caster the victim; a group's leader casts
/// it and it falls on the caster's whole side, however far; a cone or a blast
/// falls on the caster's side where the spell was to go off; the rest are cast
/// as they were but fall on the caster's own side.
pub fn backfire(facts: &Aiming, reach_of_caster: &[Entity], spot: Option<Vec3>) -> Backfire {
    let caster = facts.caster;
    let eye_of = |id: Entity| facts.find(id).map_or(facts.eye, |body| body.eye);
    let (owner, from) = match (facts.targeting, facts.aim) {
        (Targeting::Enemy | Targeting::Ally, Aim::Body(entity)) => (entity, eye_of(entity)),
        (Targeting::EnemyGroup, Aim::Body(entity)) => {
            let leader = facts.leader_of(entity);
            (leader, eye_of(leader))
        }
        (Targeting::Radius | Targeting::Cone, _) => (caster, spot.unwrap_or(facts.eye)),
        _ => (caster, facts.eye),
    };
    let falls = match facts.targeting {
        Targeting::Caster | Targeting::Ally | Targeting::Enemy => Falls::On(vec![caster]),
        Targeting::Radius => Falls::Blast,
        // Turned back, it takes the caster's whole party or group, however far
        // or out of sight (retail `RedirectBackfiredSpellTarget`).
        Targeting::EnemyGroup => Falls::On(
            facts
                .group_of(caster)
                .into_iter()
                .filter(|&other| facts.find(other).is_some_and(|to| !to.dead))
                .collect(),
        ),
        _ => Falls::On(reach_of_caster.to_vec()),
    };
    let aim = match (facts.targeting, spot) {
        (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
        _ => facts.aim,
    };
    Backfire { owner, from, aim, falls }
}

/// Those of the caster's own side a backfire of a wide spell falls on: the
/// living of the caster's side the caster reaches, themselves among them.
pub fn own_side(facts: &Aiming) -> Vec<Entity> {
    let side = facts.find(facts.caster).map(|body| body.stance);
    facts
        .bodies
        .iter()
        .filter(|body| {
            !body.dead && Some(body.stance) == side && (body.id == facts.caster || body.reached)
        })
        .map(|body| body.id)
        .collect()
}

/// The seconds a missile takes from where the cast set out to where it lands.
pub fn flight(from: Vec3, to: Vec3) -> f32 {
    to.distance(from) / MISSILE_SPEED
}

/// What a utility spell is aimed at, as the party's surroundings offer it.
#[derive(Clone, Copy, Debug)]
pub struct Marking {
    pub working: Working,
    pub aim: Aim,
    /// The metres from the party's eye to the spot aimed at, and whether the
    /// line to it is clear.
    pub spot: Option<(f32, bool)>,
    /// Metres the spell reaches, and the height of the party's eye, which a
    /// spot may lie beyond.
    pub reach: f32,
    pub eye_height: f32,
    /// The item aimed at, as the party has it.
    pub item: Option<Held>,
    /// A portal is saved on the caster.
    pub portal: bool,
    /// The living body of the one the party is talking with.
    pub speaker: Option<Entity>,
    /// An elemental the caster summoned still stands.
    pub elemental: bool,
    /// The lock or trap attempt under way, named by the aim.
    pub attempt: Option<Attempt>,
}

/// An item aimed at by Identify Item.
#[derive(Clone, Copy, Debug)]
pub struct Held {
    /// The party has it: in hand, in the pool, or with a member.
    pub party_has_it: bool,
    /// It is the item casting the spell.
    pub itself: bool,
    /// Wholly known already.
    pub known: bool,
}

/// A lock or trap attempt, and the device it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt {
    /// The device is still guarded.
    pub guarded: bool,
    pub kind: Device,
}

/// What an attempt works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Lock,
    /// A trap whose discharge the game resolves, or not.
    Trap {
        resolved: bool,
    },
    /// A container being looted.
    Container,
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

/// Whether a utility spell has a target, and which: Identify Item an item the
/// party has, not the item casting it, not wholly known; Return to Portal a
/// portal saved; Charm and Mindread the one the party is talking with; Lure
/// and Summon Elemental a spot within the spell's reach and an eye's height
/// that the party sees, an elemental of the caster's not standing; Knock Knock
/// and Divine Trap the attempt under way, on a device still guarded, a trap the
/// game resolves.
pub fn mark(facts: &Marking) -> Result<Marked, Refusal> {
    match facts.working {
        Working::Identify => match (facts.aim, facts.item) {
            (Aim::Item(item), Some(held)) if held.party_has_it && !held.itself && !held.known => {
                Ok(Marked::Item(item))
            }
            _ => Err(Refusal::NoTarget),
        },
        Working::SetPortal => Ok(Marked::Nothing),
        Working::ReturnToPortal if facts.portal => Ok(Marked::Nothing),
        Working::ReturnToPortal => Err(Refusal::NoTarget),
        Working::Charm | Working::Mindread => match (facts.aim, facts.speaker) {
            (Aim::Body(aimed), Some(speaker)) if aimed == speaker => Ok(Marked::Body(speaker)),
            _ => Err(Refusal::NoTarget),
        },
        Working::Lure | Working::Summon => {
            let Aim::Point(at) = facts.aim else { return Err(Refusal::NoTarget) };
            let seen = facts
                .spot
                .is_some_and(|(far, clear)| far <= facts.reach + facts.eye_height && clear);
            if !seen || (facts.working == Working::Summon && facts.elemental) {
                return Err(Refusal::NoTarget);
            }
            Ok(Marked::Spot(at))
        }
        Working::Knock | Working::DivineTrap => {
            let Aim::Device(on) = facts.aim else { return Err(Refusal::NoTarget) };
            match facts.attempt {
                Some(Attempt { guarded: true, kind: Device::Lock }) => Ok(Marked::Attempt(on)),
                Some(Attempt { guarded: true, kind: Device::Trap { resolved: true } }) => {
                    Ok(Marked::Attempt(on))
                }
                Some(Attempt { guarded: true, kind: Device::Trap { resolved: false } }) => {
                    Err(Refusal::Unsupported)
                }
                _ => Err(Refusal::NoTarget),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    /// Touch reaches 2 metres, short 4, long 25, extreme 50, edge to edge; a
    /// wall between the eyes stops it whatever the gap.
    #[test]
    fn a_body_is_reached_within_the_reach_in_clear_sight() {
        assert!(reached(0.0, 2.0, true));
        assert!(reached(2.0, 2.0, true));
        assert!(!reached(2.1, 2.0, true));
        assert!(!reached(1.0, 2.0, false));
        assert!(reached(25.0, 25.0, true) && !reached(25.5, 25.0, true));
    }

    /// A body at (x, 0, z), the caster standing at the origin looking along +z.
    fn body(index: u32, x: f32, z: f32) -> Target {
        Target {
            id: id(index),
            member: false,
            dead: false,
            stance: Stance::Hostile,
            toward: Stance::Hostile,
            reached: true,
            gap: Vec2::new(x, z).length(),
            feet: Vec3::new(x, 0.0, z),
            eye: Vec3::new(x, 1.7, z),
            group: None,
            joined: index as usize,
        }
    }

    fn member(index: u32, x: f32, z: f32) -> Target {
        Target {
            member: true,
            stance: Stance::Friendly,
            toward: Stance::Friendly,
            ..body(index, x, z)
        }
    }

    const CASTER: u32 = 1;

    fn aiming<'a>(
        targeting: Targeting,
        aim: Aim,
        bodies: &'a [Target],
        party: &'a [Entity],
        groups: &'a [Vec<Entity>],
    ) -> Aiming<'a> {
        Aiming {
            targeting,
            caster: id(CASTER),
            aim,
            eye: Vec3::new(0.0, 1.7, 0.0),
            forward: Vec3::Z,
            bodies,
            party,
            groups,
        }
    }

    /// The party of two members (1 the caster, 2) and three monsters: 3 ahead, 4
    /// to the side, 5 out of reach.
    fn fight() -> Vec<Target> {
        vec![
            member(1, 0.0, 0.0),
            member(2, -1.0, 0.0),
            body(3, 0.0, 3.0),
            body(4, 3.0, 0.0),
            Target { reached: false, ..body(5, 0.0, 40.0) },
        ]
    }

    fn victims_of(targeting: Targeting, aim: Aim, bodies: &[Target]) -> Result<Reached, Refusal> {
        let party = [id(1), id(2)];
        super::victims(&aiming(targeting, aim, bodies, &party, &[]))
    }

    #[test]
    fn a_caster_spell_reaches_the_caster_and_a_party_spell_every_member_the_dead_too() {
        let mut bodies = fight();
        bodies[1].dead = true;
        let at = |targeting| victims_of(targeting, Aim::Body(id(2)), &bodies).unwrap();
        assert_eq!(at(Targeting::Caster).victims, [id(1)]);
        assert_eq!(at(Targeting::Party).victims, [id(1), id(2)]);
    }

    /// A friend aimed at is reached if the caster or a friend and in reach.
    #[test]
    fn a_friend_is_reached_if_aimed_at_and_in_reach() {
        let mut bodies = fight();
        let at = |aim, bodies: &[Target]| victims_of(Targeting::Ally, aim, bodies);
        assert_eq!(at(Aim::Body(id(2)), &bodies).unwrap().victims, [id(2)]);
        assert_eq!(at(Aim::Body(id(1)), &bodies).unwrap().victims, [id(1)]);
        // A foe is no friend; a friend out of reach is not reached; nobody aimed.
        assert_eq!(at(Aim::Body(id(3)), &bodies), Err(Refusal::NoTarget));
        bodies[1].reached = false;
        assert_eq!(at(Aim::Body(id(2)), &bodies), Err(Refusal::NoTarget));
        assert_eq!(at(Aim::Ahead, &bodies), Err(Refusal::NoTarget));
        // The caster is always a friend to themselves, even a turncoat's.
        bodies[0].toward = Stance::Hostile;
        assert_eq!(at(Aim::Body(id(1)), &bodies).unwrap().victims, [id(1)]);
    }

    /// An enemy aimed at must be a foe reached: 3 is, 5 is out of reach, 2 is a
    /// friend. It lands at the foe's feet.
    #[test]
    fn an_enemy_must_be_a_foe_in_reach() {
        let bodies = fight();
        let enemy = |aim| victims_of(Targeting::Enemy, aim, &bodies);
        assert_eq!(
            enemy(Aim::Body(id(3))).unwrap(),
            Reached { victims: vec![id(3)], spot: Some(Vec3::new(0.0, 0.0, 3.0)) }
        );
        assert_eq!(enemy(Aim::Body(id(5))), Err(Refusal::NoTarget));
        assert_eq!(enemy(Aim::Body(id(2))), Err(Refusal::NoTarget));
        let mut dead = fight();
        dead[2].dead = true;
        assert_eq!(victims_of(Targeting::Enemy, Aim::Body(id(3)), &dead), Err(Refusal::NoTarget));
    }

    /// Aimed ahead: the foe whose direction is nearest the way the caster faces
    /// (3, straight ahead, 0 degrees; 4 is 90 degrees); the nearer of two equal
    /// (6 is straight ahead too, but further).
    #[test]
    fn an_enemy_aimed_ahead_is_the_foe_nearest_the_heading_the_nearer_of_equals() {
        let mut bodies = fight();
        let ahead = |bodies: &[Target]| victims_of(Targeting::Enemy, Aim::Ahead, bodies).unwrap();
        assert_eq!(ahead(&bodies).victims, [id(3)]);
        // Another straight ahead but further: the nearer, 3, still.
        bodies.push(body(6, 0.0, 6.0));
        assert_eq!(ahead(&bodies).victims, [id(3)]);
        // No foe reached: no target.
        let none = [member(1, 0.0, 0.0), Target { reached: false, ..body(5, 0.0, 40.0) }];
        assert_eq!(victims_of(Targeting::Enemy, Aim::Ahead, &none), Err(Refusal::NoTarget));
    }

    /// A group: the foe aimed at, then every other of its group that lives and
    /// is on its side, reached or not (5 is out of reach); the dead are left.
    #[test]
    fn an_enemy_group_is_the_foe_and_the_rest_of_its_group_however_far() {
        let mut bodies = fight();
        bodies[2].group = Some(0);
        bodies[3].group = Some(0);
        bodies[4].group = Some(0);
        let groups = [vec![id(3), id(4), id(5)]];
        let party = [id(1), id(2)];
        let group = |bodies: &[Target]| {
            super::victims(&aiming(
                Targeting::EnemyGroup,
                Aim::Body(id(4)),
                bodies,
                &party,
                &groups,
            ))
        };
        let reached = group(&bodies).unwrap();
        assert_eq!(reached.victims, [id(4), id(3), id(5)]);
        assert_eq!(reached.spot, Some(Vec3::new(3.0, 0.0, 0.0)));
        bodies[4].dead = true;
        assert_eq!(group(&bodies).unwrap().victims, [id(4), id(3)]);
        // The foe aimed at must itself be reached.
        let far = super::victims(&aiming(
            Targeting::EnemyGroup,
            Aim::Body(id(5)),
            &fight(),
            &party,
            &groups,
        ));
        assert_eq!(far, Err(Refusal::NoTarget));
    }

    /// A group spell cast by a monster at a member takes the whole party.
    #[test]
    fn a_group_of_members_is_the_party() {
        let mut bodies = vec![body(1, 0.0, 0.0), member(2, 0.0, 2.0), member(3, 1.0, 2.0)];
        bodies[1].toward = Stance::Hostile;
        bodies[2].toward = Stance::Hostile;
        let party = [id(2), id(3)];
        let reached =
            super::victims(&aiming(Targeting::EnemyGroup, Aim::Body(id(2)), &bodies, &party, &[]))
                .unwrap();
        // They are all on one side: the friendly.
        assert_eq!(reached.victims, [id(2), id(3)]);
    }

    /// All enemies: every foe reached; none, no target.
    #[test]
    fn all_enemies_are_every_foe_reached() {
        let bodies = fight();
        let all = victims_of(Targeting::AllEnemies, Aim::Ahead, &bodies).unwrap();
        assert_eq!(all, Reached { victims: vec![id(3), id(4)], spot: None });
        let none = [member(1, 0.0, 0.0), Target { reached: false, ..body(5, 0.0, 40.0) }];
        assert_eq!(victims_of(Targeting::AllEnemies, Aim::Ahead, &none), Err(Refusal::NoTarget));
    }

    /// A blast needs a foe to land at but takes its victims when the power is
    /// known.
    #[test]
    fn a_blast_lands_at_the_foes_feet_and_waits_for_its_power() {
        let bodies = fight();
        assert_eq!(
            victims_of(Targeting::Radius, Aim::Body(id(4)), &bodies).unwrap(),
            Reached { victims: vec![], spot: Some(Vec3::new(3.0, 0.0, 0.0)) }
        );
        assert_eq!(
            victims_of(Targeting::Radius, Aim::Body(id(5)), &bodies),
            Err(Refusal::NoTarget)
        );
    }

    /// A cone toward the foe at (0, 0, 3): the apex at the eye (0, 1.7, 0). A body
    /// is inside by heading and elevation each within 45 degrees of the aimed
    /// spot's: 3 yes; 4 at 90 degrees no; a member at (0.5, 0, 2) is heading
    /// atan(0.25) = 14 degrees, inside, friends included; one behind, no; the
    /// caster never; the dead no; out of reach no.
    #[test]
    fn a_cone_takes_every_living_body_reached_inside_its_arcs_friends_too() {
        let mut bodies = fight();
        bodies.push(member(6, 0.5, 2.0));
        bodies.push(body(7, 0.0, -3.0));
        let cone =
            |bodies: &[Target]| victims_of(Targeting::Cone, Aim::Body(id(3)), bodies).unwrap();
        let reached = cone(&bodies);
        assert_eq!(reached.victims, [id(3), id(6)]);
        // The spot is the apex plus the way to the foe's feet: (0, 1.7, 0) + (0, -1.7, 3).
        assert_eq!(reached.spot, Some(Vec3::new(0.0, 0.0, 3.0)));
        bodies[5].dead = true;
        assert_eq!(cone(&bodies).victims, [id(3)]);
        bodies[2].reached = false;
        // A foe out of reach cannot be aimed at.
        assert_eq!(victims_of(Targeting::Cone, Aim::Body(id(3)), &bodies), Err(Refusal::NoTarget));
    }

    #[test]
    fn a_cone_is_the_arcs_of_heading_and_elevation_each_under_forty_five_degrees() {
        let aim = Vec3::new(0.0, 0.0, 10.0);
        assert!(in_cone(aim, Vec3::new(0.0, 0.0, 5.0)));
        assert!(in_cone(aim, Vec3::new(4.0, 0.0, 5.0)), "39 degrees");
        assert!(!in_cone(aim, Vec3::new(6.0, 0.0, 5.0)));
        assert!(!in_cone(aim, Vec3::new(0.0, 0.0, -5.0)));
        // Elevation: 4 over 5 is 38 degrees up, 6 over 5 is 50.
        assert!(in_cone(aim, Vec3::new(0.0, 4.0, 5.0)));
        assert!(!in_cone(aim, Vec3::new(0.0, 6.0, 5.0)));
        // Heading wraps: aimed at -z, a body at slightly the other side of the wrap.
        assert!(in_cone(Vec3::new(-0.1, 0.0, -10.0), Vec3::new(0.1, 0.0, -10.0)));
    }

    #[test]
    fn a_blasts_radius_grows_by_the_power() {
        let spell = SpellDefinition { radius: 2.0, radius_per_power: 1.5, ..Default::default() };
        assert_eq!(blast_radius(&spell, 1), 3.5);
        assert_eq!(blast_radius(&spell, 4), 8.0);
    }

    fn struck(index: u32, stance: Stance, at: f32, clear: bool) -> Struck {
        Struck {
            id: id(index),
            dead: false,
            stance,
            centre: Vec3::new(at, 1.0, 0.0),
            radius: 0.5,
            order: (1, index as usize),
            clear,
        }
    }

    /// A blast at the origin: a body's centre is (at, 1, 0) and its radius 0.5,
    /// so at 3.5 the gap is sqrt(3.5^2 + 1) - 0.5 = 3.14, at 3.6 it is 3.24. Of
    /// radius 3.2 the first is in and the second out. Only the side it is
    /// against; the dead and a body behind a wall are spared; in the order of
    /// the fight.
    #[test]
    fn a_blast_takes_the_living_foes_within_its_radius_in_sight_of_it() {
        let mut bodies = vec![
            struck(9, Stance::Hostile, 3.5, true),
            struck(4, Stance::Hostile, 0.0, true),
            struck(5, Stance::Hostile, 3.6, true),
            struck(6, Stance::Friendly, 0.0, true),
            struck(7, Stance::Hostile, 1.0, false),
            Struck { dead: true, ..struck(8, Stance::Hostile, 1.0, true) },
        ];
        let against = Recipients::Opponents(Stance::Friendly);
        let reached = |radius, bodies: &[Struck]| blasted(Vec3::ZERO, radius, against, bodies);
        assert_eq!(reached(3.2, &bodies), [id(4), id(9)]);
        assert_eq!(reached(3.1, &bodies), [id(4)]);
        // Everyone, when it comes from no one.
        bodies.push(struck(1, Stance::Friendly, 0.0, true));
        let everyone = blasted(Vec3::ZERO, 3.2, Recipients::Everyone, &bodies);
        assert_eq!(everyone, [id(1), id(4), id(6), id(9)]);
        // No radius, no one.
        assert!(reached(0.0, &bodies).is_empty());
    }

    fn turned(
        targeting: Targeting,
        aim: Aim,
        bodies: &[Target],
        groups: &[Vec<Entity>],
    ) -> Backfire {
        let party = [id(1), id(2)];
        let facts = aiming(targeting, aim, bodies, &party, groups);
        backfire(&facts, &own_side(&facts), Some(Vec3::new(0.0, 0.0, 3.0)))
    }

    /// Aimed at one body: it casts it back on the caster, from its eye.
    #[test]
    fn a_backfire_aimed_at_one_body_comes_from_it_on_the_caster() {
        let bodies = fight();
        let back = turned(Targeting::Enemy, Aim::Body(id(3)), &bodies, &[]);
        assert_eq!(
            back,
            Backfire {
                owner: id(3),
                from: Vec3::new(0.0, 1.7, 3.0),
                aim: Aim::Body(id(3)),
                falls: Falls::On(vec![id(1)]),
            }
        );
        let healing = turned(Targeting::Ally, Aim::Body(id(2)), &bodies, &[]);
        assert_eq!((healing.owner, healing.falls), (id(2), Falls::On(vec![id(1)])));
    }

    /// Aimed at a group, its leader (the first to have joined: 4 joined before 3
    /// here) casts it and it falls on the caster's whole group, the dead spared,
    /// however far.
    #[test]
    fn a_backfire_aimed_at_a_group_comes_from_its_leader_on_the_casters_group() {
        let mut bodies = fight();
        bodies[2].group = Some(0);
        bodies[3].group = Some(0);
        bodies[2].joined = 9;
        bodies[3].joined = 2;
        bodies[1].dead = true;
        let groups = [vec![id(3), id(4)]];
        let back = turned(Targeting::EnemyGroup, Aim::Body(id(3)), &bodies, &groups);
        assert_eq!(back.owner, id(4));
        assert_eq!(back.from, Vec3::new(3.0, 1.7, 0.0));
        // The caster is a member: the party, the dead spared.
        assert_eq!(back.falls, Falls::On(vec![id(1)]));
        assert_eq!(back.aim, Aim::Body(id(3)));
    }

    /// A cone or a blast falls on the caster's side where it was to go off, from
    /// the caster; the aim becomes the point.
    #[test]
    fn a_backfire_of_a_cone_or_a_blast_falls_where_it_was_to_go_off() {
        let bodies = fight();
        let cone = turned(Targeting::Cone, Aim::Body(id(3)), &bodies, &[]);
        assert_eq!(cone.owner, id(CASTER));
        assert_eq!(cone.from, Vec3::new(0.0, 0.0, 3.0));
        assert_eq!(cone.aim, Aim::Point(Vec3::new(0.0, 0.0, 3.0)));
        // The caster's side: members reached (the caster, and 2 at 1 m).
        assert_eq!(cone.falls, Falls::On(vec![id(1), id(2)]));
        let blast = turned(Targeting::Radius, Aim::Body(id(3)), &bodies, &[]);
        assert_eq!(blast.falls, Falls::Blast);
        assert_eq!(blast.aim, Aim::Point(Vec3::new(0.0, 0.0, 3.0)));
    }

    /// At all enemies, the party or an item: cast as it was, from the caster's
    /// eye, falling on the caster's own side reached.
    #[test]
    fn a_backfire_of_a_wide_spell_is_cast_as_it_was_on_the_casters_side() {
        let mut bodies = fight();
        bodies[1].reached = false;
        let all = turned(Targeting::AllEnemies, Aim::Ahead, &bodies, &[]);
        assert_eq!(all.owner, id(CASTER));
        assert_eq!(all.from, Vec3::new(0.0, 1.7, 0.0));
        assert_eq!(all.aim, Aim::Ahead);
        assert_eq!(all.falls, Falls::On(vec![id(1)]));
    }

    #[test]
    fn a_missile_flies_at_fifteen_metres_a_second() {
        assert_eq!(flight(Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0)), 2.0);
        assert_eq!(flight(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, 1.0, 1.0)), 0.0);
    }

    fn marking(working: Working, aim: Aim) -> Marking {
        Marking {
            working,
            aim,
            spot: Some((10.0, true)),
            reach: 25.0,
            eye_height: 1.7,
            item: Some(Held { party_has_it: true, itself: false, known: false }),
            portal: true,
            speaker: Some(id(8)),
            elemental: false,
            attempt: Some(Attempt { guarded: true, kind: Device::Lock }),
        }
    }

    #[test]
    fn identify_needs_an_item_the_party_has_that_is_not_itself_nor_known() {
        let identify = |marking: Marking| mark(&marking);
        let item = marking(Working::Identify, Aim::Item(id(4)));
        assert_eq!(identify(item), Ok(Marked::Item(id(4))));
        let held = |held| Marking { item: Some(held), ..item };
        let has = Held { party_has_it: true, itself: false, known: false };
        assert_eq!(identify(held(Held { party_has_it: false, ..has })), Err(Refusal::NoTarget));
        assert_eq!(identify(held(Held { itself: true, ..has })), Err(Refusal::NoTarget));
        assert_eq!(identify(held(Held { known: true, ..has })), Err(Refusal::NoTarget));
        assert_eq!(identify(Marking { item: None, ..item }), Err(Refusal::NoTarget));
        assert_eq!(identify(Marking { aim: Aim::Body(id(4)), ..item }), Err(Refusal::NoTarget));
    }

    #[test]
    fn a_return_to_portal_needs_a_portal_saved_and_a_set_portal_nothing() {
        let back = marking(Working::ReturnToPortal, Aim::Ahead);
        assert_eq!(mark(&back), Ok(Marked::Nothing));
        assert_eq!(mark(&Marking { portal: false, ..back }), Err(Refusal::NoTarget));
        let set = Marking { portal: false, ..marking(Working::SetPortal, Aim::Ahead) };
        assert_eq!(mark(&set), Ok(Marked::Nothing));
    }

    #[test]
    fn charm_and_mindread_are_cast_at_the_one_the_party_is_talking_with() {
        for working in [Working::Charm, Working::Mindread] {
            let at = marking(working, Aim::Body(id(8)));
            assert_eq!(mark(&at), Ok(Marked::Body(id(8))));
            assert_eq!(mark(&Marking { aim: Aim::Body(id(9)), ..at }), Err(Refusal::NoTarget));
            assert_eq!(mark(&Marking { speaker: None, ..at }), Err(Refusal::NoTarget));
        }
    }

    /// A spot within the spell's reach plus an eye height: 25 + 1.7 = 26.7, so
    /// 26 is in and 27 is out.
    #[test]
    fn a_lure_and_a_summoning_need_a_spot_in_reach_and_in_sight() {
        let spot = Vec3::new(0.0, 0.0, 10.0);
        for working in [Working::Lure, Working::Summon] {
            let at = |far, clear, elemental| {
                mark(&Marking {
                    spot: Some((far, clear)),
                    elemental,
                    ..marking(working, Aim::Point(spot))
                })
            };
            assert_eq!(at(26.0, true, false), Ok(Marked::Spot(spot)));
            assert_eq!(at(27.0, true, false), Err(Refusal::NoTarget));
            assert_eq!(at(10.0, false, false), Err(Refusal::NoTarget));
            // Only a summoning is stopped by an elemental that still stands.
            assert_eq!(at(10.0, true, true) == Err(Refusal::NoTarget), working == Working::Summon);
        }
        assert_eq!(
            mark(&marking(Working::Lure, Aim::Body(id(1)))),
            Err(Refusal::NoTarget),
            "a body is not a spot"
        );
    }

    #[test]
    fn a_lock_spell_and_a_trap_spell_need_the_attempt_under_way_on_a_guarded_device() {
        let on = id(6);
        for working in [Working::Knock, Working::DivineTrap] {
            let at = marking(working, Aim::Device(on));
            assert_eq!(mark(&at), Ok(Marked::Attempt(on)));
            let attempt = |attempt| mark(&Marking { attempt, ..at });
            assert_eq!(attempt(None), Err(Refusal::NoTarget));
            assert_eq!(
                attempt(Some(Attempt { guarded: false, kind: Device::Lock })),
                Err(Refusal::NoTarget)
            );
            assert_eq!(
                attempt(Some(Attempt { guarded: true, kind: Device::Container })),
                Err(Refusal::NoTarget)
            );
            assert_eq!(
                attempt(Some(Attempt { guarded: true, kind: Device::Trap { resolved: true } })),
                Ok(Marked::Attempt(on))
            );
            assert_eq!(
                attempt(Some(Attempt { guarded: true, kind: Device::Trap { resolved: false } })),
                Err(Refusal::Unsupported)
            );
            assert_eq!(mark(&Marking { aim: Aim::Ahead, ..at }), Err(Refusal::NoTarget));
        }
    }
}
