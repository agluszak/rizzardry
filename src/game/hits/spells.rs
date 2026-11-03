//! What follows once all of a spell has landed: a drain's share
//! ([`drain_back`]), Boiling Blood's bursts ([`burst_blood`]) and Eye for
//! an Eye ([`reflect_spells`]), each over the [`Spell`] its act carries;
//! and the strength of an act that comes from someone other than its
//! caster ([`OwnersStrength`]).

use bevy::prelude::*;

use super::effects::{InFlight, Reach};
use super::prototypes::{Definitions, Uses};
use super::reach::{Recipients, within};
use super::{Deed, DeedHits, Effect, HitOf, Strikes};
use crate::content::spells::{SpellDefinition, SpellId, Targeting};
use crate::game::characters::{Character, Vitals};
use crate::game::data::Catalogs;
use crate::game::magic::casting::{Aim, MISSILE_SPEED, in_cone};
use crate::game::magic::{LearnedSpells, SpellPoints, Strength};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::world::footing::Footing;
use crate::game::world::hostility::Side;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Monster, Untargetable};

/// The share of Lifesteal's damage that heals the most wounded party member,
/// in percent.
const LIFESTEAL_SHARE: u32 = 70;

/// A deed of Lifesteal or Might to Magic: what its damage drained, to give
/// to its caster's side once all of it has landed.
#[derive(Component, Debug)]
pub struct Drains {
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
    /// Whose side and level it carries: its caster, or the body a backfire
    /// or reflection comes from; none for a point with no owner.
    pub owner: Option<Entity>,
    /// Where it set out from.
    pub from: Vec3,
    pub aim: Aim,
    pub victims: Vec<Entity>,
}

/// The act is as strong as the body it comes from, not its caster: a
/// backfire, a reflection or a burst. Its [`Strength`] has its power and
/// boost; what it is resisted against follows from its owner's caster
/// level in the spell, settled as it starts to land.
#[derive(Component, Clone, Copy, Debug)]
pub struct OwnersStrength {
    pub owner: Option<Entity>,
    pub spell: SpellId,
}

/// What a spell's strength from a body grows from: a member's caster level
/// in the spell's books, a monster's species' level, nothing from anyone
/// else.
pub fn caster_figure(
    (character, monster): (Option<&Character>, Option<&Monster>),
    spell: &SpellDefinition,
    species: &crate::content::species::MonsterCatalog,
) -> u32 {
    match (character, monster) {
        (Some(character), _) => {
            crate::game::magic::total_caster_level(character, spell.books).max(0) as u32
        }
        (None, Some(monster)) => u32::from(species[monster.species].level),
        (None, None) => 0,
    }
}

/// An act's strength is settled by its owner's figure.
pub fn strength_of_owners(
    mut commands: Commands,
    mut acts: Query<(Entity, &OwnersStrength, &mut Strength)>,
    bodies: Query<(Option<&Character>, Option<&Monster>)>,
    content: If<Res<Catalogs>>,
) {
    for (act, owned, mut strength) in &mut acts {
        let spell = &content.spells[owned.spell];
        let figure = owned
            .owner
            .and_then(|owner| bodies.get(owner).ok())
            .map_or(0, |owner| caster_figure(owner, spell, &content.species));
        *strength = Strength::at(figure, spell, strength.power, strength.boost);
        commands.entity(act).remove::<OwnersStrength>();
    }
}

/// Spawn an act's reaches on `victims`, each after its missile's flight.
pub fn reach(commands: &mut Commands, act: Entity, victims: &[(Entity, Option<f32>)]) {
    for &(victim, flight) in victims {
        let mut reach = commands.spawn((Reach::default(), HitOf(act), Strikes(victim)));
        if let Some(seconds) = flight {
            reach.insert(InFlight(Timer::from_seconds(seconds, TimerMode::Once)));
        }
    }
}

