//! What follows once all of a deed of magic has landed, each its own
//! observer, set by whoever put what it follows beside the deed: the telling
//! of it ([`Magical`]), a drain's share ([`Leech`]), Boiling Blood's bursts
//! and Eye for an Eye ([`Spell`]), its caster's practice ([`practise`]) and
//! an auto recast ([`recast`]). A burst or a reflection is an aim, and then a
//! deed of its own.

use bevy::prelude::*;

use super::aim::{Against, Area, Reaching};
use super::casting::{MagicNews, Practice, Recast};
use super::reach::{self, Aim, flight};
use super::{MagicRequest, SpellPoints};
use crate::content::spells::{Returns, SpellDefinition, SpellId};
use crate::game::actors::{CanFight, CasterLevels, Laying};
use crate::game::characters::{Practise, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::{Leeches, Rows};
use crate::game::hits::effects::Strike;
use crate::game::hits::{Deed, Effect, Landed};
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::Standing;
use crate::game::world::population::{SpellBudget, Untargetable};

/// A deed of magic: what it did is told once all of it has landed
/// ([`tell`]), its own observer from the moment it is made, before it
/// lands.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = watched_as_magic)]
pub struct Magical;

fn watched_as_magic(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    world.commands().entity(context.entity).observe(tell);
}

fn follows_spell(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    world.commands().entity(context.entity).observe(follow_spell);
}

fn drains_back(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    world.commands().entity(context.entity).observe(drain_back);
}

/// The share of Lifesteal's damage that heals, in percent.
const LIFESTEAL_SHARE: u32 = 70;

/// A leeching deed: what its damage drained goes to its caster's side once
/// all of it has landed ([`drain_back`], its own observer).
#[derive(Component, Clone, Copy, Debug)]
#[require(Magical)]
#[component(on_add = drains_back)]
pub struct Leech {
    pub caster: Entity,
    pub gives: Leeches,
}

/// The spell a deed is, for what follows once all of it has landed: whose
/// and where from, aimed how, at whom; turned on its caster's side when it
/// `backfired`, and not to be turned back once `reflected`.
#[derive(Component, Clone, Debug)]
#[require(Magical)]
#[component(on_add = follows_spell)]
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

/// What a leeching deed drained goes to the caster's side (retail
/// `DrainTargetsLife`, chosen after every victim has been drained): a party
/// caster's to the living member lowest in proportion in what it gives, the
/// first of equals, if any is short of whole; a monster's to itself.
/// Lifesteal heals 70 percent of the health taken; Might to Magic gives a
/// member the spell points taken, evenly, and a monster half of them as
/// budget.
pub fn drain_back(
    landed: On<Landed>,
    leeches: Query<&Leech>,
    roster: Single<&PartyRoster>,
    members: Query<(), With<crate::game::party::roster::PartyMember>>,
    mut bodies: Query<(
        &mut crate::game::characters::Vitals,
        Option<&mut SpellPoints>,
        Option<&mut SpellBudget>,
    )>,
    mut news: MessageWriter<MagicNews>,
) -> Result {
    let &leech = leeches.get(landed.deed)?;
    let drained = landed.harm;
    let candidates: Vec<Entity> =
        if members.contains(leech.caster) { roster.iter().collect() } else { vec![leech.caster] };
    // How far from whole each is, in percent; a monster with no points is
    // always in need of budget.
    let need = |victim: Entity| -> Option<u32> {
        let (vitals, spells, _) =
            bodies.get(victim).ok().filter(|(vitals, ..)| !vitals.is_dead())?;
        match (leech.gives, spells) {
            (Leeches::Health, _) => Some(vitals.health.current * 100 / vitals.health.max),
            (Leeches::Points, Some(pool)) => {
                let (have, most) = pool.total();
                (most != 0).then(|| have * 100 / most)
            }
            (Leeches::Points, None) => Some(0),
        }
    };
    let Some(victim) = candidates
        .into_iter()
        .filter_map(|victim| need(victim).filter(|&short| short < 100).map(|short| (victim, short)))
        .min_by_key(|&(_, short)| short)
        .map(|(victim, _)| victim)
    else {
        return Ok(());
    };
    let (mut vitals, spells, budget) = bodies.get_mut(victim)?;
    let effect = match (leech.gives, spells, budget) {
        (Leeches::Health, ..) => {
            let amount = vitals.heal(drained * LIFESTEAL_SHARE / 100);
            Effect::Healed { victim, amount }
        }
        (Leeches::Points, Some(mut pool), _) => {
            let amount = pool.restore_evenly(drained);
            Effect::Replenished { victim, amount }
        }
        (Leeches::Points, None, Some(mut budget)) => {
            budget.0 += drained / 2;
            return Ok(());
        }
        (Leeches::Points, None, None) => return Ok(()),
    };
    news.write(MagicNews::Landed { effects: vec![effect] });
    Ok(())
}

/// What a deed of magic did is told as it lands.
fn tell(landed: On<Landed>, mut news: MessageWriter<MagicNews>) {
    if !landed.effects.is_empty() {
        news.write(MagicNews::Landed { effects: landed.effects.clone() });
    }
}

/// A cast's caster practises once its deed has landed, if it affected
/// anyone (retail `SpellAffectedTarget`) or a missile was on its way: set on
/// the deed beside its [`Practice`].
pub fn practise(landed: On<Landed>, deeds: Query<&Practice>, mut commands: Commands) {
    let Ok(practice) = deeds.get(landed.deed) else { return };
    if practice.sure || landed.affected() {
        for &(skill, usage) in &practice.skills {
            commands.trigger(Practise { member: practice.caster, skill, usage });
        }
    }
}

/// An auto recast goes on once its deed has landed, while a living victim
/// is not yet whole in what the spell restores, and its caster's stamina and
/// points last: set on the deed beside its [`Recast`].
pub fn recast(
    landed: On<Landed>,
    deeds: Query<(&Recast, &Spell)>,
    vitals: Query<&Vitals>,
    casters: Query<(&Vitals, &SpellPoints)>,
    learned: Query<&super::castable::CastableSpell>,
    mut requests: MessageWriter<MagicRequest>,
) {
    let Ok((recast, deed)) = deeds.get(landed.deed) else { return };
    let Ok(super::castable::CastableSpell(spell)) = learned.get(recast.choice.castable) else {
        return;
    };
    let short = |victim: &Entity| {
        vitals.get(*victim).is_ok_and(|vitals| {
            let pool = match recast.watches_stamina {
                true => vitals.stamina,
                false => vitals.health,
            };
            !vitals.is_dead() && pool.current < pool.max
        })
    };
    let goes_again = casters.get(recast.caster).is_ok_and(|(vitals, pool)| {
        super::casting::goes_again((vitals.stamina.current, pool), spell)
    });
    if deed.victims.iter().any(short) && goes_again {
        requests.write(MagicRequest { caster: recast.caster, choice: recast.choice });
    }
}

/// Once a spell has landed, its fallen burst ([`bursts`]), then the
/// strongest Eye for an Eye among its living victims may turn it back
/// ([`turned_back`]): one moment, in that order.
#[allow(clippy::type_complexity)]
fn follow_spell(
    landed: On<Landed>,
    deeds: Query<(&Spell, &Deed)>,
    vitals: Query<(&Vitals, Has<Untargetable>)>,
    (standings, levels, fighters): (
        Query<&Standing>,
        Query<&CasterLevels>,
        Query<(), With<CanFight>>,
    ),
    reaching: Reaching,
    mut laying: Laying,
    content: Res<Catalogs>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) -> Result {
    let (cast, made) = deeds.get(landed.deed)?;
    let spell = &content.spells[cast.id];
    let side = |body: Option<Entity>| body.and_then(|b| standings.get(b).ok()).map(|s| s.stance);
    let figure = |body: Option<Entity>, spell: &SpellDefinition| {
        body.and_then(|b| levels.get(b).ok()).map_or(0, |l| l.caster_level(spell))
    };
    let alive = |body: Entity| vitals.get(body).is_ok_and(|(v, never)| !never && !v.is_dead());
    let fallen = |body: Entity| vitals.get(body).is_ok_and(|(v, never)| !never && v.is_dead());
    let strength = made.strike.strength;
    if let Some(burst) = bursts(spell, &content) {
        let radius = reach::blast_radius(burst, strength.power);
        let against = Against::opposing(side(cast.owner), cast.backfired);
        let figure = figure(cast.owner, burst);
        let strength = super::power::strength(figure, strength.power, strength.boost, burst);
        let strike = Strike::new(Rows::Spell(burst.id), strength, cast.owner, &content);
        for &victim in cast.victims.iter().filter(|&&victim| fallen(victim)) {
            let at = reaching.place(victim).feet;
            let area = Area::Blast { spot: at, origin: at, radius };
            let taken = reaching.takes(area, None, against);
            commands.spawn((Deed::at(strike.clone(), taken), Magical));
        }
    }
    let turnable = crate::game::hits::definitions::row(spell).is_some()
        && spell.aimed_at_enemies()
        && !cast.backfired
        && !cast.reflected;
    if !turnable {
        return Ok(());
    }
    let mut order = cast.victims.clone();
    order.sort_by_key(|victim| !reaching.party().any(|member| member == *victim));
    let Some((charged, eye)) = order
        .into_iter()
        .filter(|&victim| alive(victim))
        .filter_map(|victim| Some((victim, laying.turns_back(victim)?)))
        .reduce(|best, eye| if eye.1 > best.1 { eye } else { best })
    else {
        return Ok(());
    };
    laying.turn_back(charged);
    let power = strength.power.min(eye);
    news.write(MagicNews::Landed { effects: vec![Effect::Reflected { victim: charged }] });
    let first = reaching.party().find(|&member| fighters.contains(member)).unwrap_or(cast.caster);
    let party_eye = reaching.party().next().map_or(cast.from, |m| reaching.place(m).eye);
    let eye_of = |body: Entity| match vitals.contains(body) {
        true => reaching.place(body).eye,
        false => party_eye,
    };
    // Whom it comes from, and where from.
    let from_body = |lead: Entity| (Some(lead), eye_of(lead));
    let from_area = match cast.aim {
        Aim::Point(spot) => (cast.owner, spot),
        _ => (cast.owner, cast.from),
    };
    let returns = spell.targeting.aiming().returns;
    let (source, from) = match (returns, cast.aim) {
        (Returns::Blast | Returns::Cone, _) => from_area,
        (Returns::Reached, _) => (cast.owner, cast.from),
        (Returns::Group, Aim::Body(aimed)) => from_body(reaching.leader(aimed)),
        (_, Aim::Body(aimed)) => from_body(aimed),
        _ => (Some(first), party_eye),
    };
    // Whom it falls on: its owner's own side.
    let against = Against::opposing(side(cast.owner), true);
    let owner = cast.owner;
    let at_owner = owner.map_or(Aim::Point(cast.from), Aim::Body);
    let area = match returns {
        Returns::Owner => Some(Area::One(at_owner)),
        Returns::Group => owner.map(Area::Group),
        Returns::Blast => {
            let party = reaching.party().next().map_or(cast.from, |m| reaching.place(m).feet);
            let centre = owner
                .filter(|&owner| vitals.contains(owner))
                .map_or(party, |owner| reaching.place(owner).feet);
            let radius = reach::blast_radius(spell, power);
            Some(Area::Blast { spot: centre, origin: cast.from, radius })
        }
        Returns::Cone => {
            Some(Area::Cone { origin: from, spot: cast.from, reach: spell.reach.metres() })
        }
        Returns::Reached => {
            let radius = spell.reach.metres();
            Some(Area::Blast { spot: cast.from, origin: cast.from, radius })
        }
        Returns::NoOne => None,
    };
    let victims = area.map_or_else(Vec::new, |area| reaching.takes(area, None, against));
    let strength = super::power::strength(figure(source, spell), power, strength.boost, spell);
    let strike = Strike::new(Rows::Spell(spell.id), strength, Some(charged), &content);
    let flies = spell.missile.is_some();
    let flights: Vec<(Entity, Option<f32>)> = victims
        .iter()
        .map(|&victim| (victim, flies.then(|| flight(from, reaching.place(victim).eye))))
        .collect();
    let back = Spell {
        id: spell.id,
        caster: cast.caster,
        backfired: false,
        reflected: true,
        owner: source,
        from,
        aim: at_owner,
        victims,
    };
    commands.spawn((back, Deed::flying(strike, flights)));
    Ok(())
}

/// What a spell's fallen burst into, if its row says they do: Boiling
/// Blood's explosion.
fn bursts<'a>(spell: &SpellDefinition, content: &'a Catalogs) -> Option<&'a SpellDefinition> {
    let into = crate::game::hits::definitions::row(spell)?.bursts_into?;
    content.spells.get(into).filter(|burst| crate::game::hits::definitions::row(burst).is_some())
}
