//! Whom a spell takes in (`docs/rules/casting.md` section 5 and "How this is
//! built"). An aim is an entity, made by whoever casts or sets off a spell with
//! what its kind of target reads; that kind's taker, set on it by its
//! [`Takes`], fills in its [`Victims`] when it is told to [`Take`], and
//! says it [`Took`]. What follows is whatever its maker set on it.

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;

use super::reach::{Aim, ahead, in_cone};
use crate::game::actors::{Place, Places};
use crate::game::characters::Vitals;
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::world::hostility::{self, Stance, Standing};
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Untargetable};

/// The cast an aim is of; it goes with the cast.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Aims)]
pub struct AimOf(pub Entity);

/// The aims a cast has made.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = AimOf, linked_spawn)]
pub struct Aims(Vec<Entity>);

/// What it was aimed at.
#[derive(Component, Clone, Copy, Debug)]
pub struct At(pub Aim);

/// Whose aim it is: the body it comes from, whose side and place it reads.
/// One with none (a burst from no one, a cloud's pulse) comes from nowhere.
#[derive(Component, Clone, Copy, Debug)]
pub struct Whose(pub Entity);

/// Where it goes off from: what a blast's victims must have a clear line to.
#[derive(Component, Clone, Copy, Debug)]
pub struct Origin(pub Vec3);

/// How far it reaches from whose it is, body to body, in metres.
#[derive(Component, Clone, Copy, Debug)]
pub struct Reach(pub f32);

/// How far a blast spreads from where it lands, in metres.
#[derive(Component, Clone, Copy, Debug)]
pub struct Radius(pub f32);

/// Whom an aim is against.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
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

    /// Whether it is against `body`, standing so, when `whose` (standing
    /// so, magic never targeting it or not) is whose it is.
    fn admits(
        self,
        whose: Option<(Entity, Standing, bool)>,
        (body, theirs): (Entity, Standing),
    ) -> bool {
        match (self, whose) {
            (Self::Opponents(side), _) => side.opposes(theirs.stance),
            (Self::Everyone, _) => true,
            (Self::FoesOf, Some((_, mine, false))) => {
                hostility::between(mine, theirs) == Stance::Hostile
            }
            (Self::FellowsOf, Some((whose, mine, _))) => {
                body == whose || mine.stance == theirs.stance
            }
            (Self::FoesOf | Self::FellowsOf, _) => false,
        }
    }
}

/// Its kind of target, whose taker fills it in.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(on_add = taken_by_kind)]
pub enum Takes {
    /// Whose it is.
    Caster,
    /// The body aimed at.
    One,
    /// The body aimed at, if it is whose the aim is or a friend, and reached.
    Friend,
    /// Every member of the party, the dead too.
    Party,
    /// The foe aimed at, or the one nearest ahead, among those reached.
    Foe,
    /// The body aimed at and the rest of its group of its side, living,
    /// reached or not: the party for a member, its monster group for a
    /// monster.
    Group,
    /// Every living body it is against that whose it is reaches.
    Reached,
    /// Every living body it is against but whose it is, inside the cone
    /// from where it goes off toward its spot.
    Cone,
    /// Every living body it is against within its radius of its spot, with
    /// a clear line from where it went off.
    Blast,
}

fn taken_by_kind(mut world: DeferredWorld, context: HookContext) {
    let Some(&kind) = world.get::<Takes>(context.entity) else { return };
    let mut commands = world.commands();
    let mut aim = commands.entity(context.entity);
    match kind {
        Takes::Caster => aim.observe(caster),
        Takes::One => aim.observe(one),
        Takes::Friend => aim.observe(friend),
        Takes::Party => aim.observe(party),
        Takes::Foe => aim.observe(foe),
        Takes::Group => aim.observe(group),
        Takes::Reached => aim.observe(reached),
        Takes::Cone => aim.observe(cone),
        Takes::Blast => aim.observe(blast),
    };
}

/// Whom it takes in, in the order of the fight: the members by seat, then the
/// monsters as they were placed.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Victims(pub Vec<Entity>);

/// Where it lands: the foe's feet, the spot a cone fans toward, where a
/// blast goes off.
#[derive(Component, Clone, Copy, Debug)]
pub struct Spot(pub Vec3);

/// An aim is to take in whom its kind of target takes.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Take {
    #[event_target]
    pub aim: Entity,
}

/// An aim has taken in its victims. Its maker's observer goes on.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Took {
    #[event_target]
    pub aim: Entity,
}

/// Where a body stands among those magic takes: the members by seat, then
/// the monsters as they were placed (retail walks its monster list,
/// `CollectMonstersWithinRadius`).
fn order_of(body: Entity, roster: &PartyRoster, monsters: &Query<&Monster>) -> (u8, i64) {
    match roster.get_index_of(&body) {
        Some(seat) => (0, seat as i64),
        None => (1, monsters.get(body).map_or(i64::MAX, |monster| i64::from(monster.location))),
    }
}

