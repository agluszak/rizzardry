//! Whom a monster could cast at (`docs/rules/monsters.md` section 5): what it
//! makes of each body it considers, the aims each slot it weighs has, what
//! each aim would take in, and whether an aim is worth casting.

use bevy::prelude::*;

use super::sight::MakesOutParty;
use super::slot::{Area, CastAt, Casts, Range, Works};
use crate::game::characters::{Conditions, Vitals};
use crate::game::data::Catalogs;
use crate::game::magic::reach::{Aim, blast_radius, in_cone};
use crate::game::party::roster::PartyMember;
use crate::game::world::hostility::{self, Side, Stance, Standing};
use crate::game::world::identify::GroupMember;
use crate::game::world::population::{Monster, SpellBudget};

/// A monster considering whom it could act on: whose. The deliberation at
/// its moment, or a look at the party from outside a fight.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Considering)]
pub struct Considers(pub Entity);

/// What a monster is considering; it goes with the monster.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Considers, linked_spawn)]
pub struct Considering(Vec<Entity>);

/// What a monster considering makes of one body.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Views)]
pub struct ViewOf(pub Entity);

/// What a monster considering makes of each living body it considers.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = ViewOf, linked_spawn)]
pub struct Views(Vec<Entity>);

/// The body a view is of.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = ViewedIn)]
pub struct Viewed(pub Entity);

/// The views made of a body; they go with it.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Viewed, linked_spawn)]
pub struct ViewedIn(Vec<Entity>);

/// The monster sees the body: a member if its eyes make out the party, a
/// monster if its eyes make it out (retail's monster-to-monster sight),
/// itself always.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Seen;

/// How the body stands toward the monster: itself a friend.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toward(pub Stance);

/// One way a slot could be cast, in the consideration it was made for.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = AimsMade)]
pub struct AimOf(pub Entity);

/// The aims a consideration made.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = AimOf, linked_spawn)]
pub struct AimsMade(Vec<Entity>);

/// The slot an aim is of.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = SlotAims)]
pub struct BySlot(pub Entity);

/// The aims made of a slot; they go with it.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = BySlot, linked_spawn)]
pub struct SlotAims(Vec<Entity>);

/// Where the aim is cast.
#[derive(Component, Clone, Copy, Debug)]
pub struct At(pub Aim);

/// A body an aim would take in.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = TakesIn)]
pub struct TakenIn(pub Entity);

/// The bodies an aim would take in.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = TakenIn, linked_spawn)]
pub struct TakesIn(Vec<Entity>);

/// Which body is taken in.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Takings)]
pub struct Taken(pub Entity);

/// The aims that would take a body in; they go with it.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Taken, linked_spawn)]
pub struct Takings(Vec<Entity>);

/// The body taken in would feel the spell (retail `MonsterSpellTargetOK`): a
/// heal the hurt, a refresh the tired, a condition one without it, an
/// enchantment one it is not laid on, a cure one bearing what it cures; a
/// spell that does none of these anyone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Feels;

/// At least half of those the aim takes in would feel it, and one does.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Worth;

/// The aim would take in a monster that stands with neither side.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Spoiled;

/// A consideration weighs a slot: its aims are made.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Weigh {
    #[event_target]
    pub slot: Entity,
    pub considering: Entity,
}

/// An aim and all it takes in are made.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Made {
    #[event_target]
    aim: Entity,
}

/// What a monster considering makes of each of `bodies`.
pub fn view(commands: &mut Commands, considering: Entity, bodies: impl Iterator<Item = Entity>) {
    for body in bodies {
        commands.spawn((ViewOf(considering), Viewed(body)));
    }
}

/// An aim of a slot, taking in `bodies`.
fn make(
    commands: &mut Commands,
    (considering, slot): (Entity, Entity),
    at: Aim,
    bodies: impl Iterator<Item = Entity>,
) {
    let aim = commands.spawn((AimOf(considering), BySlot(slot), At(at))).id();
    for body in bodies {
        commands.spawn((TakenIn(aim), Taken(body)));
    }
    commands.trigger(Made { aim });
}

/// The monster sees a body it views, or not.
fn see(
    made: On<Add<Viewed>>,
    mut commands: Commands,
    views: Query<(&ViewOf, &Viewed)>,
    considering: Query<&Considers>,
    members: Query<(), With<PartyMember>>,
    making_out: Query<(), With<MakesOutParty>>,
    eyesight: super::sight::Eyesight,
) -> Result {
    let view = made.entity;
    let (&ViewOf(parent), &Viewed(body)) = views.get(view)?;
    let &Considers(monster) = considering.get(parent)?;
    let seen = match members.contains(body) {
        true => making_out.contains(monster),
        false => body == monster || eyesight.sees(monster, body),
    };
    if seen {
        commands.entity(view).insert(Seen);
    }
    Ok(())
}

