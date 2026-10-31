//! Weapon blows in a fight, swing by swing: a member's attack with the hand
//! due, and a monster's (retail `StartCharacterAttack`,
//! `ResolveCharacterAttack`, `AnnounceMonsterAttack`,
//! `ContinueMonsterAttack`).
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use super::attack::{self, Aim, Defence, Hand, Surprise};
use super::round::HandTurns;
use super::{
    Announcement, Armour, Battle, CombatNews, PartyPace, PartyView, RoundAction, RoundPhase,
};
use crate::content::characters::{Skill, Trait};
use crate::content::items::{AttackEffect, AttackMode};
use crate::content::species::{Reach, Species};
use crate::game::characters::{Character, CharacterStats, Condition, Vitals};
use crate::game::magic::effects::{self, Effect, Field, Victim};
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::party::body;
use crate::game::party::formation::{self, Row};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;
use crate::game::world::navigation::feet;
use crate::game::world::perception::party_distance;
use bevy::prelude::*;

mod member;
mod monster;
pub(super) use member::*;
pub(super) use monster::*;

/// What a blow needs of the fight besides the field it changes.
pub(super) struct Fight<'a, 'w, 's> {
    pub combatants: &'a [Entity],
    pub party: &'a PartyView,
    /// The party's armour as this round's blows meet it ([`super::party_armours`]).
    pub armours: Vec<(Entity, Armour)>,
    /// Monsters advancing this round, which drop their guard.
    pub moving: &'a Query<'w, 's, (), With<super::approach::CombatMove>>,
    pub points: &'a mut Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    /// Whom the acting monster may attack (retail `BuildMonsterActionQueue`'s
    /// `MonsterVsCharDisposition` and `MonsterHostility` tests); members
    /// act on their own orders.
    pub attacks: &'a dyn Fn(Victim) -> bool,
    /// The acting monster may hold back instead, as one more choice beside
    /// its attacks (a wary monster's wait).
    pub waits: bool,
}

impl Fight<'_, '_, '_> {
    fn armour(&self, member: Entity) -> Armour {
        self.armours.iter().find(|&&(m, _)| m == member).expect("a member's armour").1
    }
}

/// How an attack goes at its target: a shot from `from` (blinded, perhaps,
/// and sure to get through armour for some), or a swing among the
/// `bystanders` it may fumble onto at the `odds` it has.
enum Delivery {
    Shot { from: Vec3, blind: bool, through: bool },
    Swing { bystanders: Vec<Victim>, odds: i32 },
}

/// Where an attack aimed at `target` lands, if anywhere: a shot scatters
/// by its aim and lands where it strikes the target's body; a swing
/// reaches, misses, or fumbles onto a bystander, which spends its surprise.
/// An error when the target has no body to aim at.
fn land(
    rng: &mut GameRng,
    field: &Field,
    fight: &Fight,
    (target, aim): (Victim, i32),
    delivery: Delivery,
    surprise: &mut Surprise,
) -> Result<Option<Landing>, ()> {
    match delivery {
        Delivery::Shot { from, blind, through } => {
            let (centre, radius, half) = victim_body(field, fight, target).ok_or(())?;
            let shot = attack::scatter(rng, from, centre, aim, blind);
            Ok(attack::strikes(centre, shot, radius, half).then(|| Landing {
                victim: target,
                roll: rng.below(100) + 1,
                sure: true,
                through,
                fumbled: false,
            }))
        }
        Delivery::Swing { bystanders, odds } => {
            let fumble = attack::fumble_chance(bystanders.len(), odds);
            Ok(match attack::swing(rng, bystanders.len(), fumble) {
                Aim::Missed => None,
                Aim::Reaches { roll, sure } => {
                    Some(Landing { victim: target, roll, sure, through: sure, fumbled: false })
                }
                Aim::Fumbled { bystander, roll } => {
                    *surprise = Surprise::default();
                    Some(Landing {
                        victim: bystanders[bystander],
                        roll,
                        sure: true,
                        through: false,
                        fumbled: true,
                    })
                }
            })
        }
    }
}

/// Whoever a swing landed on, and how.
#[derive(Clone, Copy)]
struct Landing {
    victim: Victim,
    roll: u32,
    /// It struck whatever its aim: a roll of 5 or under, a fumble or a
    /// missile that flew true.
    sure: bool,
    /// It gets through armour whatever the roll.
    through: bool,
    fumbled: bool,
}

/// What one attack did to whom, reported once its swings are over.
#[derive(Default)]
struct Tally {
    hits: u32,
    damage: u32,
    /// Monsters killed by the party, with their experience.
    kills: Vec<(Entity, u32)>,
    deaths: Vec<Entity>,
    effects: Vec<Effect>,
    improved: Vec<(Entity, Skill)>,
}