/// Whether a body at `from` reaches one at `to` within `reach`: body to
/// body, along a clear line between their eyes (retail
/// `CharacterActionReachesTarget`, `SourceActionReachesTarget`).
pub fn reaches(from: &Place, to: &Place, reach: f32, obstacles: &WorldObstacles) -> bool {
    from.gap(to) <= reach && obstacles.clear(from.eye, to.eye)
}

fn took(
    commands: &mut Commands,
    aim: Entity,
    mut victims: Vec<Entity>,
    order: impl Fn(Entity) -> (u8, i64),
) {
    victims.sort_by_key(|&body| order(body));
    commands.entity(aim).insert(Victims(victims));
    commands.trigger(Took { aim });
}

fn caster(take: On<Take>, aims: Query<&Whose>, mut commands: Commands) -> Result {
    let &Whose(whose) = aims.get(take.aim)?;
    took(&mut commands, take.aim, vec![whose], |_| (0, 0));
    Ok(())
}

fn one(take: On<Take>, aims: Query<&At>, mut commands: Commands) -> Result {
    let &At(at) = aims.get(take.aim)?;
    took(&mut commands, take.aim, at.body().into_iter().collect(), |_| (0, 0));
    Ok(())
}

#[allow(clippy::type_complexity)]
fn friend(
    take: On<Take>,
    aims: Query<(&Whose, &At, &Reach)>,
    bodies: Query<&Standing, (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>)>,
    places: Places,
    obstacles: WorldObstacles,
    mut commands: Commands,
) -> Result {
    let (&Whose(whose), &At(at), &Reach(reach)) = aims.get(take.aim)?;
    let friend = at.body().filter(|&aimed| {
        let (Ok(&mine), Ok(&theirs)) = (bodies.get(whose), bodies.get(aimed)) else { return false };
        (aimed == whose || hostility::between(mine, theirs) == Stance::Friendly)
            && reaches(&places.of(whose), &places.of(aimed), reach, &obstacles)
    });
    took(&mut commands, take.aim, friend.into_iter().collect(), |_| (0, 0));
    Ok(())
}

fn party(take: On<Take>, roster: Single<&PartyRoster>, mut commands: Commands) {
    took(&mut commands, take.aim, roster.iter().collect(), |_| (0, 0));
}

/// The foe an aim takes among the living foes whose it is reaches: the one
/// aimed at, or for an automatic aim the one nearest its heading, the nearer
/// of equals. Its feet are the aim's spot.
#[allow(clippy::type_complexity)]
fn foe(
    take: On<Take>,
    aims: Query<(&Whose, &At, &Reach)>,
    bodies: Query<
        (Entity, &Vitals, &Standing),
        (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>),
    >,
    whose_stand: Query<(&Standing, Has<Untargetable>)>,
    (places, obstacles): (Places, WorldObstacles),
    mut commands: Commands,
) -> Result {
    let (&Whose(whose), &At(at), &Reach(reach)) = aims.get(take.aim)?;
    let mine = whose_stand.get(whose).ok().map(|(&standing, never)| (whose, standing, never));
    let from = places.of(whose);
    let foes: Vec<(Entity, Place)> = bodies
        .iter()
        .filter(|&(body, vitals, &theirs)| {
            body != whose && !vitals.is_dead() && Against::FoesOf.admits(mine, (body, theirs))
        })
        .map(|(body, ..)| (body, places.of(body)))
        .filter(|(_, there)| reaches(&from, there, reach, &obstacles))
        .collect();
    let chosen = match at {
        Aim::Ahead => ahead(
            from.eye,
            from.forward(),
            f32::INFINITY,
            foes.iter().map(|(foe, there)| (*foe, there.feet, from.gap(there))),
        ),
        at => at.body().filter(|aimed| foes.iter().any(|(foe, _)| foe == aimed)),
    };
    if let Some(chosen) = chosen {
        commands.entity(take.aim).insert((At(Aim::Body(chosen)), Spot(places.of(chosen).feet)));
    }
    took(&mut commands, take.aim, chosen.into_iter().collect(), |_| (0, 0));
    Ok(())
}

#[allow(clippy::type_complexity)]
fn group(
    take: On<Take>,
    aims: Query<&At>,
    bodies: Query<
        (&Vitals, &Standing),
        (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>),
    >,
    groups: (Query<&GroupMember>, Query<&GroupMembers>),
    (roster, monsters): (Single<&PartyRoster>, Query<&Monster>),
    mut commands: Commands,
) -> Result {
    let &At(at) = aims.get(take.aim)?;
    let Some(first) = at.body() else {
        took(&mut commands, take.aim, vec![], |_| (0, 0));
        return Ok(());
    };
    let side = bodies.get(first).map(|(_, standing)| standing.stance).ok();
    let group: Vec<Entity> = match roster.contains(&first) {
        true => roster.iter().collect(),
        false => groups
            .0
            .get(first)
            .and_then(|member| groups.1.get(member.0))
            .map_or_else(|_| vec![first], |members| members.iter().collect()),
    };
    let mates = group.into_iter().filter(|&mate| {
        mate != first
            && bodies
                .get(mate)
                .is_ok_and(|(vitals, standing)| !vitals.is_dead() && Some(standing.stance) == side)
    });
    let mut victims: Vec<Entity> = mates.collect();
    victims.sort_by_key(|&mate| order_of(mate, *roster, &monsters));
    victims.insert(0, first);
    commands.entity(take.aim).insert(Victims(victims));
    commands.trigger(Took { aim: take.aim });
    Ok(())
}

