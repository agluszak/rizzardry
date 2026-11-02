//! What follows once all of a spell has landed: a drain's share
//! ([`drain_life`]), Boiling Blood's bursts ([`burst_blood`]) and Eye for an
//! Eye ([`reflect_spells`]), each over the [`Spell`] its act carries.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::prototypes::{Definitions, Reach, Uses};
use super::reach::Recipients;
use super::{Body, BodyReadOnlyItem, Deed, DeedHits, Effect, HitOf, Strikes};
use crate::content::spells::{SpellId, Targeting};
use crate::game::data::Catalogs;
use crate::game::hits::effects::InFlight;
use crate::game::magic::Strength;
use crate::game::magic::casting::{Aim, MISSILE_SPEED, in_cone};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::level::Level;
use crate::game::world::obstacles::WorldObstacles;

/// The share of Lifesteal's damage that heals the most wounded party member,
/// in percent.
const LIFESTEAL_SHARE: u32 = 70;

/// A deed of Lifesteal or Might to Magic: what its damage drained, to give
/// to its caster's side once all of it has landed.
#[derive(Component, Debug)]
pub struct Drains {
    /// A monster caster heals itself; a member's drain goes to the party.
    pub caster: Entity,
    pub lifesteal: bool,
    pub drained: u32,
}

/// The spell a cast's act carries, for what follows once all of it has
/// landed: whose and where from, aimed how, at whom; turned on its caster's
/// side when it `backfired`, and not to be turned back once `reflected`.
#[derive(Component, Clone, Debug)]
pub struct Spell {
    pub id: SpellId,
    pub caster: Entity,
    pub backfired: bool,
    pub reflected: bool,
    /// Whose side and level it carries: its caster, or the monster a
    /// backfire comes from; none for a point with no owner.
    pub owner: Option<Entity>,
    /// Where it set out from.
    pub from: Vec3,
    pub aim: Aim,
    pub victims: Vec<Entity>,
}

/// Spawn an act that reaches `victims`, each after its missile's flight.
pub fn reach(commands: &mut Commands, act: Entity, victims: &[(Entity, Option<f32>)]) {
    for &(victim, flight) in victims {
        let mut reach = commands.spawn((Reach, HitOf(act), Strikes(victim)));
        if let Some(seconds) = flight {
            reach.insert(InFlight(Timer::from_seconds(seconds, TimerMode::Once)));
        }
    }
}

/// Once a Lifesteal or Might to Magic deed's hits have landed: a monster
/// caster takes 70% of the drained health back, or half of it as spell
/// points; a member's drain goes, once, to the party member most in need
/// (retail chooses after every victim has been drained).
pub fn drain_life(
    mut deeds: Query<(&mut Deed, &Drains), Without<DeedHits>>,
    mut bodies: Query<Body>,
    roster: Single<&PartyRoster>,
    content: If<Res<crate::game::data::Catalogs>>,
) {
    for (mut deed, drains) in &mut deeds {
        let Ok(mut caster) = bodies.get_mut(drains.caster) else { continue };
        if let Some(monster) = caster.monster.as_mut() {
            if drains.lifesteal {
                caster.vitals.heal(drains.drained * 7 / 10);
            } else {
                monster.spell_points += (drains.drained / 2) as i32;
            }
            continue;
        }
        let need = |body: &BodyReadOnlyItem| -> Option<u32> {
            if body.is_dead() {
                return None;
            }
            let percent = if drains.lifesteal {
                body.vitals.health.current * 100 / body.vitals.health.max
            } else {
                let maxima = body.mana_maximum(&content.spells)?;
                let maximum: u32 = maxima.values().sum();
                if maximum == 0 {
                    return None;
                }
                let (_, pool) = body.magic.as_ref()?;
                maxima.iter().map(|(realm, _)| pool.available(realm, &maxima)).sum::<u32>() * 100
                    / maximum
            };
            (percent < 100).then_some(percent)
        };
        let neediest = roster
            .iter()
            .filter_map(|member| need(&bodies.get(member).ok()?).map(|p| (member, p)))
            .min_by_key(|&(_, percent)| percent)
            .map(|(member, _)| member);
        let Some(member) = neediest else { continue };
        let mut body = bodies.get_mut(member).unwrap();
        if drains.lifesteal {
            let amount = body.vitals.heal(drains.drained * LIFESTEAL_SHARE / 100);
            deed.effects.push(Effect::Healed { victim: member, amount });
        } else {
            let (_, pool) = body.magic.as_mut().unwrap();
            let amount = pool.restore_evenly(drains.drained);
            deed.effects.push(Effect::Replenished { victim: member, amount });
        }
    }
}

