//! What follows once all of a deed of magic has landed ([`settle`]): a
//! drain's share ([`Leech`]), Boiling Blood's bursts and Eye for an Eye
//! ([`Spell`]), its caster's practice and an auto recast, and the telling of
//! it. Each is something its maker put beside the deed; a burst or a
//! reflection is a deed of its own.

use bevy::prelude::*;

use super::casting::{MagicNews, Practice, Recast};
use super::reach::{self, Aim, flight, in_cone};
use super::{MagicRequest, Strength};
use crate::content::spells::{SpellDefinition, SpellId, Targeting};
use crate::game::actors::{Laying, Places};
use crate::game::characters::Practise;
use crate::game::combat::{Battle, Combatants};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::{Leeches, Rows};
use crate::game::hits::effects::{Body, BodyReadOnly, Strike};
use crate::game::hits::reach::{Field, Recipients};
use crate::game::hits::{Deed, Effect, Landed};
use crate::game::party::roster::PartyRoster;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;

/// A deed of magic: what it did is told as magic's news once it has landed.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Magical;

/// The share of Lifesteal's damage that heals, in percent.
const LIFESTEAL_SHARE: u32 = 70;

/// A leeching deed: what its damage drained goes to its caster's side once
/// all of it has landed.
#[derive(Component, Clone, Copy, Debug)]
#[require(Magical)]
pub struct Leech {
    pub caster: Entity,
    pub gives: Leeches,
}

/// The spell a deed is, for what follows once all of it has landed: whose
/// and where from, aimed how, at whom; turned on its caster's side when it
/// `backfired`, and not to be turned back once `reflected`.
#[derive(Component, Clone, Debug)]
#[require(Magical)]
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

/// What a leeching deed drained goes to the caster's side (retail
/// `DrainTargetsLife`, chosen after every victim has been drained): a party
/// caster's to the living member lowest in proportion in what it gives, the
/// first of equals, if any is short of whole; a monster's to itself.
/// Lifesteal heals 70 percent of the health taken; Might to Magic gives a
/// member the spell points taken, evenly, and a monster half of them as
/// budget.
fn drain_back(
    leech: Leech,
    drained: u32,
    roster: &PartyRoster,
    bodies: &mut Query<Body>,
    content: &Catalogs,
) -> Option<Effect> {
    let member = bodies.get(leech.caster).ok()?.member;
    let candidates: Vec<Entity> = if member { roster.iter().collect() } else { vec![leech.caster] };
    // How far from whole each is, in percent; a monster with no points is
    // always in need of budget.
    let need = |victim: Entity| -> Option<u32> {
        let body = bodies.get(victim).ok().filter(|body| !body.vitals.is_dead())?;
        match (leech.gives, body.character.zip(body.spells)) {
            (Leeches::Health, _) => Some(body.vitals.health.current * 100 / body.vitals.health.max),
            (Leeches::Points, Some((character, (learned, pool)))) => {
                let maxima = character.max_points(body.attributes, learned, &content.spells);
                let most: u32 = maxima.values().sum();
                let have: u32 =
                    maxima.iter().map(|(realm, _)| pool.available(realm, &maxima)).sum();
                (most != 0).then(|| have * 100 / most)
            }
            (Leeches::Points, None) => Some(0),
        }
    };
    let victim = candidates
        .into_iter()
        .filter_map(|victim| need(victim).filter(|&short| short < 100).map(|short| (victim, short)))
        .min_by_key(|&(_, short)| short)
        .map(|(victim, _)| victim)?;
    let mut body = bodies.get_mut(victim).ok()?;
    match leech.gives {
        Leeches::Health => {
            let amount = body.vitals.heal(drained * LIFESTEAL_SHARE / 100);
            Some(Effect::Healed { victim, amount })
        }
        Leeches::Points => match body.spells.as_mut() {
            Some((_, pool)) => {
                let amount = pool.restore_evenly(drained);
                Some(Effect::Replenished { victim, amount })
            }
            None => {
                if let Some(budget) = body.budget.as_mut() {
                    budget.0 += drained / 2;
                }
                None
            }
        },
    }
}

