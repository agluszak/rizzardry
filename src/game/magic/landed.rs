//! What follows once all of a deed of magic has landed, each its own
//! observer, set by whoever put what it follows beside the deed: the telling
//! of it ([`Magical`]), a drain's share ([`Leech`]), Boiling Blood's bursts
//! and Eye for an Eye ([`Spell`]), its caster's practice ([`practise`]) and
//! an auto recast ([`recast`]). A burst or a reflection is an aim, and then a
//! deed of its own.

use bevy::prelude::*;

use super::aim::{Against, At, Origin, Radius, Reach, Spot, Take, Takes, Took, Victims};
use super::casting::{MagicNews, Practice, Recast, Source};
use super::reach::{self, Aim, flight};
use super::{MagicRequest, SpellPoints};
use crate::content::spells::{SpellDefinition, SpellId, Targeting};
use crate::game::actors::{CanFight, CasterLevels, Laying, Places};
use crate::game::characters::{Practise, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::definitions::{Leeches, Rows};
use crate::game::hits::effects::Strike;
use crate::game::hits::{Deed, Effect, Landed};
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::Standing;
use crate::game::world::identify::{GroupMember, GroupMembers, Leads};
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
    world
        .commands()
        .entity(context.entity)
        .observe(burst)
        .observe(turn_back)
        .observe(comes_back_from)
        .observe(turned_aim);
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
pub fn practise(landed: On<Landed>, deeds: Query<&Practice>, mut commands: Commands) -> Result {
    let practice = deeds.get(landed.deed)?;
    if practice.sure || landed.affected() {
        for &(skill, usage) in &practice.skills {
            commands.trigger(Practise { member: practice.caster, skill, usage });
        }
    }
    Ok(())
}

/// An auto recast goes on once its deed has landed, while a living victim
/// is not yet whole in what the spell restores, and its caster's stamina and
/// points last: set on the deed beside its [`Recast`].
pub fn recast(
    landed: On<Landed>,
    deeds: Query<&Recast>,
    vitals: Query<&Vitals>,
    casters: Query<(&Vitals, &SpellPoints)>,
    content: Res<Catalogs>,
    mut requests: MessageWriter<MagicRequest>,
) -> Result {
    let recast = deeds.get(landed.deed)?;
    let Source::Spell { spell, .. } = recast.choice.source else { return Ok(()) };
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
        super::casting::goes_again((vitals.stamina.current, pool), &content.spells[spell])
    });
    if recast.victims.iter().any(short) && goes_again {
        requests.write(MagicRequest { caster: recast.caster, choice: recast.choice });
    }
    Ok(())
}

/// What a burst strikes, carried by its aim until it has taken in whom.
#[derive(Component, Clone, Debug)]
struct Burst(Strike);

/// A spell turned back, carried by its aim until it has taken in whom: who
/// paid the charge, its strike, the spell it is now, and whether it flies.
#[derive(Component, Clone, Debug)]
struct TurnedBack {
    charged: Entity,
    strike: Strike,
    spell: Spell,
    flies: bool,
}

/// Once a spell has landed, each of its victims that died bursts where it
/// fell, if the spell's row says it does (Boiling Blood): at the cast's
/// power, from the spell's owner, a blast at the fallen's feet against
/// whoever opposes the owner's side (the owner's own side if the spell
/// backfired, anyone if it has no owner), as strong as its owner. Then it may
/// be turned back ([`MayTurnBack`]).
#[allow(clippy::type_complexity)]
fn burst(
    landed: On<Landed>,
    deeds: Query<(&Spell, &Deed)>,
    vitals: Query<(&Vitals, Has<Untargetable>)>,
    (standings, levels): (Query<&Standing>, Query<&CasterLevels>),
    places: Places,
    content: Res<Catalogs>,
    mut commands: Commands,
) -> Result {
    let deed = landed.deed;
    let (cast, made) = deeds.get(deed)?;
    let strength = made.strike.strength;
    let explosion = content
        .spells
        .get(cast.id)
        .and_then(crate::game::hits::definitions::row)
        .and_then(|row| row.bursts_into)
        .and_then(|into| content.spells.get(into))
        .filter(|explosion| crate::game::hits::definitions::row(explosion).is_some());
    if let Some(explosion) = explosion {
        let side = cast.owner.and_then(|owner| standings.get(owner).ok()).map(|s| s.stance);
        let against = match side {
            Some(side) if cast.backfired => Against::Opponents(side).reversed(),
            Some(side) => Against::Opponents(side),
            None => Against::Everyone,
        };
        let radius = Radius(reach::blast_radius(explosion, strength.power));
        let figure = caster_level(cast.owner, explosion, &levels);
        let strength = super::power::strength(figure, strength.power, strength.boost, explosion);
        let strike = Strike::new(Rows::Spell(explosion.id), strength, cast.owner, &content);
        let fallen = cast.victims.iter().filter(|&&victim| {
            vitals.get(victim).is_ok_and(|(vitals, never)| !never && vitals.is_dead())
        });
        for &victim in fallen {
            let at = places.of(victim).feet;
            let blast = (Takes::Blast, Spot(at), Origin(at), radius, against);
            let aim = commands.spawn((blast, Burst(strike.clone()))).observe(burst_taken).id();
            commands.trigger(Take { aim });
        }
    }
    commands.trigger(MayTurnBack { deed });
    Ok(())
}