/// Once a Boiling Blood cast has landed, each of its victims that died
/// bursts where it fell, at the cast's power, from the spell's owner
/// (retail's Boiling Blood explosion): an act of its own, reaching whoever
/// stands within its radius.
pub fn burst_blood(
    mut commands: Commands,
    acts: Query<(&Spell, &Strength), (With<Deed>, Without<DeedHits>)>,
    mut bodies: Query<Body>,
    levels: Query<(), With<Level>>,
    party: Single<(&Position, &PartyRoster), With<PartyBody>>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    obstacles: WorldObstacles,
) {
    let (species, explosion) =
        (&content.species, &content.spells[SpellId::BOILING_BLOOD_EXPLOSION]);
    let Some(uses) = definitions.spell(explosion.id) else { return };
    let (position, roster) = *party;
    let eye = PartyBody::eye(position);
    for (spell, strength) in &acts {
        if spell.id != SpellId::BOILING_BLOOD {
            continue;
        }
        let owner = spell.owner.and_then(|owner| bodies.get(owner).ok());
        let figure = owner.as_ref().map_or(0, |owner| owner.figure(explosion, species));
        let recipients = match owner.map(|owner| owner.side.0) {
            Some(side) if spell.backfired => Recipients::Opponents(side).reversed(),
            Some(side) => Recipients::Opponents(side),
            None => Recipients::Everyone,
        };
        let strength = Strength::at(figure, explosion, strength.power, strength.boost);
        let radius = explosion.radius + explosion.radius_per_power * strength.power as f32;
        for &victim in &spell.victims {
            let Ok(body) = bodies.get(victim) else { continue };
            if !body.is_dead() {
                continue;
            }
            let centre = body.feet(eye);
            let victims: Vec<_> = super::reach::radius(
                centre,
                centre,
                radius,
                recipients,
                Some((eye, roster)),
                &mut bodies,
                &levels,
                species,
                &obstacles,
            )
            .into_iter()
            .map(|victim| (victim, None))
            .collect();
            let act = commands
                .spawn((
                    Deed::new(super::Act::Missile { caster: spell.caster, spell: explosion.id }),
                    Uses(uses),
                    strength,
                ))
                .id();
            reach(&mut commands, act, &victims);
        }
    }
}

/// A monster's group, itself when it has none.
fn group_of(
    entity: Entity,
    memberships: &Query<&GroupMember>,
    groups: &Query<&GroupMembers>,
) -> Vec<Entity> {
    memberships
        .get(entity)
        .ok()
        .and_then(|member| groups.get(member.0).ok())
        .map_or_else(|| vec![entity], |members| members.iter().collect())
}

/// The first placed of a monster's group, whom a group spell turned back
/// comes from.
pub fn leader(
    entity: Entity,
    bodies: &mut Query<Body>,
    memberships: &Query<&GroupMember>,
    groups: &Query<&GroupMembers>,
) -> Entity {
    group_of(entity, memberships, groups)
        .into_iter()
        .min_by_key(|&member| {
            bodies
                .get_mut(member)
                .ok()
                .and_then(|body| body.monster.as_ref().map(|monster| monster.location))
                .unwrap_or(i32::MAX)
        })
        .unwrap_or(entity)
}

