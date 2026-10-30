//! Finalization is accepted work on the last-arriving missile of a cast.
//! It runs after every original hit, once, and retains the original aim.
use super::{
    LearnedSpells, SpellPoints, Strength,
    casting::{self, Aim, SpellInFlight},
    effects::{self, Effect, Field, Victim},
    targeting::{self, Recipients},
};
use crate::{
    content::spells::{SpellCatalog, SpellDefinition, SpellId, Targeting},
    game::{rng::GameRng, world::hostility::Stance},
};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug)]
pub enum Origin {
    Actor(Victim),
    Point { at: Vec3, owner: Option<Victim> },
}
impl Origin {
    pub fn actor(self) -> Option<Victim> {
        match self {
            Self::Actor(actor) => Some(actor),
            Self::Point { .. } => None,
        }
    }
    fn owner(self) -> Option<Victim> {
        match self {
            Self::Actor(actor) => Some(actor),
            Self::Point { owner, .. } => owner,
        }
    }
    pub fn position(self, world: &Field, party: Vec3) -> Vec3 {
        match self {
            Self::Point { at, .. } => at,
            Self::Actor(Victim::Member(_)) => party,
            Self::Actor(Victim::Monster(entity)) => {
                world.monsters.monsters().get(entity).map_or(party, |(body, at, walker, ..)| {
                    body.eye(crate::game::world::navigation::feet(at.translation(), walker))
                })
            }
        }
    }
    fn ground(self, world: &Field, party: Vec3) -> Vec3 {
        match self {
            Self::Point { at, .. } => at,
            Self::Actor(Victim::Member(_)) => {
                crate::game::party::body::PartyBody::feet_below(party)
            }
            Self::Actor(Victim::Monster(entity)) => {
                world.monsters.monsters().get(entity).map_or(party, |(_, at, walker, ..)| {
                    crate::game::world::navigation::feet(at.translation(), walker)
                })
            }
        }
    }
    pub(crate) fn recipients(self, world: &Field, reversed: bool) -> Recipients {
        let side = match self.owner() {
            Some(Victim::Member(entity)) => {
                if world.members.get(entity).is_ok_and(|(_, v, ..)| {
                    v.conditions.has(crate::game::characters::Condition::Turncoat)
                }) {
                    Stance::Hostile
                } else {
                    Stance::Friendly
                }
            }
            Some(Victim::Monster(entity)) => world.monsters.stance(entity, world.now),
            None => return Recipients::Everyone,
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
        match self.owner() {
            Some(Victim::Member(entity)) => world.members.get(entity).map_or(0, |(profile, ..)| {
                super::total_caster_level(profile, spell.books).max(0) as u32
            }),
            Some(Victim::Monster(entity)) => world
                .monsters
                .monsters()
                .get(entity)
                .ok()
                .map(|(body, ..)| &world.species[body.species])
                .map_or(0, |kind| u32::from(kind.level)),
            None => 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Finalization {
    pub aim: Aim,
    pub victims: Vec<Victim>,
    pub reflected: bool,
}

pub(crate) fn attach(flights: &mut [SpellInFlight], finish: Finalization) {
    if let Some((index, _)) =
        flights.iter().enumerate().max_by_key(|(index, flight)| (flight.arrives.duration(), *index))
    {
        flights[index].finish = Some(finish);
    }
}

pub(crate) fn resolve(
    caster: Entity,
    origin: Origin,
    backfired: bool,
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    finish: Option<&Finalization>,
    party_at: Vec3,
    spells: &SpellCatalog,
    rng: &mut GameRng,
    world: &mut Field,
    points: &mut Query<(&LearnedSpells, &mut SpellPoints)>,
    flights: &mut Vec<SpellInFlight>,
) -> Vec<Effect> {
    let mut landed =
        effects::resolve(origin.actor(), backfired, spell, strength, victims, rng, world, points);
    let Some(finish) = finish else { return landed };
    // The strongest surviving charge pays once, after all original impacts.
    if !backfired && !finish.reflected && spell.aimed_at_enemies() {
        let mut order = finish.victims.clone();
        order.sort_by_key(|victim| match *victim {
            Victim::Member(entity) => {
                (0, world.party.iter().position(|&member| member == entity).unwrap_or(usize::MAX))
            }
            Victim::Monster(entity) => (
                1,
                world
                    .monsters
                    .monsters()
                    .get(entity)
                    .map_or(usize::MAX, |(body, ..)| body.location as usize),
            ),
        });
        let mut best = None;
        for victim in order {
            let effect = match victim {
                Victim::Member(entity) => world
                    .members
                    .get(entity)
                    .ok()
                    .filter(|(_, v, ..)| !v.is_dead())
                    .and_then(|(_, v, ..)| v.enchantments.get(SpellId::EYE_FOR_AN_EYE))
                    .copied(),
                Victim::Monster(entity) => world
                    .monsters
                    .monsters()
                    .get(entity)
                    .ok()
                    .filter(|(body, ..)| !body.is_dead())
                    .and_then(|(body, ..)| body.enchantments.get(SpellId::EYE_FOR_AN_EYE))
                    .copied(),
            };
            if let Some(effect) = effect
                && effect.power > best.map_or(0, |(_, power)| power)
            {
                best = Some((victim, effect.power));
            }
        }
        if let Some((charged, power)) = best {
            match charged {
                Victim::Member(entity) => {
                    world
                        .members
                        .get_mut(entity)
                        .unwrap()
                        .1
                        .enchantments
                        .spend_charge(SpellId::EYE_FOR_AN_EYE);
                }
                Victim::Monster(entity) => {
                    world
                        .monsters
                        .enchantments_mut(entity)
                        .unwrap()
                        .spend_charge(SpellId::EYE_FOR_AN_EYE);
                }
            }
            landed.push(Effect::Reflected { victim: charged });
            let original = origin.owner();
            let old_position = origin.position(world, party_at);
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
                    Aim::Monster(entity) => Origin::Actor(Victim::Monster(
                        if spell.targeting == Targeting::EnemyGroup {
                            world.monsters.leader(entity)
                        } else {
                            entity
                        },
                    )),
                    _ => Origin::Actor(Victim::Member(
                        world
                            .party
                            .iter()
                            .copied()
                            .find(|&actor| {
                                world.members.get(actor).is_ok_and(|(_, v, ..)| v.can_fight())
                            })
                            .unwrap_or(caster),
                    )),
                },
            };
            let victims = match spell.targeting {
                Targeting::Enemy => original.into_iter().collect(),
                Targeting::EnemyGroup => match original {
                    Some(Victim::Member(_)) => world
                        .party
                        .iter()
                        .copied()
                        .filter(|&entity| {
                            world.members.get(entity).is_ok_and(|(_, v, ..)| !v.is_dead())
                        })
                        .map(Victim::Member)
                        .collect(),
                    Some(Victim::Monster(entity)) => {
                        world.monsters.group(entity).into_iter().map(Victim::Monster).collect()
                    }
                    None => vec![],
                },
                Targeting::Radius => targeting::radius(
                    world,
                    origin.ground(world, party_at),
                    old_position,
                    spell.radius + spell.radius_per_power * f32::from(strength.power.min(power)),
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
            let strength = Strength::at(
                source.figure(world, spell),
                spell,
                strength.power.min(power),
                strength.boost,
            );
            let aim = original.map_or(Aim::Point(old_position), |victim| match victim {
                Victim::Member(entity) => Aim::Member(entity),
                Victim::Monster(entity) => Aim::Monster(entity),
            });
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
                landed.extend(resolve(
                    caster,
                    source,
                    false,
                    spell,
                    strength,
                    &victims,
                    Some(&finish),
                    party_at,
                    spells,
                    rng,
                    world,
                    points,
                    flights,
                ));
            }
        }
    }
    if spell.id == SpellId::BOILING_BLOOD
        && let Some(explosion) = spells.get(SpellId::BOILING_BLOOD_EXPLOSION)
    {
        for &victim in &finish.victims {
            let dead = match victim {
                Victim::Member(entity) => {
                    world.members.get(entity).is_ok_and(|(_, v, ..)| v.is_dead())
                }
                Victim::Monster(entity) => {
                    world.monsters.monsters().get(entity).is_ok_and(|(body, ..)| body.is_dead())
                }
            };
            if !dead {
                continue;
            }
            let centre = Origin::Actor(victim).ground(world, party_at);
            let strength = Strength::at(
                origin.figure(world, explosion),
                explosion,
                strength.power,
                strength.boost,
            );
            let victims = targeting::radius(
                world,
                centre,
                centre,
                explosion.radius + explosion.radius_per_power * f32::from(strength.power),
                Some(party_at),
                origin.recipients(world, backfired),
            );
            let finish = Finalization {
                aim: Aim::Point(centre),
                victims: victims.clone(),
                reflected: finish.reflected,
            };
            landed.extend(resolve(
                caster,
                origin,
                false,
                explosion,
                strength,
                &victims,
                Some(&finish),
                party_at,
                spells,
                rng,
                world,
                points,
                flights,
            ));
        }
    }
    landed
}
