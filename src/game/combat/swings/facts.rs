//! Reading the fight for the rules of [`crate::game::combat::attack`]: what each
//! takes is a small struct of facts, built here from one view of an actor
//! ([`BodyReadOnlyItem`]) whichever of member and monster it is, and from where
//! everyone stands. Nothing here decides anything.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use crate::content::characters::{Attribute, Skill, Trait};
use crate::content::items::AttackMode;
use crate::content::species::MonsterKind;
use crate::game::actors::{Arm, Places, reaches};
use crate::game::characters::{Condition, Load};
use crate::game::combat::approach::CombatMove;
use crate::game::combat::attack::{self, Surprise};
use crate::game::combat::monster::close::{self, MoveBudget, Moving};
use crate::game::combat::turn::{Guards, Interceptions, Ordered, Spotted, Turn};
use crate::game::difficulty::Difficulty;
use crate::game::hits::effects::{Body, BodyReadOnlyItem};
use crate::game::items::{Items, Missile, Weapon};
use crate::game::magic::Strength;
use crate::game::npcs::NpcBody;
use crate::game::party::formation::Formation;
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility;
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// How those in the fight stand as blows come at them: who is an NPC's body,
/// who is walking, and what each member was ordered to do.
#[derive(QueryData)]
pub struct Bearing {
    npc: Has<NpcBody>,
    walking: Has<CombatMove>,
    ordered: Option<&'static Ordered>,
}

/// What is the same all through the attack.
#[derive(Clone, Copy)]
pub struct Setting<'a> {
    pub combatants: &'a [Entity],
    pub difficulty: Difficulty,
    /// The party is running.
    pub running: bool,
}

/// What is looked at to learn the facts the rules take.
pub struct Scene<'a, 'w, 's> {
    bodies: &'a Query<'w, 's, Body>,
    places: &'a Places<'w, 's>,
    bearing: &'a Query<'w, 's, Bearing>,
    formation: &'a Formation,
    setting: Setting<'a>,
}

/// The attack under way: who strikes whom, with which arm and how.
pub struct Attack<'a> {
    pub actor: Entity,
    pub target: Entity,
    pub arm: &'a Arm,
    pub mode: AttackMode,
    /// A Berserk order's fury, which costs twice the stamina.
    pub berserk: bool,
    /// What its weapon effects are saved against.
    pub strength: Strength,
}

/// The attacker, as one swing finds it.
pub struct Striker {
    pub able: bool,
    pub fatigue: i32,
    pub agility: i32,
    pub scaling: attack::Scaling,
    pub blind: bool,
    pub fights_blind: Option<u8>,
    pub press: attack::Press,
    pub backstab: i32,
    pub reduction: i32,
    pub training: Option<attack::Training>,
    pub thrown_criticals: bool,
    pub diseased: bool,
    pub slowed: bool,
    pub hasted: bool,
    pub load: Option<Load>,
}

/// The one struck, as one swing finds it.
pub struct Victim {
    pub alive: bool,
    pub defence: attack::Defence,
    pub agility: i32,
    pub health: u32,
    pub kind: Option<MonsterKind>,
}

/// Where a swing reached: its roll, whether it lands and gets through armour
/// without penetrating, and whether it went astray.
pub struct Reached {
    pub roll: u32,
    pub lands: bool,
    pub through: bool,
    pub fumbled: bool,
}

impl<'a, 'w, 's> Scene<'a, 'w, 's> {
    pub fn new(
        bodies: &'a Query<'w, 's, Body>,
        places: &'a Places<'w, 's>,
        bearing: &'a Query<'w, 's, Bearing>,
        formation: &'a Formation,
        setting: Setting<'a>,
    ) -> Self {
        Self { bodies, places, bearing, formation, setting }
    }