/// The caster level whoever a spell comes from casts it at; none for no one.
fn caster_level(
    owner: Option<Entity>,
    spell: &SpellDefinition,
    levels: &Query<&CasterLevels>,
) -> i32 {
    owner.and_then(|owner| levels.get(owner).ok()).map_or(0, |owned| owned.caster_level(spell))
}

/// A spell's bursts are set off: it may now be turned back.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct MayTurnBack {
    #[event_target]
    pub deed: Entity,
}

/// An Eye for an Eye turns a spell back: who paid the charge, and the power
/// it comes back at.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct TurnedBy {
    #[event_target]
    pub deed: Entity,
    pub charged: Entity,
    pub power: u32,
}

/// Whom a spell turned back comes from (whose level it is as strong as) and
/// where from.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct ComesBack {
    #[event_target]
    pub deed: Entity,
    pub charged: Entity,
    pub power: u32,
    pub source: Option<Entity>,
    pub from: Vec3,
}

/// A spell aimed at enemies (not a backfire, and not one turned back
/// already) is turned back by the strongest Eye for an Eye among its living
/// victims, members first in the order the cast took them, then monsters,
/// the first of equals, who pays a charge (retail `FinishSpellEffectTargets`),
/// at the lesser of the cast's power and the eye's.
fn turn_back(
    may: On<MayTurnBack>,
    deeds: Query<(&Spell, &Deed)>,
    vitals: Query<(&Vitals, Has<Untargetable>)>,
    roster: Single<&PartyRoster>,
    mut laying: Laying,
    content: Res<Catalogs>,
    mut commands: Commands,
) -> Result {
    let deed = may.deed;
    let (cast, made) = deeds.get(deed)?;
    let spell = &content.spells[cast.id];
    if crate::game::hits::definitions::row(spell).is_none()
        || !spell.aimed_at_enemies()
        || cast.backfired
        || cast.reflected
    {
        return Ok(());
    }
    let living =
        |body: Entity| vitals.get(body).is_ok_and(|(vitals, never)| !never && !vitals.is_dead());
    let mut order = cast.victims.clone();
    order.sort_by_key(|victim| !roster.contains(victim));
    let Some((charged, eye_power)) = order
        .into_iter()
        .filter(|&victim| living(victim))
        .filter_map(|victim| Some((victim, laying.turns_back(victim)?)))
        .reduce(|best, eye| if eye.1 > best.1 { eye } else { best })
    else {
        return Ok(());
    };
    laying.turn_back(charged);
    let power = made.strike.strength.power.min(eye_power);
    commands.trigger(TurnedBy { deed, charged, power });
    Ok(())
}

/// Whom and where from a spell turned back comes: a blast or a cone from
/// where it went off, an all-enemies spell from where it set out, as its
/// owner; a group spell from the leader of the group aimed at (`Leads`); a
/// spell at a body from that body; anything else from the first member able
/// to fight, at the party's eye.
#[allow(clippy::type_complexity)]
fn comes_back_from(
    turned: On<TurnedBy>,
    deeds: Query<&Spell>,
    bodies: Query<(), With<Vitals>>,
    fighters: Query<(), With<CanFight>>,
    grouping: (Query<&GroupMember>, Query<&GroupMembers>, Query<(), With<Leads>>),
    roster: Single<&PartyRoster>,
    places: Places,
    content: Res<Catalogs>,
    mut commands: Commands,
) -> Result {
    let TurnedBy { deed, charged, power } = *turned;
    let cast = deeds.get(deed)?;
    let eye = roster.first().map_or(cast.from, |&member| places.of(member).eye);
    let eye_of = |actor: Entity| if bodies.contains(actor) { places.of(actor).eye } else { eye };
    // The leader of a body's monster group, or the body itself.
    let leader = |body: Entity| {
        grouping
            .0
            .get(body)
            .and_then(|member| grouping.1.get(member.0))
            .ok()
            .and_then(|members| members.iter().find(|&mate| grouping.2.contains(mate)))
            .unwrap_or(body)
    };
    let (source, from) = match (content.spells[cast.id].targeting, cast.aim) {
        (Targeting::Radius | Targeting::Cone, Aim::Point(at)) => (cast.owner, at),
        (Targeting::Radius | Targeting::Cone | Targeting::AllEnemies, _) => (cast.owner, cast.from),
        (Targeting::EnemyGroup, Aim::Body(entity)) => {
            let leader = leader(entity);
            (Some(leader), eye_of(leader))
        }
        (_, Aim::Body(entity)) => (Some(entity), eye_of(entity)),
        _ => {
            let first =
                roster.iter().find(|&member| fighters.contains(member)).unwrap_or(cast.caster);
            (Some(first), eye)
        }
    };
    commands.trigger(ComesBack { deed, charged, power, source, from });
    Ok(())
}