/// Once a spell aimed at enemies has landed, the strongest Eye for an Eye
/// among its surviving victims (members first, then monsters in placement
/// order) pays a charge to turn it back on its source (retail
/// `FinishSpellEffectTargets`): an aimed spell on its owner, a group spell
/// on the owner's side, a blast, cone or all-enemies spell on whoever of
/// that side stands where it went off. The reflection is an act of its
/// own, flying back if the spell is a missile, and is not turned back again.
pub fn reflect_spells(
    mut commands: Commands,
    mut acts: Query<(&Spell, &Strength, &mut Deed), Without<DeedHits>>,
    mut bodies: Query<Body>,
    levels: Query<(), With<Level>>,
    memberships: Query<&GroupMember>,
    group_members: Query<&GroupMembers>,
    party: Single<(&Position, &PartyRoster), With<PartyBody>>,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    obstacles: WorldObstacles,
) {
    let (species, spells) = (&content.species, &content.spells);
    let (position, roster) = *party;
    let eye = PartyBody::eye(position);
    let party: Vec<Entity> = roster.iter().collect();

    for (cast, &strength, mut deed) in &mut acts {
        let spell = &spells[cast.id];
        let Some(uses) = definitions.spell(cast.id) else { continue };
        if cast.backfired || cast.reflected || !spell.aimed_at_enemies() {
            continue;
        }
        let mut best = None;
        let mut order = cast.victims.clone();
        order.sort_by_key(|&victim| match party.iter().position(|&member| member == victim) {
            Some(seat) => (0, seat as i32),
            None => (
                1,
                bodies
                    .get_mut(victim)
                    .ok()
                    .and_then(|body| body.monster.as_ref().map(|monster| monster.location))
                    .unwrap_or(i32::MAX),
            ),
        });
        for victim in order {
            let Ok(body) = bodies.get(victim) else { continue };
            if body.is_dead() {
                continue;
            }
            if let Some(charge) = body.vitals.enchantments.get(SpellId::EYE_FOR_AN_EYE)
                && charge.power > best.map_or(0, |(_, power)| power)
            {
                best = Some((victim, charge.power));
            }
        }
        let Some((charged, power)) = best else { continue };
        bodies.get_mut(charged).unwrap().vitals.enchantments.spend_charge(SpellId::EYE_FOR_AN_EYE);
        deed.effects.push(Effect::Reflected { victim: charged });
        let owner = cast.owner;
        let owner_is_member = owner.is_some_and(|owner| party.contains(&owner));
        let eye_of = |actor: Entity, bodies: &mut Query<Body>| {
            bodies.get(actor).map_or(eye, |body| body.eye(eye))
        };
        // Whom and where it comes back from.
        let (source, from) = match (spell.targeting, cast.aim) {
            (Targeting::Radius | Targeting::Cone, Aim::Point(at)) => (owner, at),
            (Targeting::Radius | Targeting::Cone | Targeting::AllEnemies, _) => (owner, cast.from),
            (Targeting::EnemyGroup, Aim::Monster(entity)) => {
                let leader = leader(entity, &mut bodies, &memberships, &group_members);
                (Some(leader), eye_of(leader, &mut bodies))
            }
            (_, Aim::Monster(entity)) => (Some(entity), eye_of(entity, &mut bodies)),
            _ => {
                let first = party
                    .iter()
                    .copied()
                    .find(|&member| bodies.get(member).is_ok_and(|b| b.vitals.can_fight()))
                    .unwrap_or(cast.caster);
                (Some(first), eye)
            }
        };
        let recipients = match owner.and_then(|owner| bodies.get(owner).ok()) {
            Some(body) => Recipients::Opponents(body.side.0).reversed(),
            None => Recipients::Everyone,
        };
        let within = |(centre, origin, radius): (Vec3, Vec3, f32), bodies: &mut Query<Body>| {
            super::reach::radius(
                centre,
                origin,
                radius,
                recipients,
                Some((eye, roster)),
                bodies,
                &levels,
                species,
                &obstacles,
            )
        };
        let victims = match spell.targeting {
            Targeting::Enemy => owner.into_iter().collect(),
            Targeting::EnemyGroup => match owner {
                Some(_) if owner_is_member => party
                    .iter()
                    .copied()
                    .filter(|&member| bodies.get(member).is_ok_and(|b| !b.is_dead()))
                    .collect(),
                Some(owner) => group_of(owner, &memberships, &group_members),
                None => vec![],
            },
            Targeting::Radius => {
                let centre = owner
                    .and_then(|owner| bodies.get(owner).ok())
                    .map_or(PartyBody::feet_below(eye), |body| body.feet(eye));
                let radius =
                    spell.radius + spell.radius_per_power * strength.power.min(power) as f32;
                within((centre, cast.from, radius), &mut bodies)
            }
            Targeting::Cone => within((from, from, spell.reach.metres()), &mut bodies)
                .into_iter()
                .filter(|&victim| in_cone(cast.from - from, eye_of(victim, &mut bodies) - from))
                .collect(),
            Targeting::AllEnemies => {
                within((cast.from, cast.from, spell.reach.metres()), &mut bodies)
            }
            _ => vec![],
        };
        let figure = source
            .and_then(|source| bodies.get(source).ok())
            .map_or(0, |body| body.figure(spell, species));
        let strength = Strength::at(figure, spell, strength.power.min(power), strength.boost);
        let aim = match owner {
            Some(owner) if owner_is_member => Aim::Member(owner),
            Some(owner) => Aim::Monster(owner),
            None => Aim::Point(cast.from),
        };
        let flights: Vec<_> = victims
            .iter()
            .map(|&victim| {
                let flight = spell
                    .missile
                    .map(|_| from.distance(eye_of(victim, &mut bodies)) / MISSILE_SPEED);
                (victim, flight)
            })
            .collect();
        let act = commands
            .spawn((
                Deed::new(super::Act::Missile { caster: cast.caster, spell: spell.id }),
                Uses(uses),
                strength,
                Spell {
                    id: spell.id,
                    caster: cast.caster,
                    backfired: false,
                    reflected: true,
                    owner: source,
                    from,
                    aim,
                    victims,
                },
            ))
            .id();
        reach(&mut commands, act, &flights);
    }
}