    fn defence(&self, entity: Entity, body: &BodyReadOnlyItem) -> attack::Defence {
        let (walking, ordered) = self
            .bearing
            .get(entity)
            .map_or((false, None), |bearing| (bearing.walking, bearing.ordered));
        attack::Defence {
            armour: *body.armour,
            fatigue: attack::fatigue(body.vitals),
            defending: ordered.is_some_and(Ordered::defends),
            moving: if body.member { self.setting.running } else { walking },
        }
    }

    fn press(&self, body: &BodyReadOnlyItem) -> attack::Press {
        attack::Press {
            press: body.attacks.press,
            fatigue: attack::fatigue(body.vitals),
            tires_little: body.attacks.tires_little,
        }
    }

    pub fn striker(&self, on: &Attack) -> Option<Striker> {
        let own = self.bodies.get(on.actor).ok()?;
        let character = own.character;
        Some(Striker {
            able: own.vitals.can_fight(),
            fatigue: attack::fatigue(own.vitals),
            agility: own.attributes.agility(),
            scaling: attack::Scaling { difficulty: self.setting.difficulty, side: own.stance() },
            blind: own.vitals.conditions.has(Condition::Blind),
            fights_blind: own.attacks.blind_fighting,
            press: self.press(&own),
            backstab: own.attacks.backstab,
            reduction: own.armour.reduction,
            training: character.map(|character| attack::Training {
                critical: on.arm.critical_skill.map(|skill| character.skill(skill)),
                knock_out: character
                    .has_trait(Trait::Knockout)
                    .then(|| character.profession_levels[character.profession]),
            }),
            thrown_criticals: character.is_some_and(|c| c.has_trait(Trait::ThrownCriticals)),
            diseased: own.vitals.conditions.has(Condition::Diseased),
            slowed: own.vitals.conditions.has(Condition::Slowed),
            hasted: own.attacks.hasted > 0.0,
            load: own.burden.map(|burden| burden.load),
        })
    }

    pub fn victim(&self, entity: Entity) -> Option<Victim> {
        let body = self.bodies.get(entity).ok()?;
        Some(Victim {
            alive: !body.vitals.is_dead(),
            defence: self.defence(entity, &body),
            agility: body.attributes.agility(),
            health: body.vitals.health.max,
            kind: body.resistances.kind,
        })
    }

    /// The living in reach of the arm, other than the target: the attacker
    /// itself, friends and foes alike. A shot meets nobody on the way.
    pub fn bystanders(&self, on: &Attack) -> Vec<Entity> {
        if attack::is_missile(on.arm.reach) {
            return Vec::new();
        }
        let from = self.places.of(on.actor);
        self.setting
            .combatants
            .iter()
            .copied()
            .filter(|&other| other != on.target)
            .filter(|&other| {
                self.bodies.get(other).is_ok_and(|there| {
                    !there.vitals.is_dead()
                        && (other == on.actor
                            || reaches(
                                self.formation,
                                (on.actor, &from),
                                (other, &self.places.of(other)),
                                on.arm.reach,
                            ))
                })
            })
            .collect()
    }

    /// A swing's fate: its chance, and the line of flight of a shot.
    pub fn reaching(
        &self,
        on: &Attack,
        chance: attack::Chance,
        bystanders: usize,
        me: &Striker,
    ) -> attack::Reaching {
        let (from, at) = (self.places.of(on.actor), self.places.of(on.target));
        attack::Reaching {
            reach: on.arm.reach,
            chance,
            bystanders,
            sure_throw: on.arm.sure_throw,
            flight: attack::Flight {
                from: from.eye,
                at: at.centre,
                radius: at.radius,
                half_height: at.half_height,
                blind: me.blind,
            },
        }
    }

    /// The guards of a ward, and the facts of each.
    pub fn guards_of(
        &self,
        ward: Entity,
        guards: &Query<&Guards>,
        interceptions: &Query<&mut Interceptions>,
    ) -> (Vec<Entity>, Vec<attack::Guard>) {
        let at = self.places.of(ward);
        guards
            .get(ward)
            .into_iter()
            .flat_map(Guards::entities)
            .filter_map(|&guard| {
                let (body, taken) = (self.bodies.get(guard).ok()?, interceptions.get(guard).ok()?);
                Some((
                    guard,
                    attack::Guard {
                        able: body.vitals.can_guard(),
                        reach: body.attacks.arms.first().map(|arm| arm.reach),
                        gap: self.places.of(guard).gap(&at),
                        taken: taken.taken,
                        allowed: taken.allowed,
                        press: self.press(&body),
                    },
                ))
            })
            .unzip()
    }