/// Once a Lifesteal or Might to Magic deed's hits have landed, what it
/// drained goes to the caster's side: a party caster's to the member most
/// in need of it (retail chooses after every victim has been drained), a
/// monster's to itself, 70% of the health back or half of it as spell
/// points.
pub fn drain_back(
    mut deeds: Query<(&mut Deed, &Drains), Without<DeedHits>>,
    casters: Query<Has<PartyMember>>,
    roster: Single<&PartyRoster>,
    mut bodies: Query<&mut Vitals>,
    mut pools: Query<(&Character, &LearnedSpells, &mut SpellPoints)>,
    mut budgets: Query<&mut Monster>,
    content: If<Res<Catalogs>>,
) {
    for (mut deed, drains) in &mut deeds {
        let Ok(member) = casters.get(drains.caster) else { continue };
        let candidates: Vec<Entity> =
            if member { roster.iter().collect() } else { vec![drains.caster] };
        let need = |victim: Entity| -> Option<u32> {
            let vitals = bodies.get(victim).ok()?;
            if vitals.is_dead() {
                return None;
            }
            let percent = if drains.lifesteal {
                vitals.health.current * 100 / vitals.health.max
            } else if let Ok((character, learned, pool)) = pools.get(victim) {
                let maxima =
                    character.stats(&vitals.enchantments).max_points(learned, &content.spells);
                let maximum: u32 = maxima.values().sum();
                if maximum == 0 {
                    return None;
                }
                maxima.iter().map(|(realm, _)| pool.available(realm, &maxima)).sum::<u32>() * 100
                    / maximum
            } else {
                0
            };
            (percent < 100).then_some(percent)
        };
        let neediest = candidates
            .into_iter()
            .filter_map(|victim| need(victim).map(|percent| (victim, percent)))
            .min_by_key(|&(_, percent)| percent)
            .map(|(victim, _)| victim);
        let Some(victim) = neediest else { continue };
        if drains.lifesteal {
            let amount =
                bodies.get_mut(victim).unwrap().heal(drains.drained * LIFESTEAL_SHARE / 100);
            deed.effects.push(Effect::Healed { victim, amount });
        } else if let Ok((.., mut pool)) = pools.get_mut(victim) {
            let amount = pool.restore_evenly(drains.drained);
            deed.effects.push(Effect::Replenished { victim, amount });
        } else if let Ok(mut monster) = budgets.get_mut(victim) {
            monster.spell_points += (drains.drained / 2) as i32;
        }
    }
}

/// Once a Boiling Blood cast has landed, each of its victims that died
/// bursts where it fell, at the cast's power, from the spell's owner
/// (retail's Boiling Blood explosion): an act of its own, reaching whoever
/// stands within its radius, as strong as its owner.
pub fn burst_blood(
    mut commands: Commands,
    acts: Query<(&Spell, &Strength), (With<Deed>, Without<DeedHits>)>,
    bodies: Query<(Entity, &Vitals, &Side, &Footing, Option<&Monster>), Without<Untargetable>>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
) {
    let Some(explosion) = content.spells.get(SpellId::BOILING_BLOOD_EXPLOSION) else { return };
    let Some(uses) = definitions.spell(explosion.id) else { return };
    for (spell, strength) in &acts {
        if spell.id != SpellId::BOILING_BLOOD {
            continue;
        }
        let side =
            spell.owner.and_then(|owner| bodies.get(owner).ok()).map(|(_, _, side, ..)| side.0);
        let recipients = match side {
            Some(side) if spell.backfired => Recipients::Opponents(side).reversed(),
            Some(side) => Recipients::Opponents(side),
            None => Recipients::Everyone,
        };
        let radius = explosion.radius + explosion.radius_per_power * strength.power as f32;
        for &victim in &spell.victims {
            let Ok((_, vitals, _, at, _)) = bodies.get(victim) else { continue };
            if !vitals.is_dead() {
                continue;
            }
            let victims: Vec<_> =
                within(at.feet, at.feet, radius, recipients, &bodies, &roster, &obstacles)
                    .into_iter()
                    .map(|victim| (victim, None))
                    .collect();
            let act = commands
                .spawn((
                    Deed::new(super::Act::Missile { caster: spell.caster, spell: explosion.id }),
                    Uses(uses),
                    Strength { power: strength.power, against: 0, boost: strength.boost },
                    OwnersStrength { owner: spell.owner, spell: explosion.id },
                ))
                .id();
            reach(&mut commands, act, &victims);
        }
    }
}

/// A body's group: the party for a member, its monster group for a
/// monster, itself for anyone else.
pub fn group_of(
    entity: Entity,
    roster: &PartyRoster,
    memberships: &Query<&GroupMember>,
    groups: &Query<&GroupMembers>,
) -> Vec<Entity> {
    if roster.contains(&entity) {
        return roster.iter().collect();
    }
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
    monsters: &Query<&Monster>,
    memberships: &Query<&GroupMember>,
    groups: &Query<&GroupMembers>,
) -> Entity {
    memberships
        .get(entity)
        .ok()
        .and_then(|member| groups.get(member.0).ok())
        .map_or_else(|| vec![entity], |members| members.iter().collect())
        .into_iter()
        .min_by_key(|&member| monsters.get(member).map_or(i32::MAX, |monster| monster.location))
        .unwrap_or(entity)
}

