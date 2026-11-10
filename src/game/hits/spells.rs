//! What follows once all of a spell has landed: a drain's share
//! ([`drain_back`]), Boiling Blood's bursts ([`bursts`]) and Eye for an Eye
//! ([`reflection`]). Each is settled by [`super::resolve`] as the deed
//! completes; a burst or a reflection is a deed of its own.

use bevy::prelude::*;

use super::definitions::Definitions;
use super::effects::{Body, BodyReadOnly, Strike};
use super::reach::{Recipients, within};
use super::{Act, Deed, Effect};
use crate::content::spells::{SpellDefinition, SpellId, Targeting};
use crate::game::characters::Vitals;
use crate::game::data::Catalogs;
use crate::game::magic::Strength;
use crate::game::magic::casting::{Aim, MISSILE_SPEED, in_cone};
use crate::game::party::roster::PartyRoster;
use crate::game::world::footing::Footing;
use crate::game::world::hostility::Stance;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;

/// The share of Lifesteal's damage that heals the most wounded party member,
/// in percent.
const LIFESTEAL_SHARE: u32 = 70;

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
    pub content: &'a Catalogs,
    pub definitions: &'a Definitions,
}

/// The bodies spells may take, as [`within`] reads them.
pub fn targets<'a>(
    bodies: &'a Query<BodyReadOnly>,
) -> impl Iterator<Item = (Entity, &'a Vitals, Stance, &'a Footing)> {
    bodies
        .iter()
        .filter(|body| !body.untargetable)
        .map(|body| (body.entity, body.vitals, body.side.0, body.at))
}

/// The strength of a spell that comes from someone other than its caster (a
/// backfire, a reflection or a burst): its power and boost, resisted
/// against by its owner's caster level in it; an owner gone, or none, casts
/// from nothing.
pub fn owners_strength(
    owner: Option<Entity>,
    spell: &SpellDefinition,
    (power, boost): (u32, u32),
    bodies: &Query<BodyReadOnly>,
) -> Strength {
    let figure = owner
        .and_then(|owner| bodies.get(owner).ok())
        .map_or(0, |body| body.resistances.caster_level(spell));
    Strength::at(figure, spell, power, boost)
}

/// What a Lifesteal or Might to Magic deed drained goes to the caster's
/// side: a party caster's to the member most in need of it (retail chooses
/// after every victim has been drained), a monster's to itself, 70% of the
/// health back or half of it as spell points.
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
        let percent = if leech.lifesteal {
            body.vitals.health.current * 100 / body.vitals.health.max
        } else if let (Some(character), Some((learned, pool))) = (body.character, body.spells) {
            let maxima = character.max_points(body.attributes, learned, &content.spells);
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
    let victim = candidates
        .into_iter()
        .filter_map(|victim| need(victim).map(|percent| (victim, percent)))
        .min_by_key(|&(_, percent)| percent)
        .map(|(victim, _)| victim)?;
    let mut body = bodies.get_mut(victim).ok()?;
    if leech.lifesteal {
        let amount = body.vitals.heal(drained * LIFESTEAL_SHARE / 100);
        Some(Effect::Healed { victim, amount })
    } else if let Some((_, pool)) = body.spells.as_mut() {
        let amount = pool.restore_evenly(drained);
        Some(Effect::Replenished { victim, amount })
    } else {
        if let Some(budget) = body.budget.as_mut() {
            budget.0 += drained / 2;
        }
        None
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
    if spell.id != SpellId::BOILING_BLOOD {
        return vec![];
    }
    let Some(explosion) = around.content.spells.get(SpellId::BOILING_BLOOD_EXPLOSION) else {
        return vec![];
    };
    let Some(definition) = around.definitions.spell(explosion.id) else { return vec![] };
    let side = spell.owner.and_then(|owner| bodies.get(owner).ok()).map(|body| body.side.0);
    let recipients = match side {
        Some(side) if spell.backfired => Recipients::Opponents(side).reversed(),
        Some(side) => Recipients::Opponents(side),
        None => Recipients::Everyone,
    };
    let radius = explosion.radius + explosion.radius_per_power * strength.power as f32;
    let strength =
        owners_strength(spell.owner, explosion, (strength.power, strength.boost), bodies);
    spell
        .victims
        .iter()
        .filter_map(|&victim| bodies.get(victim).ok())
        .filter(|body| !body.untargetable && body.vitals.is_dead())
        .map(|body| {
            let victims = within(
                body.at.feet,
                body.at.feet,
                radius,
                recipients,
                targets(bodies),
                (around.roster, around.combatants),
                around.obstacles,
            );
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
    bodies: &mut Query<Body>,
    around: &Surroundings,
) -> Option<(Entity, Deed, Vec<(Entity, Option<f32>)>)> {
    let Surroundings { roster, combatants, memberships, groups, obstacles, .. } = *around;
    let spell = &around.content.spells[cast.id];
    let definition = around.definitions.spell(cast.id)?;
    if cast.backfired || cast.reflected || !spell.aimed_at_enemies() {
        return None;
    }
    let mut order = cast.victims.clone();
    order.sort_by_key(|&victim| match roster.get_index_of(&victim) {
        Some(seat) => (0, seat),
        None => (1, combatants.iter().position(|&c| c == victim).unwrap_or(usize::MAX)),
    });
    let (charged, power) = order
        .into_iter()
        .filter_map(|victim| {
            let body = bodies.get(victim).ok()?;
            let charge = body.vitals.enchantments.get(SpellId::EYE_FOR_AN_EYE)?;
            (!body.untargetable && !body.vitals.is_dead()).then_some((victim, charge.power))
        })
        .fold(None, |best: Option<(Entity, u32)>, (victim, power)| match best {
            Some((_, most)) if most >= power => best,
            _ => Some((victim, power)),
        })?;
    bodies.get_mut(charged).unwrap().vitals.enchantments.spend_charge(SpellId::EYE_FOR_AN_EYE);
    let bodies = bodies.as_readonly();
    let party_at = roster.iter().find_map(|member| bodies.get(member).ok().map(|body| *body.at));
    let owner = cast.owner;
    let eye = party_at.map_or(cast.from, |at| at.eye);
    let eye_of = |actor: Entity| bodies.get(actor).map_or(eye, |body| body.at.eye);
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
        Some(body) => Recipients::Opponents(body.side.0).reversed(),
        None => Recipients::Everyone,
    };
    let power = strength.power.min(power);
    let reached = |centre, origin, radius| {
        within(
            centre,
            origin,
            radius,
            recipients,
            targets(&bodies),
            (roster, combatants),
            obstacles,
        )
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
                .map_or(party_at.map_or(cast.from, |at| at.feet), |body| body.at.feet);
            reached(centre, cast.from, spell.radius + spell.radius_per_power * power as f32)
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
        .map(|&victim| {
            (victim, spell.missile.map(|_| from.distance(eye_of(victim)) / MISSILE_SPEED))
        })
        .collect();
    let strength = owners_strength(source, spell, (power, strength.boost), &bodies);
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