/// A spell turned back falls on its owner's side: an aimed spell on its
/// owner, a group spell on the owner's group, a blast on whoever of that
/// side stands within its radius of the owner, a cone on whoever is in it
/// from where it comes back toward where it went off, an all-enemies spell
/// on whoever is within its reach of where it set out. It is a deed of its
/// own, as strong as its new source, flying back if the spell is a missile.
#[allow(clippy::type_complexity)]
fn turned_aim(
    comes: On<ComesBack>,
    deeds: Query<(&Spell, &Deed)>,
    standings: Query<&Standing>,
    (bodies, levels): (Query<(), With<Vitals>>, Query<&CasterLevels>),
    roster: Single<&PartyRoster>,
    places: Places,
    content: Res<Catalogs>,
    mut commands: Commands,
) -> Result {
    let ComesBack { deed, charged, power, source, from } = *comes;
    let (cast, made) = deeds.get(deed)?;
    let spell = &content.spells[cast.id];
    let owner = cast.owner;
    let against = match owner.and_then(|owner| standings.get(owner).ok()) {
        Some(standing) => Against::Opponents(standing.stance).reversed(),
        None => Against::Everyone,
    };
    let at_owner = At(owner.map_or(Aim::Point(cast.from), Aim::Body));
    let mut aim = match spell.targeting {
        Targeting::EnemyGroup => commands.spawn((Takes::Group, at_owner)),
        Targeting::Radius => {
            let party = roster.first().map_or(cast.from, |&member| places.of(member).feet);
            let centre = owner
                .filter(|&owner| bodies.contains(owner))
                .map_or(party, |owner| places.of(owner).feet);
            let radius = Radius(reach::blast_radius(spell, power));
            commands.spawn((Takes::Blast, Spot(centre), Origin(cast.from), radius, against))
        }
        Targeting::Cone => {
            let reach = Reach(spell.reach.metres());
            commands.spawn((Takes::Cone, Origin(from), Spot(cast.from), reach, against))
        }
        Targeting::AllEnemies => {
            let radius = Radius(spell.reach.metres());
            commands.spawn((Takes::Blast, Spot(cast.from), Origin(cast.from), radius, against))
        }
        Targeting::Enemy => commands.spawn((Takes::One, at_owner)),
        // Nobody else.
        _ => commands.spawn((Takes::One, At(Aim::Point(cast.from)))),
    };
    let boost = made.strike.strength.boost;
    let strength =
        super::power::strength(caster_level(source, spell, &levels), power, boost, spell);
    let turned = TurnedBack {
        charged,
        strike: Strike::new(Rows::Spell(spell.id), strength, Some(charged), &content),
        spell: Spell {
            id: spell.id,
            caster: cast.caster,
            backfired: false,
            reflected: true,
            owner: source,
            from,
            aim: at_owner.0,
            victims: Vec::new(),
        },
        flies: spell.missile.is_some(),
    };
    let aim = aim.insert(turned).observe(turned_back).id();
    commands.trigger(Take { aim });
    Ok(())
}

/// A burst has taken in whom it strikes: it is a deed of its own.
fn burst_taken(took: On<Took>, aims: Query<(&Burst, &Victims)>, mut commands: Commands) -> Result {
    let (Burst(strike), Victims(victims)) = aims.get(took.aim)?;
    commands.spawn((Deed::at(strike.clone(), victims.iter().copied()), Magical));
    commands.entity(took.aim).despawn();
    Ok(())
}

/// A spell turned back has taken in whom it strikes: the turning back is
/// told, and it is a deed of its own, flying back to each if it is a missile.
fn turned_back(
    took: On<Took>,
    aims: Query<(&TurnedBack, &Victims)>,
    places: Places,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) -> Result {
    let (turned, Victims(victims)) = aims.get(took.aim)?;
    news.write(MagicNews::Landed { effects: vec![Effect::Reflected { victim: turned.charged }] });
    let from = turned.spell.from;
    let flights = victims
        .iter()
        .map(|&victim| (victim, turned.flies.then(|| flight(from, places.of(victim).eye))));
    let spell = Spell { victims: victims.clone(), ..turned.spell.clone() };
    commands.spawn((spell, Deed::flying(turned.strike.clone(), flights)));
    commands.entity(took.aim).despawn();
    Ok(())
}