    /// Whether the target can be caught unaware: which side the attacker is
    /// on, which way the target faces, and whether it may notice.
    pub fn unaware(
        &self,
        on: &Attack,
        watchful: &Query<(&Ordered, &mut Spotted)>,
    ) -> attack::Unaware {
        let watch = self.bodies.get(on.target).ok().and_then(|their| {
            let (ordered, spotted) =
                watchful.get(on.target).ok().filter(|_| their.character.is_some())?;
            Some(attack::Watch {
                defending: ordered.defends(),
                alive: !their.vitals.is_dead(),
                stamina: their.vitals.stamina.current,
                worst: their.vitals.conditions.worst(),
                blind: their.vitals.conditions.has(Condition::Blind),
                senses: their.attributes[Attribute::Senses],
                tries: spotted.0,
            })
        });
        attack::Unaware {
            reach: on.arm.reach,
            attacker_on: self.places.of(on.target).quadrant(self.places.of(on.actor).feet),
            facing: self.formation.facing(on.target),
            ambushes: on.arm.ambushes,
            watch,
        }
    }

    /// The bodies the actor may strike or close in on, as it sees them: every
    /// living body in the fight but itself, and the actor's arms.
    pub fn look_about(
        &self,
        actor: Entity,
        own: &BodyReadOnlyItem,
        (roster, obstacles): (&PartyRoster, &WorldObstacles),
    ) -> (Vec<attack::ArmFacts>, Vec<attack::Foe>, Vec<Entity>) {
        let at = self.places.of(actor);
        let arms = own
            .attacks
            .arms
            .iter()
            .map(|arm| attack::ArmFacts { slot: arm.slot, modes: arm.modes })
            .collect();
        let (mut foes, mut bodies) = (Vec::new(), Vec::new());
        for &other in self.setting.combatants.iter().filter(|&&other| other != actor) {
            let Ok(their) = self.bodies.get(other) else { continue };
            if their.vitals.is_dead() {
                continue;
            }
            let there = self.places.of(other);
            foes.push(attack::Foe {
                stance: hostility::between(own.standing(), their.standing()),
                npc: self.bearing.get(other).is_ok_and(|bearing| bearing.npc),
                reached_by: own
                    .attacks
                    .arms
                    .iter()
                    .map(|arm| reaches(self.formation, (actor, &at), (other, &there), arm.reach))
                    .collect(),
                seen: obstacles.clear(at.eye, there.centre),
                stealth: roster
                    .contains(&other)
                    .then(|| their.character.map_or(0, |c| c.skill(Skill::Stealth))),
                worst: their.vitals.conditions.worst(),
                armed: !their.attacks.arms.is_empty(),
                side: at.quadrant(there.feet),
                gap: at.gap(&there),
                distance_squared: at.feet.distance_squared(there.feet),
            });
            bodies.push(other);
        }
        (arms, foes, bodies)
    }

    /// What a body that can walk could do with nobody to strike.
    pub fn closing(
        &self,
        own: &BodyReadOnlyItem,
        walker: Option<&Walker>,
        turn: &Turn,
        now: u32,
        roster: &PartyRoster,
    ) -> Option<attack::Closing> {
        let (_, walker) = own.attacks.stride.zip(walker).filter(|_| !turn.moved)?;
        let hurt = close::charges(own.vitals.health.current, own.vitals.health.max);
        let at = self.places.of(own.entity);
        let party = roster.iter().find_map(|member| self.bodies.get(member).ok());
        let moving = Moving {
            walks: !own.vitals.is_dead() && walker.speed > 0.0,
            reach: own.attacks.reach(),
            stride: own.attacks.stride.unwrap_or(0.0),
            hasted: own.attacks.hasted,
            slowed: own.vitals.conditions.has(Condition::Slowed),
        };
        Some(attack::Closing {
            stance: own.stance(),
            toward_party: party.and_then(|party| {
                close::budget(&moving, hurt, at.gap(&self.places.of(party.entity)))
            }),
            toward_foes: (walker.speed > 0.0)
                .then(|| MoveBudget { metres: close::metres(&moving, hurt), charge: hurt }),
            now,
        })
    }
}

