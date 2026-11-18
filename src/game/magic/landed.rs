//! What follows once all of a deed of magic has landed: the telling of it
//! ([`Magical`]), a drain's share ([`Leech`]), Boiling Blood's bursts
//! ([`Bursts`]) and Eye for an Eye ([`TurnsBack`]), its caster's practice
//! ([`practise`]) and an auto recast ([`recast`]). Each is put beside the
//! deed as it is let go; a burst or a reflection is a deed of its own.

use bevy::prelude::*;

use super::Strength;
use super::aim::{Against, Area, Reaching};
use super::casting::{MagicNews, Practice, Recast};
use super::reach::{self, Aim, flight};
use super::{MagicRequest, SpellPoints};
use crate::content::spells::{Returns, SpellDefinition};
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

/// A deed of a cast, which counts as an attack in a fight: whom it was let
/// go at.
#[derive(Component, Clone, Debug)]
#[require(Magical)]
pub struct LetGoAt(pub Vec<Entity>);

/// Each of a deed's victims it killed bursts where it fell, once all of it
/// has landed (Boiling Blood): a blast of this strike, this wide, against
/// these.
#[derive(Component, Clone, Debug)]
pub struct Bursts {
    strike: Strike,
    radius: f32,
    against: Against,
}

impl Bursts {
    /// What a spell's fallen burst into, if its row says they do: Boiling
    /// Blood's explosion, at the deed's power and boost, from `owner`, as
    /// strong as its `figure` for the explosion is, against `against`.
    pub fn of(
        spell: &SpellDefinition,
        strength: Strength,
        owner: Option<Entity>,
        figure: impl Fn(&SpellDefinition) -> i32,
        against: Against,
        content: &Catalogs,
    ) -> Option<Self> {
        let into = crate::game::hits::definitions::row(spell)?.bursts_into?;
        let burst = content.spells.get(into)?;
        crate::game::hits::definitions::row(burst)?;
        let radius = reach::blast_radius(burst, strength.power);
        let strength = super::power::strength(figure(burst), strength.power, strength.boost, burst);
        let strike = Strike::new(Rows::Spell(burst.id), strength, owner, content);
        Some(Self { strike, radius, against })
    }
}

