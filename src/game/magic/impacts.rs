//! Finalization is accepted work on the last-arriving missile of a cast.
//! It runs after every original hit, once, and retains the original aim.
use super::{
    Strength,
    casting::{self, Aim, SpellInFlight},
    effects::Field,
    targeting,
};
use crate::{
    content::spells::{SpellDefinition, SpellId, Targeting},
    game::hits::{Effect, reach::Recipients},
    game::world::hostility::Stance,
};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug)]
pub enum Origin {
    Actor(Entity),
    Point { at: Vec3, owner: Option<Entity> },
}
impl Origin {
    pub fn actor(self) -> Option<Entity> {
        match self {
            Self::Actor(actor) => Some(actor),
            Self::Point { .. } => None,
        }
    }
    fn owner(self) -> Option<Entity> {
        match self {
            Self::Actor(actor) => Some(actor),
            Self::Point { owner, .. } => owner,
        }
    }
    pub fn position(self, world: &Field, party: Vec3) -> Vec3 {
        match self {
            Self::Point { at, .. } => at,
            Self::Actor(actor) => {
                world.monsters.monsters().get(actor).map_or(party, |(body, at, walker, ..)| {
                    body.eye(crate::game::world::navigation::feet(at.translation(), walker))
                })
            }
        }
    }
    fn ground(self, world: &Field, party: Vec3) -> Vec3 {
        match self {
            Self::Point { at, .. } => at,
            Self::Actor(actor) => world.monsters.monsters().get(actor).map_or_else(
                |_| crate::game::party::body::PartyBody::feet_below(party),
                |(_, at, walker, ..)| {
                    crate::game::world::navigation::feet(at.translation(), walker)
                },
            ),
        }
    }
    pub(crate) fn recipients(self, world: &Field, reversed: bool) -> Recipients {
        let Some(owner) = self.owner() else { return Recipients::Everyone };
        let side = match world.members.get(owner) {
            Ok((_, vitals, ..)) => crate::game::world::hostility::member_stance(
                vitals.conditions.has(crate::game::characters::Condition::Turncoat),
            ),
            Err(_) => world.monsters.stance(owner, world.now),
        };
        Recipients::Opponents(if reversed {
            match side {
                Stance::Hostile => Stance::Friendly,
                Stance::Friendly => Stance::Hostile,
                Stance::Neutral => Stance::Neutral,
            }
        } else {
            side
        })
    }
    pub(crate) fn figure(self, world: &Field, spell: &SpellDefinition) -> u32 {
        let Some(owner) = self.owner() else { return 0 };
        match world.members.get(owner) {
            Ok((profile, ..)) => super::total_caster_level(profile, spell.books).max(0) as u32,
            Err(_) => world
                .monsters
                .monsters()
                .get(owner)
                .map_or(0, |(body, ..)| u32::from(world.species[body.species].level)),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Finalization {
    pub aim: Aim,
    pub victims: Vec<Entity>,
    pub reflected: bool,
}

pub(crate) fn attach(flights: &mut [SpellInFlight], finish: Finalization) {
    if let Some((index, _)) =
        flights.iter().enumerate().max_by_key(|(index, flight)| (flight.arrives.duration(), *index))
    {
        flights[index].finish = Some(finish);
    }
}

/// Land a spell from `origin` on `victims` as part of `deed`, and what its
/// last impact brings once: the strongest Eye for an Eye among its victims
/// turns it back on its source, and Boiling Blood's dead burst once its
/// hits have landed. Reflected missiles are returned to be flown.
#[allow(clippy::too_many_arguments)]
pub(crate) fn land(
    commands: &mut Commands,
    (deed, effects): (Entity, &mut Vec<Effect>),
    (caster, origin, backfired): (Entity, Origin, bool),
    (spell, strength): (&SpellDefinition, Strength),
    victims: &[Entity],
    finish: Option<&Finalization>,
    party_at: Vec3,
    world: &mut Field,
    flights: &mut Vec<SpellInFlight>,
) {
    use crate::game::hits::spells::{self, Kind};
    spells::land(commands, deed, (spell.id, strength, backfired), victims, world.party);
    if let (Some(Kind::Lifesteal | Kind::MightToMagic), Some(actor)) =
        (spells::kind(spell.id), origin.actor())
    {
        commands.entity(deed).insert(spells::Drains {
            caster: actor,
            lifesteal: spells::kind(spell.id) == Some(Kind::Lifesteal),
            drained: 0,
        });
    }
    let Some(finish) = finish else { return };
    if spell.id == SpellId::BOILING_BLOOD {
        let explosion = &world.spells[SpellId::BOILING_BLOOD_EXPLOSION];
        commands.entity(deed).insert(spells::Bursts {
            caster,
            victims: finish.victims.clone(),
            strength,
            figure: origin.figure(world, explosion),
            recipients: origin.recipients(world, backfired),
        });
    }
    // The strongest surviving charge pays once, after all original impacts.
    if backfired || finish.reflected || !spell.aimed_at_enemies() {
        return;
    }
    let mut order = finish.victims.clone();
    order.sort_by_key(|&victim| match world.party.iter().position(|&member| member == victim) {
        Some(seat) => (0, seat),
        None => (1, world.monsters.monsters().get(victim).unwrap().0.location as usize),
    });
    let mut best = None;
    for victim in order {
        let v = world.vitals(victim);
        let effect = v.enchantments.get(SpellId::EYE_FOR_AN_EYE).filter(|_| !v.is_dead()).copied();
        if let Some(effect) = effect
            && effect.power > best.map_or(0, |(_, power)| power)
        {
            best = Some((victim, effect.power));
        }
    }
    let Some((charged, power)) = best else { return };
    world.vitals_mut(charged).enchantments.spend_charge(SpellId::EYE_FOR_AN_EYE);
    effects.push(Effect::Reflected { victim: charged });
    let original = origin.owner();
    let old_position = origin.position(world, party_at);
    let first_able = || {
        world
            .party
            .iter()
            .copied()
            .find(|&actor| world.members.get(actor).unwrap().1.can_fight())
            .unwrap_or(caster)
    };
    let source = match spell.targeting {
        Targeting::Radius | Targeting::Cone => Origin::Point {
            at: match finish.aim {
                Aim::Point(at) => at,
                _ => old_position,
            },
            owner: original,
        },
        Targeting::AllEnemies => origin,
        _ => match finish.aim {
            Aim::Monster(entity) if spell.targeting == Targeting::EnemyGroup => {
                Origin::Actor(world.monsters.leader(entity))
            }
            Aim::Monster(entity) => Origin::Actor(entity),
            _ => Origin::Actor(first_able()),
        },
    };
    let victims = match spell.targeting {
        Targeting::Enemy => original.into_iter().collect(),
        Targeting::EnemyGroup => match original {
            Some(owner) if world.members.contains(owner) => world
                .party
                .iter()
                .copied()
                .filter(|&member| !world.members.get(member).unwrap().1.is_dead())
                .collect(),
            Some(owner) => world.monsters.group(owner),
            None => vec![],
        },
        Targeting::Radius => targeting::radius(
            world,
            origin.ground(world, party_at),
            old_position,
            spell.radius + spell.radius_per_power * strength.power.min(power) as f32,
            Some(party_at),
            origin.recipients(world, true),
        ),
        Targeting::Cone => {
            let point = source.position(world, party_at);
            targeting::radius(
                world,
                point,
                point,
                spell.reach.metres(),
                Some(party_at),
                origin.recipients(world, true),
            )
            .into_iter()
            .filter(|&victim| {
                casting::in_cone(
                    old_position - point,
                    Origin::Actor(victim).position(world, party_at) - point,
                )
            })
            .collect()
        }
        Targeting::AllEnemies => targeting::radius(
            world,
            old_position,
            old_position,
            spell.reach.metres(),
            Some(party_at),
            origin.recipients(world, true),
        ),
        _ => vec![],
    };
    let strength =
        Strength::at(source.figure(world, spell), spell, strength.power.min(power), strength.boost);
    let aim = match original {
        Some(owner) if world.members.contains(owner) => Aim::Member(owner),
        Some(owner) => Aim::Monster(owner),
        None => Aim::Point(old_position),
    };
    let finish = Finalization { aim, victims: victims.clone(), reflected: true };
    if spell.missile.is_some() {
        let start = flights.len();
        let from = source.position(world, party_at);
        for victim in victims {
            let seconds = from.distance(Origin::Actor(victim).position(world, party_at))
                / casting::MISSILE_SPEED;
            flights.push(SpellInFlight {
                caster,
                origin: source,
                spell: spell.id,
                strength,
                victims: vec![victim],
                arrives: Timer::from_seconds(seconds, TimerMode::Once),
                launched: 0,
                finish: None,
            });
        }
        attach(&mut flights[start..], finish);
    } else {
        land(
            commands,
            (deed, effects),
            (caster, source, false),
            (spell, strength),
            &victims,
            Some(&finish),
            party_at,
            world,
            flights,
        );
    }
}