/// How a body it views stands toward the monster.
fn stand(
    made: On<Add<Viewed>>,
    mut commands: Commands,
    views: Query<(&ViewOf, &Viewed)>,
    considering: Query<&Considers>,
    standings: Query<&Standing>,
) -> Result {
    let view = made.entity;
    let (&ViewOf(parent), &Viewed(body)) = views.get(view)?;
    let &Considers(monster) = considering.get(parent)?;
    let mine = standings.get(monster)?;
    let theirs = standings.get(body)?;
    let stance =
        if body == monster { Stance::Friendly } else { hostility::between(*mine, *theirs) };
    commands.entity(view).insert(Toward(stance));
    Ok(())
}

/// A slot at one body has an aim at each body of the side it is cast at
/// that the monster sees and has within its reach, taking in that body; one
/// at the caster has one aim, at itself.
pub(super) fn at_bodies(
    weigh: On<Weigh>,
    mut commands: Commands,
    slots: Query<(&CastAt, &Range)>,
    considering: Query<(&Considers, &Views)>,
    views: Query<(&Viewed, &Toward), With<Seen>>,
    places: crate::game::actors::Places,
) -> Result {
    let Weigh { slot, considering: parent } = *weigh;
    let (&cast_at, &Range(range)) = slots.get(slot)?;
    let (&Considers(monster), viewed) = considering.get(parent)?;
    let side = match cast_at {
        CastAt::Itself => {
            make(&mut commands, (parent, slot), Aim::Body(monster), [monster].into_iter());
            return Ok(());
        }
        CastAt::Friends => Stance::Friendly,
        CastAt::Foes => Stance::Hostile,
    };
    for (&Viewed(body), &Toward(stance)) in views.iter_many(viewed.iter()).flatten() {
        if stance == side && places.gap(monster, body) <= range {
            make(&mut commands, (parent, slot), Aim::Body(body), [body].into_iter());
        }
    }
    Ok(())
}

/// A slot at a cone or a blast has an aim at each foe the monster sees and
/// has within its reach, taking in every living body in its cone (within its
/// reach of the monster and in its arc toward the foe) or its blast (within
/// its radius of the foe's feet with a clear line, at the power the
/// monster's budget comfortably covers).
#[allow(clippy::type_complexity)]
pub(super) fn at_areas(
    weigh: On<Weigh>,
    mut commands: Commands,
    slots: Query<(&Casts, &Area, &Range)>,
    considering: Query<(&Considers, &Views)>,
    views: Query<(&Viewed, &Toward), With<Seen>>,
    living: Query<(Entity, &Vitals), Or<(With<Monster>, With<PartyMember>)>>,
    budgets: Query<&SpellBudget>,
    places: crate::game::actors::Places,
    obstacles: crate::game::world::obstacles::WorldObstacles,
    content: Res<Catalogs>,
) -> Result {
    let Weigh { slot, considering: parent } = *weigh;
    let (&Casts(spell), area, &Range(range)) = slots.get(slot)?;
    let (&Considers(monster), viewed) = considering.get(parent)?;
    let spell = &content.spells[spell];
    let budget = budgets.get(monster).map_or(0, |budget| budget.0);
    let radius = blast_radius(spell, super::spell::comfortable(budget, spell));
    let eye = places.of(monster).eye;
    let alive = || living.iter().filter(|(_, vitals)| !vitals.is_dead()).map(|(body, _)| body);
    for (&Viewed(foe), &Toward(stance)) in views.iter_many(viewed.iter()).flatten() {
        if stance != Stance::Hostile || places.gap(monster, foe) > range {
            continue;
        }
        let target = places.of(foe).feet;
        let taken = alive().filter(|&body| {
            let there = places.of(body);
            match area {
                Area::Blast => {
                    there.centre.distance(target) - there.radius <= radius
                        && obstacles.clear(target, there.centre)
                }
                Area::Cone => {
                    places.gap(monster, body) <= range && in_cone(target - eye, there.feet - eye)
                }
            }
        });
        make(&mut commands, (parent, slot), Aim::Body(foe), taken);
    }
    Ok(())
}

/// A slot at a group has an aim at each group other than the monster's own,
/// the party or a monster group, taking in its members that stand against
/// the monster, aimed at the first of them within its reach (the party only
/// if the monster makes it out; retail `IsTargetSourceInRangeOfGroup` asks
/// no sight of a monster group).
#[allow(clippy::type_complexity)]
pub(super) fn at_groups(
    weigh: On<Weigh>,
    mut commands: Commands,
    slots: Query<&Range>,
    considering: Query<(&Considers, &Views)>,
    making_out: Query<(), With<MakesOutParty>>,
    views: Query<(&Viewed, &Toward)>,
    groups: Query<AnyOf<(&GroupMember, &PartyMember)>>,
    places: crate::game::actors::Places,
) -> Result {
    let Weigh { slot, considering: parent } = *weigh;
    let &Range(range) = slots.get(slot)?;
    let (&Considers(monster), viewed) = considering.get(parent)?;
    let sees_party = making_out.contains(monster);
    let group_of = |body: Entity| match groups.get(body) {
        Ok((Some(group), _)) => Some((group.0, false)),
        Ok((None, Some(party))) => Some((party.0, true)),
        _ => None,
    };
    let own = group_of(monster).map(|(group, _)| group);
    let foes: Vec<(Entity, (Entity, bool))> = views
        .iter_many(viewed.iter())
        .flatten()
        .filter(|(_, toward)| toward.0 == Stance::Hostile)
        .filter_map(|(&Viewed(body), _)| Some((body, group_of(body)?)))
        .filter(|&(_, (group, _))| Some(group) != own)
        .collect();
    let mut groups_seen: Vec<Entity> = Vec::new();
    for &(_, (group, party)) in &foes {
        if groups_seen.contains(&group) {
            continue;
        }
        groups_seen.push(group);
        let members = || foes.iter().filter(move |foe| foe.1.0 == group).map(|foe| foe.0);
        let reached = (sees_party || !party)
            .then(|| members().find(|&body| places.gap(monster, body) <= range))
            .flatten();
        if let Some(first) = reached {
            make(&mut commands, (parent, slot), Aim::Body(first), members());
        }
    }
    Ok(())
}

