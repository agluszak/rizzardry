//! What follows once all of a spell has landed: a drain's share
//! ([`drain_back`]), Boiling Blood's bursts ([`bursts`]) and Eye for an Eye
//! ([`reflection`]). Each is settled by the release that lands it as the deed
//! completes; a burst or a reflection is a deed of its own.

use bevy::prelude::*;

use super::definitions::Definitions;
use super::effects::{Body, BodyReadOnly, Strike};
use super::reach::{Recipients, Struck, order, within};
use super::{Act, Deed, Effect};
use crate::content::spells::{SpellDefinition, SpellId, Targeting};
use crate::game::actors::Places;
use crate::game::data::Catalogs;
use crate::game::magic::Strength;
use crate::game::magic::reach::{self, Aim, flight, in_cone};
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::Stance;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;

/// The share of Lifesteal's damage that heals the most wounded party member,
/// in percent.
const LIFESTEAL_SHARE: u32 = 70;

/// The share of the spell points Might to Magic drained that a monster gains
/// as budget, in the denominator of a half.
const BUDGET_SHARE: u32 = 2;

/// What drained from a victim goes back as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Returned {
    /// Health, for Lifesteal.
    Health(u32),
    /// Spell points given back evenly, for Might to Magic to a member.
    Points(u32),
    /// Budget, for Might to Magic to a monster caster.
    Budget(u32),
}

/// What a drain gives back (retail `DrainTargetsLife`): Lifesteal 70 percent of
/// the health taken, cut down; Might to Magic the spell points taken, to a
/// member with spell points, and half of them to a monster as budget.
pub fn returned(lifesteal: bool, has_points: bool, drained: u32) -> Returned {
    if lifesteal {
        Returned::Health(drained * LIFESTEAL_SHARE / 100)
    } else if has_points {
        Returned::Points(drained)
    } else {
        Returned::Budget(drained / BUDGET_SHARE)
    }
}

/// How far from whole health is, in percent.
pub fn health_percent(current: u32, max: u32) -> u32 {
    current * 100 / max
}

/// How far from whole spell points are, in percent; none for one with no
/// spell points at all.
pub fn points_percent(available: u32, maximum: u32) -> Option<u32> {
    (maximum != 0).then(|| available * 100 / maximum)
}

/// Who a drain goes back to: the candidate whose health or points are lowest in
/// proportion, the first of equals, if any is short of whole. A candidate with
/// no percent (dead, or with no points) is passed over.
pub fn neediest(needs: &[(Entity, Option<u32>)]) -> Option<Entity> {
    needs
        .iter()
        .filter_map(|&(victim, percent)| percent.filter(|&p| p < 100).map(|p| (victim, p)))
        .min_by_key(|&(_, percent)| percent)
        .map(|(victim, _)| victim)
}

/// The strongest Eye for an Eye among the victims, in the order of the fight:
/// the first of equal strength.
pub fn strongest(eyes: impl IntoIterator<Item = (Entity, u32)>) -> Option<(Entity, u32)> {
    eyes.into_iter().fold(None, |best: Option<(Entity, u32)>, (victim, power)| match best {
        Some((_, most)) if most >= power => best,
        _ => Some((victim, power)),
    })
}

/// Whether a landed spell may be turned back: one aimed at enemies, that has
/// not been turned back and is not itself a backfire.
pub fn turns_back(aimed_at_enemies: bool, backfired: bool, reflected: bool) -> bool {
    aimed_at_enemies && !backfired && !reflected
}

/// The power a turned-back spell is cast at: the cast's, or the eye's if less.
pub fn turned_power(cast: u32, eye: u32) -> u32 {
    cast.min(eye)
}

/// Whom a burst is against: the opponents of its owner's side, the owner's own
/// side if the spell backfired, anyone if it has no owner.
pub fn burst_against(side: Option<Stance>, backfired: bool) -> Recipients {
    match side {
        Some(side) if backfired => Recipients::Opponents(side).reversed(),
        Some(side) => Recipients::Opponents(side),
        None => Recipients::Everyone,
    }
}

/// Lifesteal or Might to Magic: what the deed's damage drained goes to its
/// caster's side once all of it has landed, as health or as spell points.
#[derive(Clone, Copy, Debug)]
pub struct Leech {
    pub caster: Entity,
    pub lifesteal: bool,
}

/// The spell a deed is, for what follows once all of it has landed: whose
/// and where from, aimed how, at whom; turned on its caster's side when it
/// `backfired`, and not to be turned back once `reflected`.
#[derive(Clone, Debug)]
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