impl Tally {
    fn struck(&mut self, struck: Option<Effect>, by_party: bool) -> u32 {
        let Some(Effect::Damaged { victim, outcome }) = struck else { return 0 };
        if outcome.died {
            match victim {
                Victim::Monster(monster) if by_party => {
                    self.kills.push((monster, outcome.experience));
                }
                Victim::Monster(e) | Victim::Member(e) => self.deaths.push(e),
            }
        }
        outcome.damage
    }

    fn report(
        self,
        battle: &mut Battle,
        (attacker, target): (Entity, Entity),
        news: &mut MessageWriter<CombatNews>,
    ) {
        // The summary, then what else the blows did, then the casualties
        // (retail `ReportCharacterAttackResult`).
        news.write(CombatNews::Result { attacker, target, hits: self.hits, damage: self.damage });
        let mut effects = self.effects;
        effects.retain(|e| !matches!(e, Effect::Damaged { .. }));
        if !effects.is_empty() {
            news.write(CombatNews::Effects(effects));
        }
        for (member, skill) in self.improved {
            news.write(CombatNews::Improved { member, skill });
        }
        for (monster, experience) in self.kills {
            battle.credit_kill(monster, experience, news);
        }
        for actor in self.deaths {
            news.write(CombatNews::Killed { actor });
        }
    }
}

/// A connecting close-range blow triggers the defender's charge even when
/// armour stopped its damage. It cannot recursively trigger another cloak.
fn razor_backlash(
    rng: &mut GameRng,
    field: &mut Field,
    fight: &Fight,
    battle: &Battle,
    defender: Victim,
    attacker: Victim,
    tally: &mut Tally,
    news: &mut MessageWriter<CombatNews>,
) {
    use crate::content::spells::SpellId;
    let charge = match defender {
        Victim::Member(member) => field
            .members
            .get_mut(member)
            .ok()
            .and_then(|(_, mut v, ..)| v.enchantments.spend_charge(SpellId::RAZOR_CLOAK)),
        Victim::Monster(monster) => {
            field.monsters.enchantments_mut(monster).spend_charge(SpellId::RAZOR_CLOAK)
        }
    };
    let Some(charge) = charge else { return };
    let rolled = effects::magnitude(
        rng,
        field.spells()[SpellId::RAZOR_CLOAK].dice,
        crate::game::magic::Strength { power: charge.power, boost: 0, against: 0 },
    );
    let amount = match attacker {
        Victim::Member(member) => {
            Some(member_defence(battle, field, fight, member).1.reduce(rolled as i32))
        }
        Victim::Monster(monster) => {
            monster_defence(field, fight, monster).map(|defence| defence.reduce(rolled as i32))
        }
    };
    let Some(amount) = amount else { return };
    let result = effects::hurt(rng, field, attacker, amount);
    let damage = tally.struck(result, matches!(defender, Victim::Member(_)));
    news.write(CombatNews::Result {
        attacker: defender.entity(),
        target: attacker.entity(),
        hits: 1,
        damage,
    });
}

/// Where a monster is struck: the middle of its body.
fn body_centre(feet: Vec3, monster: &crate::game::world::population::Monster) -> Vec3 {
    feet + Vec3::Y * monster.body.eye_height * 0.5
}

/// Whether a monster has its back to the party: facing away by 135° or
/// more (retail `IsPartyLookingAwayFrom`).
fn back_to_party(forward: Vec3, at: Vec3, party_eye: Vec3) -> bool {
    let to_party = (party_eye - at).xz();
    let forward = forward.xz();
    to_party != Vec2::ZERO
        && forward != Vec2::ZERO
        && forward.angle_to(to_party).abs() >= std::f32::consts::FRAC_PI_4 * 3.0
}

/// How a member stands this round, for the blows aimed at them.
fn member_defence(
    battle: &Battle,
    field: &Field,
    fight: &Fight,
    member: Entity,
) -> (Character, Defence<'static>) {
    let (character, vitals, ..) = field.members.get(member).unwrap();
    let armour = fight.armour(member);
    let running = battle.this_round.pace == Some(PartyPace::Run);
    let defending = battle
        .this_round
        .party
        .iter()
        .any(|p| p.actor == member && matches!(p.action, RoundAction::Defend));
    let stats = character.stats(&vitals.enchantments);
    (character.clone(), Defence::member(&stats, armour, running, defending))
}