#[allow(clippy::type_complexity)]
fn reached(
    take: On<Take>,
    aims: Query<(&Whose, &Reach, &Against)>,
    bodies: Query<
        (Entity, &Vitals, &Standing),
        (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>),
    >,
    whose_stand: Query<(&Standing, Has<Untargetable>)>,
    (roster, monsters): (Single<&PartyRoster>, Query<&Monster>),
    (places, obstacles): (Places, WorldObstacles),
    mut commands: Commands,
) -> Result {
    let (&Whose(whose), &Reach(reach), &against) = aims.get(take.aim)?;
    let mine = whose_stand.get(whose).ok().map(|(&standing, never)| (whose, standing, never));
    let from = places.of(whose);
    let victims = bodies
        .iter()
        .filter(|&(body, vitals, &theirs)| {
            !vitals.is_dead() && against.admits(mine, (body, theirs))
        })
        .map(|(body, ..)| body)
        .filter(|&body| body == whose || reaches(&from, &places.of(body), reach, &obstacles))
        .collect();
    took(&mut commands, take.aim, victims, |body| order_of(body, *roster, &monsters));
    Ok(())
}

/// Inside a cone (retail `TargetInRangeAndArcs`): its body within the reach
/// of where it goes off, both bodies' radii taken off and the rest rounded
/// down, a clear line to its eye, and its feet within the arcs toward the
/// spot.
#[allow(clippy::type_complexity)]
fn cone(
    take: On<Take>,
    aims: Query<(Option<&Whose>, &Origin, &Spot, &Reach, &Against)>,
    bodies: Query<
        (Entity, &Vitals, &Standing),
        (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>),
    >,
    whose_stand: Query<(&Standing, Has<Untargetable>)>,
    (roster, monsters): (Single<&PartyRoster>, Query<&Monster>),
    (places, obstacles): (Places, WorldObstacles),
    mut commands: Commands,
) -> Result {
    let (whose, &Origin(apex), &Spot(spot), &Reach(reach), &against) = aims.get(take.aim)?;
    let whose = whose.map(|whose| whose.0);
    let mine = whose.and_then(|whose| {
        whose_stand.get(whose).ok().map(|(&standing, never)| (whose, standing, never))
    });
    let apex_radius = whose.map_or(0.0, |whose| places.of(whose).radius);
    let victims = bodies
        .iter()
        .filter(|&(body, vitals, &theirs)| {
            Some(body) != whose && !vitals.is_dead() && against.admits(mine, (body, theirs))
        })
        .map(|(body, ..)| body)
        .filter(|&body| {
            let there = places.of(body);
            (there.centre.distance(apex) - there.radius - apex_radius).floor() <= reach
                && obstacles.clear(apex, there.eye)
                && in_cone(spot - apex, there.feet - apex)
        })
        .collect();
    took(&mut commands, take.aim, victims, |body| order_of(body, *roster, &monsters));
    Ok(())
}

/// Those it is against whose body reaches within its radius of its spot,
/// with a clear line from where it went off (retail
/// `CollectMonstersWithinRadius`).
#[allow(clippy::type_complexity)]
fn blast(
    take: On<Take>,
    aims: Query<(Option<&Whose>, &Spot, &Origin, &Radius, &Against)>,
    bodies: Query<
        (Entity, &Vitals, &Standing),
        (Or<(With<Monster>, With<PartyMember>)>, Without<Untargetable>),
    >,
    whose_stand: Query<(&Standing, Has<Untargetable>)>,
    (roster, monsters): (Single<&PartyRoster>, Query<&Monster>),
    (places, obstacles): (Places, WorldObstacles),
    mut commands: Commands,
) -> Result {
    let (whose, &Spot(centre), &Origin(origin), &Radius(radius), &against) = aims.get(take.aim)?;
    let mine = whose.and_then(|&Whose(whose)| {
        whose_stand.get(whose).ok().map(|(&standing, never)| (whose, standing, never))
    });
    let victims = match radius > 0.0 {
        false => vec![],
        true => bodies
            .iter()
            .filter(|&(body, vitals, &theirs)| {
                !vitals.is_dead() && against.admits(mine, (body, theirs))
            })
            .map(|(body, ..)| body)
            .filter(|&body| {
                let there = places.of(body);
                there.centre.distance(centre) - there.radius <= radius
                    && obstacles.clear(origin, there.centre)
            })
            .collect(),
    };
    took(&mut commands, take.aim, victims, |body| order_of(body, *roster, &monsters));
    Ok(())
}