impl Spell {
    /// A spell as its caster cast it, before it is aimed.
    pub fn of(id: SpellId, caster: Entity) -> Self {
        Self {
            id,
            caster,
            backfired: false,
            reflected: false,
            owner: Some(caster),
            from: Vec3::ZERO,
            aim: Aim::Ahead,
            victims: Vec::new(),
        }
    }
}

/// Who stands where, for a spell that bursts or turns back.
pub struct Surroundings<'a, 'w, 's> {
    pub roster: &'a PartyRoster,
    pub combatants: &'a [Entity],
    pub memberships: &'a Query<'w, 's, &'static GroupMember>,
    pub groups: &'a Query<'w, 's, &'static GroupMembers>,
    pub obstacles: &'a WorldObstacles<'w, 's>,
    pub places: &'a Places<'w, 's>,
    pub content: &'a Catalogs,
    pub definitions: &'a Definitions,
}

/// The bodies a burst may take, as [`within`] finds them from where it went
/// off: those in the party and monsters, not a character without a seat, with
/// whether a clear line runs from `origin` to each, in the fight's order.
pub fn struck(
    bodies: &Query<BodyReadOnly>,
    places: &Places,
    obstacles: &WorldObstacles,
    origin: Vec3,
    (roster, combatants): (&PartyRoster, &[Entity]),
) -> Vec<Struck> {
    bodies
        .iter()
        .filter(|body| !body.untargetable && (body.member || body.character.is_none()))
        .map(|body| {
            let at = places.of(body.entity);
            let order = order(
                roster.get_index_of(&body.entity),
                combatants.iter().position(|&c| c == body.entity),
            );
            Struck {
                id: body.entity,
                dead: body.vitals.is_dead(),
                stance: body.stance(),
                centre: at.centre,
                radius: at.radius,
                order,
                clear: obstacles.clear(origin, at.centre),
            }
        })
        .collect()
}

/// The strength of a spell that comes from someone other than its caster (a
/// backfire, a reflection or a burst): its power and boost, resisted
/// against by its owner's caster level in it; an owner gone, or none, casts
/// from nothing.
pub fn owners_strength(
    owner: Option<Entity>,
    spell: &SpellDefinition,
    (cast_power, boost): (u32, u32),
    bodies: &Query<BodyReadOnly>,
) -> Strength {
    let figure = owner
        .and_then(|owner| bodies.get(owner).ok())
        .map_or(0, |body| body.resistances.caster_level(spell));
    crate::game::magic::power::strength(figure, cast_power, boost, spell)
}

/// What a Lifesteal or Might to Magic deed drained goes to the caster's
/// side: a party caster's to the member most in need of it (retail chooses
/// after every victim has been drained), a monster's to itself.
pub fn drain_back(
    leech: Leech,
    drained: u32,
    roster: &PartyRoster,
    bodies: &mut Query<Body>,
    content: &Catalogs,
) -> Option<Effect> {
    let member = bodies.get(leech.caster).ok()?.member;
    let candidates: Vec<Entity> = if member { roster.iter().collect() } else { vec![leech.caster] };
    let need = |victim: Entity| -> Option<u32> {
        let body = bodies.get(victim).ok()?;
        if body.vitals.is_dead() {
            return None;
        }
        if leech.lifesteal {
            Some(health_percent(body.vitals.health.current, body.vitals.health.max))
        } else if let (Some(character), Some((learned, pool))) = (body.character, body.spells) {
            let maxima = character.max_points(body.attributes, learned, &content.spells);
            let available = maxima.iter().map(|(realm, _)| pool.available(realm, &maxima)).sum();
            points_percent(available, maxima.values().sum())
        } else {
            Some(0)
        }
    };
    let needs: Vec<_> = candidates.into_iter().map(|victim| (victim, need(victim))).collect();
    let victim = neediest(&needs)?;
    let mut body = bodies.get_mut(victim).ok()?;
    match returned(leech.lifesteal, body.spells.is_some(), drained) {
        Returned::Health(health) => {
            let amount = body.vitals.heal(health);
            Some(Effect::Healed { victim, amount })
        }
        Returned::Points(points) => {
            let (_, pool) = body.spells.as_mut()?;
            let amount = pool.restore_evenly(points);
            Some(Effect::Replenished { victim, amount })
        }
        Returned::Budget(budget) => {
            if let Some(gained) = body.budget.as_mut() {
                gained.0 += budget;
            }
            None
        }
    }
}