/// A slot cast ahead has one aim if any foe the monster sees is within its
/// reach, taking in every foe it views.
pub(super) fn ahead(
    weigh: On<Weigh>,
    mut commands: Commands,
    slots: Query<&Range>,
    considering: Query<(&Considers, &Views)>,
    views: Query<(&Viewed, &Toward, Has<Seen>)>,
    places: crate::game::actors::Places,
) -> Result {
    let Weigh { slot, considering: parent } = *weigh;
    let &Range(range) = slots.get(slot)?;
    let (&Considers(monster), viewed) = considering.get(parent)?;
    let foes = || {
        views
            .iter_many(viewed.iter())
            .flatten()
            .filter(|(_, toward, _)| toward.0 == Stance::Hostile)
    };
    let reached = foes().any(|(&Viewed(foe), _, seen)| seen && places.gap(monster, foe) <= range);
    if reached {
        let taken = foes().map(|(&Viewed(foe), ..)| foe).collect::<Vec<_>>();
        make(&mut commands, (parent, slot), Aim::Ahead, taken.into_iter());
    }
    Ok(())
}

/// A body taken in feels the spell, or not.
fn feel(
    made: On<Add<Taken>>,
    mut commands: Commands,
    taken: Query<(&TakenIn, &Taken)>,
    aims: Query<&BySlot>,
    slots: Query<(&Casts, Option<&Works>)>,
    bodies: Query<(&Vitals, &Conditions)>,
    laid: crate::game::actors::Laid,
) -> Result {
    let taking = made.entity;
    let (&TakenIn(aim), &Taken(body)) = taken.get(taking)?;
    let &BySlot(slot) = aims.get(aim)?;
    let (&Casts(spell), works) = slots.get(slot)?;
    let feels = bodies.get(body)?;
    if works.is_none_or(|works| works.0.worthwhile(feels, laid.has(body, spell))) {
        commands.entity(taking).insert(Feels);
    }
    Ok(())
}

/// An aim is worth casting when at least half of those it takes in would
/// feel it, and one does (retail `MonsterSpellTargetOK` and
/// `HalfSpellTargetsValid`). An aim taking in nobody is worth nothing.
fn judge(
    made: On<Made>,
    mut commands: Commands,
    aims: Query<Option<&TakesIn>>,
    feeling: Query<Has<Feels>>,
) -> Result {
    let Some(taken) = aims.get(made.aim)? else { return Ok(()) };
    let felt = feeling.iter_many(taken.iter()).flatten().filter(|&feels| feels).count();
    if felt != 0 && taken.len().div_ceil(2) <= felt {
        commands.entity(made.aim).insert(Worth);
    }
    Ok(())
}

/// An aim of a spell against enemies that would take in a living monster
/// standing with neither side, other than the caster, is spoiled (retail
/// `SpellAreaHitsNeutralMonster`).
fn spoil(
    made: On<Made>,
    mut commands: Commands,
    aims: Query<(&AimOf, &BySlot, Option<&TakesIn>)>,
    against_enemies: Query<(), With<super::slot::AtEnemies>>,
    considering: Query<&Considers>,
    taken: Query<&Taken>,
    neutrals: Query<(&Side, &Vitals), With<Monster>>,
) -> Result {
    let (&AimOf(parent), &BySlot(slot), takes) = aims.get(made.aim)?;
    if !against_enemies.contains(slot) {
        return Ok(());
    }
    let &Considers(monster) = considering.get(parent)?;
    let takes = takes.into_iter().flat_map(|takes| takes.iter());
    let neutral = taken.iter_many(takes).flatten().any(|&Taken(body)| {
        body != monster
            && neutrals
                .get(body)
                .is_ok_and(|(side, vitals)| side.own() == Stance::Neutral && !vitals.is_dead())
    });
    if neutral {
        commands.entity(made.aim).insert(Spoiled);
    }
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(see)
        .add_observer(stand)
        .add_observer(feel)
        .add_observer(judge)
        .add_observer(spoil);
}