/// Once a spell aimed at enemies has landed, the strongest Eye for an Eye
/// among its surviving victims (members first, then monsters in placement
/// order) pays a charge to turn it back on its source (retail
/// `FinishSpellEffectTargets`): an aimed spell on its owner, a group spell
/// on the owner's group, a blast, cone or all-enemies spell on whoever of
/// that side stands where it went off. The reflection is an act of its
/// own, as strong as its new source, flying back if the spell is a
/// missile, and is not turned back again.
#[allow(clippy::type_complexity)]
pub fn reflect_spells(
    mut commands: Commands,
    mut acts: Query<(&Spell, &Strength, &mut Deed), Without<DeedHits>>,
    mut bodies: Query<
        (Entity, &mut Vitals, &Side, &Footing, Option<&Monster>),
        Without<Untargetable>,
    >,
    monsters: Query<&Monster>,
    memberships: Query<&GroupMember>,
    group_members: Query<&GroupMembers>,
    roster: Single<&PartyRoster>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
) {
    let spells = &content.spells;
    let party_at = roster.iter().find_map(|member| bodies.get(member).ok().map(|(.., at, _)| *at));
    for (cast, &strength, mut deed) in &mut acts {
        let spell = &spells[cast.id];
        let Some(uses) = definitions.spell(cast.id) else { continue };
        if cast.backfired || cast.reflected || !spell.aimed_at_enemies() {
            continue;
        }
        let mut order = cast.victims.clone();
        order.sort_by_key(|&victim| match roster.get_index_of(&victim) {
            Some(seat) => (0, seat as i32),
            None => (1, monsters.get(victim).map_or(i32::MAX, |monster| monster.location)),
        });
        let best = order
            .into_iter()
            .filter_map(|victim| {
                let (_, vitals, ..) = bodies.get(victim).ok()?;
                let charge = vitals.enchantments.get(SpellId::EYE_FOR_AN_EYE)?;
                (!vitals.is_dead()).then_some((victim, charge.power))
            })
            .fold(None, |best: Option<(Entity, u32)>, (victim, power)| match best {
                Some((_, most)) if most >= power => best,
                _ => Some((victim, power)),
            });
        let Some((charged, power)) = best else { continue };
        bodies.get_mut(charged).unwrap().1.enchantments.spend_charge(SpellId::EYE_FOR_AN_EYE);
        deed.effects.push(Effect::Reflected { victim: charged });
        let owner = cast.owner;
        let owner_is_member = owner.is_some_and(|owner| roster.contains(&owner));
        let eye = party_at.map_or(cast.from, |at| at.eye);
        let eye_of = |actor: Entity| bodies.get(actor).map_or(eye, |(.., at, _)| at.eye);
        // Whom and where it comes back from.
        let (source, from) = match (spell.targeting, cast.aim) {
            (Targeting::Radius | Targeting::Cone, Aim::Point(at)) => (owner, at),
            (Targeting::Radius | Targeting::Cone | Targeting::AllEnemies, _) => (owner, cast.from),
            (Targeting::EnemyGroup, Aim::Monster(entity)) => {
                let leader = leader(entity, &monsters, &memberships, &group_members);
                (Some(leader), eye_of(leader))
            }
            (_, Aim::Monster(entity)) => (Some(entity), eye_of(entity)),
            _ => {
                let first = roster
                    .iter()
                    .find(|&member| bodies.get(member).is_ok_and(|(_, v, ..)| v.can_fight()))
                    .unwrap_or(cast.caster);
                (Some(first), eye)
            }
        };
        let recipients = match owner.and_then(|owner| bodies.get(owner).ok()) {
            Some((_, _, side, ..)) => Recipients::Opponents(side.0).reversed(),
            None => Recipients::Everyone,
        };
        let readonly = bodies.as_readonly();
        let victims = match spell.targeting {
            Targeting::Enemy => owner.into_iter().collect(),
            Targeting::EnemyGroup => match owner {
                Some(owner) => group_of(owner, &roster, &memberships, &group_members)
                    .into_iter()
                    .filter(|&body| readonly.get(body).is_ok_and(|(_, v, ..)| !v.is_dead()))
                    .collect(),
                None => vec![],
            },
            Targeting::Radius => {
                let centre = owner
                    .and_then(|owner| readonly.get(owner).ok())
                    .map_or(party_at.map_or(cast.from, |at| at.feet), |(.., at, _)| at.feet);
                let radius =
                    spell.radius + spell.radius_per_power * strength.power.min(power) as f32;
                within(centre, cast.from, radius, recipients, &readonly, &roster, &obstacles)
            }
            Targeting::Cone => {
                within(from, from, spell.reach.metres(), recipients, &readonly, &roster, &obstacles)
                    .into_iter()
                    .filter(|&victim| in_cone(cast.from - from, eye_of(victim) - from))
                    .collect()
            }
            Targeting::AllEnemies => within(
                cast.from,
                cast.from,
                spell.reach.metres(),
                recipients,
                &readonly,
                &roster,
                &obstacles,
            ),
            _ => vec![],
        };
        let aim = match owner {
            Some(owner) if owner_is_member => Aim::Member(owner),
            Some(owner) => Aim::Monster(owner),
            None => Aim::Point(cast.from),
        };
        let flights: Vec<_> = victims
            .iter()
            .map(|&victim| {
                let flight = spell.missile.map(|_| from.distance(eye_of(victim)) / MISSILE_SPEED);
                (victim, flight)
            })
            .collect();
        let act = commands
            .spawn((
                Deed::new(super::Act::Missile { caster: cast.caster, spell: spell.id }),
                Uses(uses),
                Strength { power: strength.power.min(power), against: 0, boost: strength.boost },
                OwnersStrength { owner: source, spell: spell.id },
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