impl Attack<'_> {
    /// The roll to hit against the target.
    pub fn hitting(&self, surprise: Surprise, me: &Striker, aimed: &Victim) -> attack::Hitting {
        attack::Hitting {
            rating: self.arm.rating,
            mode: self.mode,
            fatigue: me.fatigue,
            modern: self.arm.modern,
            blind: me.blind,
            fights_blind: me.fights_blind,
            surprise,
            agility: me.agility,
            target_agility: aimed.agility,
            target: aimed.defence,
            scaling: me.scaling,
        }
    }

    /// Getting through the armour of the one struck.
    pub fn piercing(
        &self,
        surprise: Surprise,
        me: &Striker,
        struck: &Victim,
        (place, by): (attack::Location, AttackMode),
        reached: &Reached,
    ) -> attack::Piercing {
        attack::Piercing {
            rating: self.arm.rating,
            swing: self.mode,
            by,
            fatigue: me.fatigue,
            fatigue_pierces: self.arm.fatigue_pierces,
            victim: struck.defence,
            surprise,
            place,
            agility: me.agility,
            victim_agility: struck.agility,
            scaling: me.scaling,
            roll: reached.roll,
            sure: reached.through,
        }
    }

    /// The damage of a blow that got through.
    pub fn blowing(
        &self,
        surprise: Surprise,
        me: &Striker,
        struck: &Victim,
        fumbled: bool,
    ) -> attack::Blowing<'_> {
        attack::Blowing {
            dice: self.arm.rolls(self.mode),
            percent: self.arm.percent,
            mode: self.mode,
            surprise,
            backstab: me.backstab,
            moving: struck.defence.moving,
            defending: struck.defence.defending,
            slays: &self.arm.slays,
            kind: struck.kind,
            reduction: struck.defence.armour.reduction,
            fumbled,
            victim_health: struck.health,
            effects: self.arm.effects,
            training: me.training,
        }
    }

    /// What the swing costs.
    pub fn cost(&self, me: &Striker) -> attack::Cost {
        attack::Cost {
            tires: self.arm.tires,
            mode: self.mode,
            weight: self.arm.weight,
            berserk: self.berserk,
            diseased: me.diseased,
            load: me.load,
            slowed: me.slowed,
            hasted: me.hasted,
        }
    }

    /// What a missile weapon shoots or throws: the item, and what using it up
    /// weighs after `swings` swings.
    pub fn ammunition(
        &self,
        swings: usize,
        held: &Items,
        strikers: &Query<(Option<&Weapon>, Option<&Missile>)>,
    ) -> (Option<Entity>, attack::Ammunition) {
        let arm = self.arm;
        let slot = if arm.loaded.is_some() { arm.slot.other() } else { arm.slot };
        let item =
            held.worn(self.actor, slot.equip()).filter(|_| arm.ranged && arm.weapon.is_some());
        let facts = attack::Ammunition {
            ranged: item.is_some(),
            launcher: arm.loaded.is_some(),
            kept: item.is_some_and(|item| {
                strikers.get(item).is_ok_and(|(weapon, missile)| {
                    weapon.is_some_and(|weapon| weapon.casts)
                        || missile.is_some_and(|missile| missile.kept)
                })
            }),
            left: item.and_then(|item| held.charges(item).or(held.count(item))),
            spent: swings,
        };
        (item, facts)
    }
}