/// A spell Eye for an Eye may turn back once it has landed: one aimed at
/// enemies, neither a backfire nor turned back already, so its caster's own.
/// What it is, who cast it, where it set out from and what it was aimed at.
#[derive(Component, Clone, Debug)]
pub struct TurnsBack {
    pub spell: SpellDefinition,
    pub caster: Entity,
    pub from: Vec3,
    pub aim: Aim,
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

/// A spell turned back by this body's Eye for an Eye.
#[derive(Component, Clone, Copy, Debug)]
pub struct TurnedBackBy(pub Entity);

/// What a deed of magic did is told as it lands; a spell turned back is told
/// so on its own first, after whatever burst before it was turned.
fn tell(landed: On<Landed>, turned: Query<&TurnedBackBy>, mut news: MessageWriter<MagicNews>) {
    if let Ok(&TurnedBackBy(victim)) = turned.get(landed.deed) {
        news.write(MagicNews::Landed { effects: vec![Effect::Reflected { victim }] });
    }
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
    deeds: Query<(&Recast, &LetGoAt)>,
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
    if deed.0.iter().any(short) && goes_again {
        requests.write(MagicRequest { caster: recast.caster, choice: recast.choice });
    }
}

/// Once a deed of a cast has landed, its fallen burst ([`Bursts`]), then
/// the strongest Eye for an Eye among its living victims may turn it back
/// ([`TurnsBack`]): one moment, in that order.
#[allow(clippy::type_complexity)]
pub fn follow_spell(
    landed: On<Landed>,
    deeds: Query<(&LetGoAt, &Deed, Option<&Bursts>, Option<&TurnsBack>)>,
    vitals: Query<(&Vitals, Has<Untargetable>)>,
    (standings, levels, fighters): (
        Query<&Standing>,
        Query<&CasterLevels>,
        Query<(), With<CanFight>>,
    ),
    reaching: Reaching,
    mut laying: Laying,
    content: Res<Catalogs>,
    mut commands: Commands,
) {
    let Ok((LetGoAt(victims), made, bursts, turns)) = deeds.get(landed.deed) else { return };
    let fallen = |body: Entity| vitals.get(body).is_ok_and(|(v, never)| !never && v.is_dead());
    if let Some(bursts) = bursts {
        for &victim in victims.iter().filter(|&&victim| fallen(victim)) {
            let at = reaching.place(victim).feet;
            let area = Area::Blast { spot: at, origin: at, radius: bursts.radius };
            let taken = reaching.takes(area, None, bursts.against);
            commands.spawn((Deed::at(bursts.strike.clone(), taken), Magical));
        }
    }
    let Some(turns) = turns else { return };
    let alive = |body: Entity| vitals.get(body).is_ok_and(|(v, never)| !never && !v.is_dead());
    let mut order = victims.clone();
    order.sort_by_key(|victim| !reaching.party().any(|member| member == *victim));
    let Some((charged, eye)) = order
        .into_iter()
        .filter(|&victim| alive(victim))
        .filter_map(|victim| Some((victim, laying.turns_back(victim)?)))
        .reduce(|best, eye| if eye.1 > best.1 { eye } else { best })
    else {
        return;
    };
    laying.turn_back(charged);
    let strength = made.strike.strength;
    let power = strength.power.min(eye);
    let &TurnsBack { ref spell, caster, from: set_out, aim } = turns;
    let owner = caster;
    let first = reaching.party().find(|&member| fighters.contains(member)).unwrap_or(caster);
    let party_eye = reaching.party().next().map_or(set_out, |m| reaching.place(m).eye);
    let eye_of = |body: Entity| match vitals.contains(body) {
        true => reaching.place(body).eye,
        false => party_eye,
    };
    // Whom it comes from (whose level it is as strong as), and where from.
    let from_body = |lead: Entity| (lead, eye_of(lead));
    let returns = spell.targeting.aiming().returns;
    let (source, from) = match (returns, aim) {
        (Returns::Blast | Returns::Cone, Aim::Point(spot)) => (owner, spot),
        (Returns::Blast | Returns::Cone | Returns::Reached, _) => (owner, set_out),
        (Returns::Group, Aim::Body(aimed)) => from_body(reaching.leader(aimed)),
        (_, Aim::Body(aimed)) => from_body(aimed),
        _ => (first, party_eye),
    };
    // It falls on its owner's own side.
    let against = Against::opposing(standings.get(owner).ok().map(|s| s.stance), true);
    let area = match returns {
        Returns::Owner => Some(Area::One(Aim::Body(owner))),
        Returns::Group => Some(Area::Group(owner)),
        Returns::Blast => {
            let party = reaching.party().next().map_or(set_out, |m| reaching.place(m).feet);
            let centre = match vitals.contains(owner) {
                true => reaching.place(owner).feet,
                false => party,
            };
            let radius = reach::blast_radius(spell, power);
            Some(Area::Blast { spot: centre, origin: set_out, radius })
        }
        Returns::Cone => {
            Some(Area::Cone { origin: from, spot: set_out, reach: spell.reach.metres() })
        }
        Returns::Reached => {
            let radius = spell.reach.metres();
            Some(Area::Blast { spot: set_out, origin: set_out, radius })
        }
        Returns::NoOne => None,
    };
    let victims = area.map_or_else(Vec::new, |area| reaching.takes(area, None, against));
    let figure = |spell: &SpellDefinition| levels.get(source).map_or(0, |l| l.caster_level(spell));
    let strength = super::power::strength(figure(spell), power, strength.boost, spell);
    let strike = Strike::new(Rows::Spell(spell.id), strength, Some(charged), &content);
    let flies = spell.missile.is_some();
    let flights: Vec<(Entity, Option<f32>)> = victims
        .iter()
        .map(|&victim| (victim, flies.then(|| flight(from, reaching.place(victim).eye))))
        .collect();
    let side = standings.get(source).ok().map(|s| s.stance);
    let bursts =
        Bursts::of(spell, strength, Some(source), figure, Against::opposing(side, false), &content);
    let mut back = commands.spawn((LetGoAt(victims), TurnedBackBy(charged)));
    if let Some(bursts) = bursts {
        back.insert(bursts);
    }
    back.insert(Deed::flying(strike, flights));
}