/// What follows a deed of magic once all of it has landed, in this order:
/// its drain goes back to its caster's side; each victim it killed bursts;
/// an Eye for an Eye turns it back; its caster practises if it affected
/// anyone; an auto cast is asked for again; and what it did is told.
pub fn settle(
    landed: On<Landed>,
    deeds: Query<
        (&Deed, Option<&Leech>, Option<&Spell>, Option<&Practice>, Option<&Recast>),
        With<Magical>,
    >,
    mut bodies: Query<Body>,
    roster: Single<&PartyRoster>,
    combatants: Option<Single<&Combatants, With<Battle>>>,
    grouping: (Query<&'static GroupMember>, Query<&'static GroupMembers>),
    (places, obstacles): (Places, WorldObstacles),
    mut laying: Laying,
    content: Res<Catalogs>,
    mut requests: MessageWriter<MagicRequest>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let Ok((deed, leech, spell, practice, recast)) = deeds.get(landed.deed) else { return };
    let mut effects = landed.effects.clone();
    if let Some(&leech) = leech {
        effects.extend(drain_back(leech, landed.harm, *roster, &mut bodies, &content));
    }
    if let Some(spell) = spell {
        let seen = bodies.as_readonly();
        let field = Field {
            bodies: &seen,
            places: &places,
            obstacles: &obstacles,
            roster: *roster,
            memberships: &grouping.0,
            groups: &grouping.1,
            combatants: combatants.as_ref().map_or(&[][..], |fight| fight.entities()),
        };
        let strength = deed.strike.strength;
        for burst in bursts(spell, strength, &field, &content) {
            commands.spawn((burst, Magical));
        }
        if let Some((charged, turned)) = reflection(spell, strength, &field, &content, &mut laying)
        {
            effects.push(Effect::Reflected { victim: charged });
            commands.spawn(turned);
        }
    }
    // Practice needs the deed to have affected someone (retail
    // `SpellAffectedTarget`), or a missile to be on its way.
    let affected = effects.iter().any(|effect| !matches!(effect, Effect::Nothing { .. }));
    if let Some(practice) = practice
        && (practice.sure || affected)
    {
        for &(skill, usage) in &practice.skills {
            commands.trigger(Practise { member: practice.caster, skill, usage });
        }
    }
    if let Some(recast) = recast
        && recast.wanted(&bodies.as_readonly(), &content)
    {
        requests.write(MagicRequest { caster: recast.caster, choice: recast.choice });
    }
    if !effects.is_empty() {
        news.write(MagicNews::Landed { effects });
    }
}

/// Each of a cast's victims that died bursts where it fell, if the spell's
/// row says it does (Boiling Blood), at the cast's power, from the spell's
/// owner: a deed of its own, reaching whoever of the other side stands
/// within its radius (the owner's own side if the spell backfired, anyone
/// if it has no owner), as strong as its owner.
fn bursts(spell: &Spell, strength: Strength, field: &Field, content: &Catalogs) -> Vec<Deed> {
    let into = content.spell_row(spell.id).and_then(|row| row.bursts_into);
    let Some(explosion) = into.and_then(|into| content.spells.get(into)) else { return vec![] };
    if content.spell_row(explosion.id).is_none() {
        return vec![];
    }
    let rows = Rows::Spell(explosion.id);
    let bodies = field.bodies;
    let side = spell.owner.and_then(|owner| bodies.get(owner).ok()).map(|body| body.stance());
    let recipients = match side {
        Some(side) if spell.backfired => Recipients::Opponents(side).reversed(),
        Some(side) => Recipients::Opponents(side),
        None => Recipients::Everyone,
    };
    let radius = reach::blast_radius(explosion, strength.power);
    let strength =
        owners_strength(spell.owner, explosion, (strength.power, strength.boost), bodies);
    spell
        .victims
        .iter()
        .filter_map(|&victim| bodies.get(victim).ok())
        .filter(|body| !body.untargetable && body.vitals.is_dead())
        .map(|body| {
            let at = field.places.of(body.entity).feet;
            let victims = field.within(at, at, radius, recipients);
            Deed::at(Strike { rows, strength, blow: None }, victims)
        })
        .collect()
}