fn monster_defence<'a>(
    field: &Field<'a, '_>,
    fight: &Fight,
    monster: Entity,
) -> Option<Defence<'a>> {
    let species = field.species();
    let (body, ..) = field.monsters.monsters().get(monster).ok()?;
    if body.is_dead() || !field.monsters.active(monster) {
        return None;
    }
    let kind = &species[body.species];
    Some(Defence::monster(kind, body, fight.moving.contains(monster)))
}

/// Both sorts of actor use the same defence and body rules, whether aimed
/// at deliberately or hit by a fumble.
fn victim_defence<'a>(
    battle: &Battle,
    field: &Field<'a, '_>,
    fight: &Fight,
    victim: Victim,
) -> Option<(Defence<'a>, Option<(Entity, Character)>)> {
    match victim {
        Victim::Monster(entity) => Some((monster_defence(field, fight, entity)?, None)),
        Victim::Member(entity) => {
            let (profile, defence) = member_defence(battle, field, fight, entity);
            Some((defence, Some((entity, profile))))
        }
    }
}

fn victim_body(field: &Field, fight: &Fight, victim: Victim) -> Option<(Vec3, f32, f32)> {
    match victim {
        Victim::Member(entity) => {
            if !field.members.get(entity).is_ok_and(|(_, vitals, ..)| !vitals.is_dead()) {
                return None;
            }
            Some((body::PartyBody::centre(fight.party.eye), body::RADIUS, body::RADIUS))
        }
        Victim::Monster(entity) => {
            let (monster, at, walker, ..) = field.monsters.monsters().get(entity).unwrap();
            Some((
                body_centre(feet(at.translation(), walker), monster),
                monster.body.radius,
                monster.body.eye_height * 0.5,
            ))
        }
    }
}

/// A guard may take a blow that lands on its ward (retail
/// `ResolveGuardianInterception`): each guard of the victim still able
/// (alive, not blinded or worse, guarding it within its first attack's
/// reach, and under its attacks a round in blows taken) rolls its
/// protection against the attacker's, raised for an unaware or natural
/// strike, within 5 and 95 percent; one that succeeds, picked at random,
/// takes the blow, tiring by 2 to 4.
fn intercept(
    battle: &mut Battle,
    rng: &mut GameRng,
    field: &mut Field,
    (attacker, surprise): (i32, &mut Surprise),
    victim: Victim,
    news: &mut MessageWriter<CombatNews>,
) -> Victim {
    let Victim::Monster(ward) = victim else { return victim };
    let species = field.species();
    let monsters = field.monsters.monsters();
    let able: Vec<(usize, i32)> = battle
        .this_round
        .guards
        .iter()
        .enumerate()
        .filter(|(_, g)| g.ward == ward)
        .filter_map(|(i, g)| {
            let (body, at, walker, ..) = monsters.get(g.guard).ok()?;
            let (ward_body, ward_at, ward_walker, ..) = monsters.get(ward).ok()?;
            let kind = &species[body.species];
            let first = kind.attacks.first()?;
            let gap = body.gap(
                feet(at.translation(), walker),
                ward_body,
                feet(ward_at.translation(), ward_walker),
            );
            (body.can_guard()
                && !ward_body.is_dead()
                && gap <= first.reach.metres()
                && g.interceptions < u32::from(kind.attacks_per_round))
            .then(|| (i, attack::monster_protection(kind, body)))
        })
        .collect();
    let pressing = attacker + 10 * i32::from(surprise.unaware) + 10 * i32::from(surprise.natural);
    let mut stepping = Vec::new();
    for (i, guarding) in able {
        let chance = (guarding - pressing + 50).clamp(5, 95) as u32;
        if rng.below(100) < chance {
            stepping.push(i);
        }
    }
    if stepping.is_empty() {
        return victim;
    }
    let chosen = stepping[rng.below(stepping.len() as u32) as usize];
    let guard = &mut battle.this_round.guards[chosen];
    guard.interceptions += 1;
    let guard = guard.guard;
    let tired = rng.below(3) + 2;
    field.monsters.fatigue(guard, tired);
    *surprise = Surprise::default();
    news.write(CombatNews::Intercepts { guard, ward });
    Victim::Monster(guard)
}

/// A member struck practises Iron Skin as their reduction is worked out
/// (retail `ApplyCharacterDamageReduction`).
fn iron_skin(
    rng: &mut GameRng,
    field: &mut Field,
    member: Entity,
    character: &Character,
    tally: &mut Tally,
) {
    if character.skill_available(Skill::IronSkin)
        && let Ok((mut c, ..)) = field.members.get_mut(member)
        && c.practise(Skill::IronSkin, 1, rng)
    {
        tally.improved.push((member, Skill::IronSkin));
    }
}