/// Each of a Boiling Blood cast's victims that died bursts where it fell,
/// at the cast's power, from the spell's owner (retail's Boiling Blood
/// explosion): a deed of its own, reaching whoever stands within its
/// radius, as strong as its owner.
pub fn bursts(
    spell: &Spell,
    strength: Strength,
    bodies: &Query<BodyReadOnly>,
    around: &Surroundings,
) -> Vec<(Deed, Vec<(Entity, Option<f32>)>)> {
    let places = around.places;
    let Some(into) = around
        .definitions
        .spell(spell.id)
        .and_then(|definition| around.definitions.get(definition).bursts_into)
    else {
        return vec![];
    };
    let Some(explosion) = around.content.spells.get(into) else {
        return vec![];
    };
    let Some(definition) = around.definitions.spell(explosion.id) else { return vec![] };
    let side = spell.owner.and_then(|owner| bodies.get(owner).ok()).map(|body| body.stance());
    let recipients = burst_against(side, spell.backfired);
    let radius = crate::game::magic::reach::blast_radius(explosion, strength.power);
    let strength =
        owners_strength(spell.owner, explosion, (strength.power, strength.boost), bodies);
    spell
        .victims
        .iter()
        .filter_map(|&victim| bodies.get(victim).ok())
        .filter(|body| !body.untargetable && body.vitals.is_dead())
        .map(|body| {
            let at = places.of(body.entity).feet;
            let found =
                struck(bodies, places, around.obstacles, at, (around.roster, around.combatants));
            let victims = within(at, radius, recipients, found);
            let deed = Deed::new(
                Act::Missile { caster: spell.caster, spell: explosion.id },
                Strike { definition, strength, blow: None },
            );
            (deed, victims.into_iter().map(|victim| (victim, None)).collect())
        })
        .collect()
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

/// The first of a monster's group to have joined the fight, whom a group
/// spell turned back comes from.
pub fn leader(
    entity: Entity,
    combatants: &[Entity],
    memberships: &Query<&GroupMember>,
    groups: &Query<&GroupMembers>,
) -> Entity {
    memberships
        .get(entity)
        .ok()
        .and_then(|member| groups.get(member.0).ok())
        .map_or_else(|| vec![entity], |members| members.iter().collect())
        .into_iter()
        .min_by_key(|&member| combatants.iter().position(|&c| c == member).unwrap_or(usize::MAX))
        .unwrap_or(entity)
}

/// Once a spell aimed at enemies has landed, the strongest Eye for an Eye
/// among its surviving victims (members first, then monsters in placement
/// order) pays a charge to turn it back on its source (retail
/// `FinishSpellEffectTargets`): an aimed spell on its owner, a group spell
/// on the owner's group, a blast, cone or all-enemies spell on whoever of
/// that side stands where it went off. The reflection is a deed of its
/// own, as strong as its new source, flying back if the spell is a
/// missile, and is not turned back again. Who paid the charge, and the
/// deed with those it reaches.
pub fn reflection(
    cast: &Spell,
    strength: Strength,
    bodies: &Query<BodyReadOnly>,
    around: &Surroundings,
    laying: &mut crate::game::actors::Laying,
) -> Option<(Entity, Deed, Vec<(Entity, Option<f32>)>)> {
    let Surroundings { roster, combatants, memberships, groups, obstacles, places, .. } = *around;
    let spell = &around.content.spells[cast.id];
    let definition = around.definitions.spell(cast.id)?;
    if !turns_back(spell.aimed_at_enemies(), cast.backfired, cast.reflected) {
        return None;
    }
    let mut order = cast.victims.clone();
    order.sort_by_key(|&victim| match roster.get_index_of(&victim) {
        Some(seat) => (0, seat),
        None => (1, combatants.iter().position(|&c| c == victim).unwrap_or(usize::MAX)),
    });
    let eye_spell = around.content.spells.0.iter().map(|laid| laid.id).find(|&laid| {
        around.definitions.spell(laid).is_some_and(|d| around.definitions.get(d).turns_back)
    })?;
    let (charged, eye_power) = strongest(order.into_iter().filter_map(|victim| {
        let body = bodies.get(victim).ok()?;
        let charge = laying.get(victim, eye_spell)?;
        (!body.untargetable && !body.vitals.is_dead()).then_some((victim, charge.power))
    }))?;
    laying.spend(charged, eye_spell);
    let party_at = roster.first().map(|&member| places.of(member));
    let owner = cast.owner;
    let eye = party_at.map_or(cast.from, |at| at.eye);
    let eye_of = |actor: Entity| bodies.get(actor).map_or(eye, |body| places.of(body.entity).eye);
    // Whom and where it comes back from.
    let (source, from) = match (spell.targeting, cast.aim) {
        (Targeting::Radius | Targeting::Cone, Aim::Point(at)) => (owner, at),
        (Targeting::Radius | Targeting::Cone | Targeting::AllEnemies, _) => (owner, cast.from),
        (Targeting::EnemyGroup, Aim::Body(entity)) => {
            let leader = leader(entity, combatants, memberships, groups);
            (Some(leader), eye_of(leader))
        }
        (_, Aim::Body(entity)) => (Some(entity), eye_of(entity)),
        _ => {
            let first = roster
                .iter()
                .find(|&member| bodies.get(member).is_ok_and(|body| body.vitals.can_fight()))
                .unwrap_or(cast.caster);
            (Some(first), eye)
        }
    };
    let recipients = match owner.and_then(|owner| bodies.get(owner).ok()) {
        Some(body) => Recipients::Opponents(body.stance()).reversed(),
        None => Recipients::Everyone,
    };
    let power = turned_power(strength.power, eye_power);
    let reached = |centre, origin, radius| {
        let found = struck(bodies, places, obstacles, origin, (roster, combatants));
        within(centre, radius, recipients, found)
    };
    let victims = match spell.targeting {
        Targeting::Enemy => owner.into_iter().collect(),
        Targeting::EnemyGroup => owner.map_or(vec![], |owner| {
            group_of(owner, roster, memberships, groups)
                .into_iter()
                .filter(|&body| bodies.get(body).is_ok_and(|body| !body.vitals.is_dead()))
                .collect()
        }),
        Targeting::Radius => {
            let centre = owner
                .and_then(|owner| bodies.get(owner).ok())
                .map_or(party_at.map_or(cast.from, |at| at.feet), |body| {
                    places.of(body.entity).feet
                });
            reached(centre, cast.from, reach::blast_radius(spell, power))
        }
        Targeting::Cone => reached(from, from, spell.reach.metres())
            .into_iter()
            .filter(|&victim| in_cone(cast.from - from, eye_of(victim) - from))
            .collect(),
        Targeting::AllEnemies => reached(cast.from, cast.from, spell.reach.metres()),
        _ => vec![],
    };
    let flights = victims
        .iter()
        .map(|&victim| (victim, spell.missile.map(|_| flight(from, eye_of(victim)))))
        .collect();
    let strength = owners_strength(source, spell, (power, strength.boost), bodies);
    let mut deed = Deed::new(
        Act::Missile { caster: cast.caster, spell: spell.id },
        Strike { definition, strength, blow: None },
    );
    deed.spell = Some(Spell {
        id: spell.id,
        caster: cast.caster,
        backfired: false,
        reflected: true,
        owner: source,
        from,
        aim: owner.map_or(Aim::Point(cast.from), Aim::Body),
        victims,
    });
    Some((charged, deed, flights))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    /// Lifesteal gives back 70 percent cut down: 10 -> 7, 9 -> 6 (6.3); Might to
    /// Magic gives a member the points drained, and a monster half as budget:
    /// 11 -> 5.
    #[test]
    fn a_drain_goes_back_as_health_points_or_budget() {
        assert_eq!(returned(true, false, 10), Returned::Health(7));
        assert_eq!(returned(true, true, 9), Returned::Health(6));
        assert_eq!(returned(true, false, 0), Returned::Health(0));
        assert_eq!(returned(false, true, 10), Returned::Points(10));
        assert_eq!(returned(false, false, 11), Returned::Budget(5));
    }

    /// The member lowest in proportion, the first of equals; the whole and the
    /// dead (no percent) are passed over; none short, none.
    #[test]
    fn a_drain_goes_to_the_member_lowest_in_proportion() {
        let (a, b, c, d) = (id(1), id(2), id(3), id(4));
        assert_eq!(neediest(&[(a, Some(100)), (b, Some(60)), (c, Some(60)), (d, None)]), Some(b));
        assert_eq!(neediest(&[(a, Some(100)), (b, Some(100))]), None);
        assert_eq!(neediest(&[(a, None), (b, None)]), None);
        assert_eq!(neediest(&[(a, Some(99)), (b, Some(0))]), Some(b));
        assert_eq!(neediest(&[]), None);
    }
}