/// Once a spell aimed at enemies has landed (not a backfire, and not one
/// turned back already), the strongest Eye for an Eye among its surviving
/// victims, members first and then monsters as they joined, the first of
/// equals, pays a charge to turn it back on its source (retail
/// `FinishSpellEffectTargets`): an aimed spell on its owner, a group spell
/// on the owner's group, a blast, cone or all-enemies spell on whoever of
/// that side stands where it went off. The reflection is a deed of its own
/// at the lesser of the cast's power and the eye's, as strong as its new
/// source, flying back if the spell is a missile. Who paid the charge, and
/// the deed.
fn reflection(
    cast: &Spell,
    strength: Strength,
    field: &Field,
    content: &Catalogs,
    laying: &mut Laying,
) -> Option<(Entity, (Deed, Spell))> {
    let Field { bodies, places, roster, .. } = *field;
    let spell = &content.spells[cast.id];
    content.spell_row(cast.id)?;
    if !spell.aimed_at_enemies() || cast.backfired || cast.reflected {
        return None;
    }
    let mut order = cast.victims.clone();
    order.sort_by_key(|&victim| match roster.get_index_of(&victim) {
        Some(seat) => (0, seat),
        None => (1, field.joined(victim)),
    });
    let (charged, eye_power) = order
        .into_iter()
        .filter(|&victim| {
            bodies.get(victim).is_ok_and(|body| !body.untargetable && !body.vitals.is_dead())
        })
        .filter_map(|victim| Some((victim, laying.turns_back(victim)?)))
        .reduce(|best, eye| if eye.1 > best.1 { eye } else { best })?;
    laying.turn_back(charged);
    let party_at = roster.first().map(|&member| places.of(member));
    let owner = cast.owner;
    let eye = party_at.map_or(cast.from, |at| at.eye);
    let eye_of = |actor: Entity| bodies.get(actor).map_or(eye, |body| places.of(body.entity).eye);
    // Whom and where it comes back from.
    let (source, from) = match (spell.targeting, cast.aim) {
        (Targeting::Radius | Targeting::Cone, Aim::Point(at)) => (owner, at),
        (Targeting::Radius | Targeting::Cone | Targeting::AllEnemies, _) => (owner, cast.from),
        // The first of the group aimed at to have joined the fight.
        (Targeting::EnemyGroup, Aim::Body(entity)) => {
            let leader = field.leader_of(entity);
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
    let power = strength.power.min(eye_power);
    let victims = match spell.targeting {
        Targeting::Enemy => owner.into_iter().collect(),
        Targeting::EnemyGroup => owner.map_or(vec![], |owner| {
            field
                .group_of(owner)
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
            field.within(centre, cast.from, reach::blast_radius(spell, power), recipients)
        }
        Targeting::Cone => field
            .within(from, from, spell.reach.metres(), recipients)
            .into_iter()
            .filter(|&victim| in_cone(cast.from - from, eye_of(victim) - from))
            .collect(),
        Targeting::AllEnemies => {
            field.within(cast.from, cast.from, spell.reach.metres(), recipients)
        }
        _ => vec![],
    };
    let flights: Vec<(Entity, Option<f32>)> = victims
        .iter()
        .map(|&victim| (victim, spell.missile.map(|_| flight(from, eye_of(victim)))))
        .collect();
    let strength = owners_strength(source, spell, (power, strength.boost), bodies);
    let deed = Deed::flying(Strike { rows: Rows::Spell(spell.id), strength, blow: None }, flights);
    let turned = Spell {
        id: spell.id,
        caster: cast.caster,
        backfired: false,
        reflected: true,
        owner: source,
        from,
        aim: owner.map_or(Aim::Point(cast.from), Aim::Body),
        victims,
    };
    Some((charged, (deed, turned)))
}
