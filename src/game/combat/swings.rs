//! Weapon attacks in a fight, swing by swing (retail `StartCharacterAttack`,
//! `ResolveCharacterAttack`, `StartMonsterAttack`, `ContinueMonsterAttack`).
//!
//! An actor that fights ([`Chosen::Fight`]) strikes with one of its arms: a
//! member's weapon slot or a monster's natural attack, judged alike on
//! every body ([`Arms`]). One rule opens the attack ([`open`]): a body
//! under orders strikes the most afflicted foe its arm reaches, one
//! deciding for itself picks at random among what it reaches and sees. Its
//! swings then follow at once ([`swing`]), each blow landing and settling
//! before the next is aimed, and the attack is told of as a whole.
//!
//! Retail's voices on a miss, a kill or a fumble draw from the same random
//! numbers as the rules; here they draw nothing.

use bevy::prelude::*;

use super::approach::{self, CombatMove};
use super::attack::{self, Surprise};
use super::turn::{Chosen, Fights, Guards, Interceptions, Ordered, Perform, Spotted, Turn};
use super::{Announcement, Battle, CombatNews, Combatants, Pause, Round, RoundPhase};
use crate::content::characters::{Attribute, BodyPart, Skill, Trait};
use crate::content::items::{AttackEffect, AttackEffects, AttackMode, Dice, EquipClass};
use crate::content::species::Reach;
use crate::content::spells::SpellId;
use crate::game::actors::{Arm, Attacks, Laying, Places, Stance as Standing, reaches};
use crate::game::characters::{Condition, Practise, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::{Blow, Definitions};
use crate::game::hits::effects::{self, Body, Log, Rules, Strike};
use crate::game::hits::{Damaged, Effect};
use crate::game::inventory::WeaponSlot;
use crate::game::magic::Strength;
use crate::game::npcs::NpcBody;
use crate::game::party::formation::{Formation, Row};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Provocation, Stance};
use crate::game::world::navigation::Walker;
use crate::game::world::obstacles::WorldObstacles;

/// An attack under way: who strikes whom, with which arm and how, and what
/// its blows have done so far.
struct Attack {
    by: Entity,
    target: Entity,
    arm: Arm,
    mode: AttackMode,
    /// The dice a blow rolls.
    dice: Dice,
    surprise: Surprise,
    /// What its weapon effects are saved against.
    strength: Strength,
    /// A berserk fury, which costs twice the stamina.
    berserk: bool,
    /// The hits that landed, damage or not.
    hits: u32,
    damage: u32,
    /// What its blows did besides damage.
    effects: Vec<Effect>,
}

/// How those in the fight stand as blows come at them: who is an NPC's
/// body, who is walking, and what each member was ordered to do.
type Bearing<'w, 's> = Query<'w, 's, (Has<NpcBody>, Has<CombatMove>, Option<&'static Ordered>)>;

/// How an actor stands as a blow comes at it: braced when it defends, on
/// the move while it walks or, for a member, while the party runs.
fn standing<'a>(
    actor: Entity,
    (member, vitals): (bool, &'a Vitals),
    bearing: &Bearing,
    running: bool,
) -> Standing<'a> {
    let (_, walking, ordered) = bearing.get(actor).unwrap_or((false, false, None));
    Standing {
        vitals: Some(vitals),
        braced: ordered.is_some_and(Ordered::defends),
        distracted: if member { running } else { walking },
    }
}

/// The step of the round being played.
fn step(round: &Round) -> u32 {
    match round.phase {
        RoundPhase::Executing(step) => step,
        _ => 1,
    }
}

/// What an actor with nobody to strike does instead.
enum Otherwise {
    /// Close in on a foe and act again later in the round.
    Advance(approach::MoveBudget, approach::MoveGoal),
    /// Hold back warily, resting.
    Wait,
    /// Nothing: done for the round.
    Stand,
}

/// Open the acting body's attack (retail `StartCharacterAttack`,
/// `StartMonsterAttack`): with the slot due, at someone it may attack and
/// reaches, by one of the slot's arms and modes. One under orders strikes
/// the most afflicted foe, then one "in reach" of the party, then the
/// nearest; ties keep the fight's order. One deciding for itself picks at
/// random among what it reaches and sees, a wary one's wait among them
/// (retail `ChooseMonsterTarget`, `BuildMonsterActionQueue`); a member's
/// Stealth may keep them out of its eye this time, and one that stayed
/// out of it may learn from it. A slot that cannot strike (out of shots,
/// or nobody in reach) gives up its attacks and the other takes over. With
/// nobody to strike, a body that walks closes in on a foe; anyone else is
/// done for the round.
#[allow(clippy::type_complexity)]
fn open(
    actor: Entity,
    fights: Fights,
    (turn, ordered, walker): (&mut Turn, bool, Option<&Walker>),
    (bodies, places): (&mut Query<Body>, &Places),
    bearing: &Bearing,
    (formation, roster, combatants): (&Formation, &PartyRoster, &[Entity]),
    obstacles: &WorldObstacles,
    rng: &mut GameRng,
    commands: &mut Commands,
) -> Result<(Attack, u32), Otherwise> {
    let own = bodies.get(actor).map_err(|_| Otherwise::Stand)?;
    let attacks = own.attacks.clone();
    let (at, mine, level) = (places.of(own.entity), own.standing(), own.level.0);
    let slowed = own.vitals.conditions.has(Condition::Slowed);
    let wanted = if turn.berserk { Stance::Friendly } else { Stance::Hostile };
    // Those a wild attacker's eye misses this time.
    let unseen: Vec<Entity> = if ordered {
        Vec::new()
    } else {
        roster
            .iter()
            .filter(|&member| {
                let stealth = bodies
                    .get(member)
                    .ok()
                    .and_then(|body| body.character)
                    .map_or(0, |c| c.skill(Skill::Stealth));
                rng.below(100) < stealth * 75 / 100
            })
            .collect()
    };
    let foe = |other: Entity| {
        bodies.get(other).ok().filter(|their| {
            other != actor
                && !their.vitals.is_dead()
                && hostility::between(mine, their.standing()) == wanted
        })
    };
    let mut evaded = Vec::new();
    let mut chosen = None;
    for slot in [turn.due, turn.due.other()] {
        if turn.attacks[slot] == 0 {
            continue;
        }
        // Whom each of the slot's arms reaches on the side it fights.
        let mut candidates: Vec<(&Arm, AttackMode, Entity)> = Vec::new();
        for arm in attacks.of(slot) {
            for &other in combatants {
                let Some(their) = foe(other) else { continue };
                let npc = bearing.get(other).is_ok_and(|(npc, ..)| npc);
                if (ordered && npc)
                    || !reaches(
                        formation,
                        (actor, &at),
                        (other, &places.of(their.entity)),
                        arm.reach,
                    )
                    || (!ordered && !obstacles.clear(at.eye, places.of(their.entity).centre))
                {
                    continue;
                }
                if unseen.contains(&other) {
                    if !evaded.contains(&other) {
                        evaded.push(other);
                    }
                    continue;
                }
                candidates.extend(arm.modes.iter().map(|mode| (arm, mode, other)));
            }
        }
        if !candidates.is_empty() {
            chosen = Some((slot, candidates));
            break;
        }
    }
    let Some((slot, candidates)) = chosen else {
        // A body that walks closes in on a foe, hurt ones charging.
        let stride = attacks.stride.zip(walker).filter(|_| !turn.moved);
        let goal = stride.and_then(|(_, walker)| match mine.0 {
            Stance::Friendly => combatants
                .iter()
                .filter_map(|&other| {
                    Some((at.feet.distance_squared(places.of(foe(other)?.entity).feet), other))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .filter(|_| walker.speed > 0.0)
                .map(|(_, target)| {
                    let charge = own.vitals.charges();
                    let metres = approach::move_metres(&attacks, own.vitals, charge);
                    (approach::MoveBudget { metres, charge }, approach::MoveGoal::Attack(target))
                }),
            Stance::Hostile => {
                let party = roster.iter().find_map(|member| bodies.get(member).ok())?;
                approach::move_budget(
                    &attacks,
                    own.vitals,
                    Some(walker),
                    at.gap(&places.of(party.entity)),
                )
                .map(|budget| (budget, approach::MoveGoal::Party))
            }
            Stance::Neutral => None,
        });
        practise_stealth(evaded, bodies, rng, commands);
        return Err(
            goal.map_or(Otherwise::Stand, |(budget, goal)| Otherwise::Advance(budget, goal))
        );
    };
    // A wary body's wait comes first among its choices.
    let waits = usize::from(fights.waits);
    let picked = if ordered {
        let (arm, target) = candidates
            .iter()
            .map(|&(arm, _, other)| {
                let their = bodies.get(other).unwrap();
                let quadrant = at.quadrant(places.of(their.entity).feet);
                let armed_foe = !their.attacks.arms.is_empty();
                let in_reach = armed_foe
                    && (quadrant == Row::Rear
                        || (!formation.occupied(quadrant) && formation.occupied(Row::Center)));
                (
                    (their.vitals.conditions.worst(), !in_reach, at.gap(&places.of(their.entity))),
                    arm,
                    other,
                )
            })
            .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.0.2.total_cmp(&b.0.2)))
            .map(|(_, arm, other)| (arm, other))
            .unwrap();
        // Retail draws the attack's mode, then its swings.
        let mode =
            if fights.berserk { AttackMode::Berserk } else { attack::choose_mode(rng, arm.modes) };
        Some((arm, mode, target))
    } else {
        let pick = rng.below((candidates.len() + waits) as u32) as usize;
        (pick >= waits).then(|| {
            let (arm, mode, target) = candidates[pick - waits];
            // Retail draws for the mode again, from just the one chosen.
            (arm, attack::choose_mode(rng, enumset::EnumSet::only(mode)), target)
        })
    };
    let arm = picked.map(|(arm, ..)| arm.clone());
    practise_stealth(evaded, bodies, rng, commands);
    let (Some(arm), Some((_, mode, target))) = (arm, picked) else {
        return Err(Otherwise::Wait);
    };
    turn.due = slot;
    turn.attacks[slot] -= 1;
    let mut swings = rng.below(arm.swings) + 1;
    swings = (swings as i32 + arm.extra_swings).max(1) as u32;
    // A Samurai's Lightning Strike, readied as the round began, with a
    // sword in the primary slot.
    if slot == WeaponSlot::Primary
        && let Some(samurai) = turn.lightning.take()
        && arm.sword
        && !slowed
    {
        swings += if rng.below(11) + u32::from(samurai) < 20 { 3 } else { 4 };
    }
    let dice = match mode {
        AttackMode::Punch => arm.punch,
        AttackMode::Kick => arm.kick,
        _ => arm.dice,
    };
    let power = attack::effect_power(level);
    let attack = Attack {
        by: actor,
        target,
        surprise: Surprise { unaware: false, natural: arm.ambushes },
        arm,
        mode,
        dice,
        strength: Strength { power, against: power, boost: 0 },
        berserk: fights.berserk,
        hits: 0,
        damage: 0,
        effects: Vec::new(),
    };
    Ok((attack, swings))
}

/// A member who stayed out of a wild attacker's eye may learn from it.
fn practise_stealth(
    evaded: Vec<Entity>,
    bodies: &Query<Body>,
    rng: &mut GameRng,
    commands: &mut Commands,
) {
    for member in evaded {
        if rng.below(2) == 0 {
            practise(member, Skill::Stealth, bodies, commands);
        }
    }
}

/// A member practises a skill it may train.
fn practise(actor: Entity, skill: Skill, bodies: &Query<Body>, commands: &mut Commands) {
    if bodies.get(actor).is_ok_and(|body| body.character.is_some_and(|c| c.skill_available(skill)))
    {
        commands.trigger(Practise { member: actor, skill, usage: 1 });
    }
}

/// A close blow from a side its target does not face catches it unaware
/// (retail `IsPartyLookingAwayFrom`, `CharacterNoticesAttacker`): a
/// member seated in the formation may turn in time, if awake, unblinded
/// and defending; the first time each round always, then by senses, 25
/// harder each time.
fn catch_unaware(
    attack: &mut Attack,
    (bodies, places): (&Query<effects::BodyReadOnly>, &Places),
    watchful: &mut Query<(&Ordered, &mut Spotted)>,
    formation: &mut Formation,
    rng: &mut GameRng,
) {
    if attack.arm.reach > Reach::Short {
        return;
    }
    let (Ok(attacker), Ok(target)) = (bodies.get(attack.by), bodies.get(attack.target)) else {
        return;
    };
    let quadrant = places.of(target.entity).quadrant(places.of(attacker.entity).feet);
    let facing = formation.facing(attack.target).unwrap_or(Row::Front);
    if facing == quadrant {
        return;
    }
    let vitals = target.vitals;
    let alert = !vitals.is_dead()
        && vitals.stamina.current > 0
        && vitals.conditions.worst().is_none_or(|c| c < Condition::Webbed)
        && !vitals.conditions.has(Condition::Blind);
    let notices = alert
        && target.character.is_some_and(|_| {
            watchful.get_mut(attack.target).is_ok_and(|(ordered, mut spotted)| {
                ordered.defends()
                    && if spotted.0 == 0 {
                        spotted.0 = 1;
                        true
                    } else {
                        // Retail compares unsigned: once senses fall short,
                        // noticing is certain.
                        let senses = target.attributes[Attribute::Senses];
                        let chance = (senses as i32 - spotted.0 as i32 * 25) as u32;
                        spotted.0 += 1;
                        rng.below(100) < chance
                    }
            })
        });
    if notices {
        formation.turn(attack.target, quadrant);
    } else {
        attack.surprise.unaware = facing.opposite() == Some(quadrant);
    }
}

/// A blind body's aim past 10 is lost, or for one who fights blind scaled
/// by its level.
fn cap_blind(aim: i32, vitals: &Vitals, blind_fighting: Option<u8>) -> i32 {
    if aim <= 10 || !vitals.conditions.has(Condition::Blind) {
        return aim;
    }
    blind_fighting.map_or(10, |level| {
        crate::game::characters::scale_by_level(level, (aim - 10) as f32) as i32 + 10
    })
}

/// A guard may take a blow meant for its ward (retail string 0x21a): each
/// able guard in reach with interceptions left steps in by its protection
/// against the attacker's press, one of those at random taking it, tired
/// by it. The body the blow lands on.
fn intercept(
    attack: &mut Attack,
    ward: Entity,
    (bodies, places): (&mut Query<Body>, &Places),
    (guards, interceptions): (&Query<&Guards>, &mut Query<&mut Interceptions>),
    rng: &mut GameRng,
    news: &mut MessageWriter<CombatNews>,
) -> Entity {
    let Ok(guards) = guards.get(ward) else { return ward };
    let Ok(warded) = bodies.get(ward) else { return ward };
    if warded.vitals.is_dead() {
        return ward;
    }
    let at = places.of(warded.entity);
    let surprise = attack.surprise;
    let pressing = bodies.get(attack.by).map_or(0, |by| by.attacks.pressing(by.vitals))
        + 10 * i32::from(surprise.unaware)
        + 10 * i32::from(surprise.natural);
    let stepping: Vec<Entity> = guards
        .entities()
        .iter()
        .copied()
        .filter(|&guard| {
            let (Ok(body), Ok(taken)) = (bodies.get(guard), interceptions.get(guard)) else {
                return false;
            };
            let protection = body.attacks.pressing(body.vitals);
            body.attacks
                .arms
                .first()
                .is_some_and(|first| places.of(body.entity).gap(&at) <= first.reach.metres())
                && body.vitals.can_guard()
                && taken.taken < taken.allowed
                && rng.below(100) < (protection - pressing + 50).clamp(5, 95) as u32
        })
        .collect();
    if stepping.is_empty() {
        return ward;
    }
    let guard = stepping[rng.below(stepping.len() as u32) as usize];
    interceptions.get_mut(guard).unwrap().taken += 1;
    bodies.get_mut(guard).unwrap().vitals.tire(rng.below(3) + 2);
    attack.surprise = Surprise::default();
    news.write(CombatNews::Intercepts { guard, ward });
    guard
}

/// Land a blow on a body: its damage, then its weapon's effects. The
/// damage it dealt, and what else it did.
fn land_blow(
    victim: Entity,
    strength: Strength,
    blow: Blow,
    bodies: &mut Query<Body>,
    rules: &Rules,
    rng: &mut GameRng,
    log: &mut Log,
    laying: &mut Laying,
) -> (u32, Vec<Effect>) {
    let Ok(mut body) = bodies.get_mut(victim) else { return (0, Vec::new()) };
    let strike = Strike { definition: rules.definitions.blow(), strength, blow: Some(blow) };
    effects::land(&strike, &mut body, rules, rng, log, laying);
    let (damage, rest): (Vec<Effect>, Vec<Effect>) =
        log.effects.drain(..).partition(|effect| matches!(effect, Effect::Damaged { .. }));
    let damage = damage
        .iter()
        .map(|effect| match effect {
            Effect::Damaged { amount, .. } => *amount,
            _ => 0,
        })
        .sum();
    (damage, rest)
}

/// A blow told of on its own: one gone astray, or Razor Cloak's answer.
fn report_blow(
    (by, at): (Entity, Entity),
    (damage, rest): (u32, Vec<Effect>),
    news: &mut MessageWriter<CombatNews>,
) {
    news.write(CombatNews::Result { attacker: by, target: at, hits: 1, damage });
    if !rest.is_empty() {
        news.write(CombatNews::Effects(rest));
    }
}

/// One swing (retail `ResolveCharacterAttack`, `ContinueMonsterAttack`):
/// its aim, whom it finds, where and whether it gets through, the blow's
/// damage and what the weapon does besides, Razor Cloak's answer, and the
/// stamina the swing costs whether it lands or misses.
#[allow(clippy::type_complexity)]
fn swing(
    attack: &mut Attack,
    (bodies, places): (&mut Query<Body>, &Places),
    (bearing, running): (&Bearing, bool),
    guards: (&Query<&Guards>, &mut Query<&mut Interceptions>),
    (formation, combatants): (&Formation, &[Entity]),
    rules: &Rules,
    rng: &mut GameRng,
    (log, laying): (&mut Log, &mut Laying),
    (news, commands): (&mut MessageWriter<CombatNews>, &mut Commands),
) {
    let (by, target, mode) = (attack.by, attack.target, attack.mode);
    let arm = attack.arm.clone();
    let readonly = bodies.as_readonly();
    let (Ok(own), Ok(their)) = (readonly.get(by), readonly.get(target)) else { return };
    let (from, side) = (places.of(own.entity), own.stance());
    let (agile, blind_fighting) = (own.attributes.agility(), own.attacks.blind_fighting);
    let (backstab, own_reduction) = (own.attacks.backstab, own.armour.reduction);
    let fatigue = attack::fatigue(own.vitals);
    let blind = own.vitals.conditions.has(Condition::Blind);
    // All aim terms share one view of the two bodies. Fumble odds leave
    // out the target's guard; blind fighting and difficulty apply to both.
    let surprise = attack.surprise;
    let base = arm.rating
        + attack::mode_to_hit(mode)
        + 10 * i32::from(surprise.unaware)
        + 10 * i32::from(surprise.natural)
        - if arm.modern { fatigue >> 1 } else { fatigue };
    let agility = (agile - their.attributes.agility()) / 10;
    let stood = standing(target, (their.member, their.vitals), bearing, running);
    let guarded = base - 5 * their.armour.against_aim(mode, surprise, stood) + agility;
    let aim = rules.difficulty.scale(cap_blind(guarded, own.vitals, blind_fighting), side);
    let odds = rules.difficulty.scale(cap_blind(base, own.vitals, blind_fighting), side);
    let mut victim = target;
    let mut fumbled = false;
    let mut through = false;
    let landed = if arm.reach >= Reach::Long {
        let at = &places.of(their.entity);
        let point = attack::scatter(rng, from.eye, at.centre, aim, blind);
        attack::strikes(at.centre, point, at.radius, at.half_height).then(|| {
            through = arm.sure_throw;
            (rng.below(100) + 1, true)
        })
    } else {
        let bystanders: Vec<_> = combatants
            .iter()
            .copied()
            .filter(|&other| other != target)
            .filter(|&other| {
                readonly.get(other).is_ok_and(|there| {
                    !there.vitals.is_dead()
                        && (other == by
                            || reaches(
                                formation,
                                (by, &from),
                                (other, &places.of(there.entity)),
                                arm.reach,
                            ))
                })
            })
            .collect();
        match attack::swing(rng, bystanders.len(), attack::fumble_chance(bystanders.len(), odds)) {
            attack::Aim::Missed => None,
            attack::Aim::Reaches { roll, sure } => {
                through = sure;
                Some((roll, sure))
            }
            attack::Aim::Fumbled { bystander, roll } => {
                attack.surprise = Surprise::default();
                victim = bystanders[bystander];
                fumbled = true;
                news.write(CombatNews::Fumble { attacker: by, victim });
                Some((roll, true))
            }
        }
    };
    if let Some((roll, sure)) = landed {
        victim = intercept(attack, victim, (bodies, places), guards, rng, news);
        let surprise = attack.surprise;
        let Ok(armour) = bodies.get(victim).map(|their| *their.armour) else { return };
        let location = match (armour.hit_locations, arm.lands_on_party) {
            (Some(table), _) => attack::locate(rng, &table, attack::MONSTER_BODY),
            (None, Some(hits)) => attack::locate(rng, hits.as_slice(), BodyPart::Body as usize),
            (None, None) => attack::locate(rng, &attack::MEMBER_LOCATIONS, BodyPart::Body as usize),
        };
        if sure || aim >= roll as i32 {
            let readonly = bodies.as_readonly();
            let (Ok(own), Ok(their)) = (readonly.get(by), readonly.get(victim)) else { return };
            let piercing = arm.loaded.map_or(mode, |loaded| attack::choose_mode(rng, loaded));
            let fatigue = if arm.fatigue_pierces { attack::fatigue(own.vitals) } else { 0 };
            let agility = (agile - their.attributes.agility()) / 10;
            let stood = standing(victim, (their.member, their.vitals), bearing, running);
            let penetration = rules.difficulty.scale(
                attack::mode_penetration(piercing)
                    - fatigue
                    - 5 * armour.at(piercing, surprise, stood, location.0)
                    + agility,
                side,
            );
            let deflected = matches!(mode, AttackMode::Shoot | AttackMode::Throw)
                && armour.missile_shield > 0
                && rng.below(100) < armour.missile_shield;
            let mut damage = 0;
            let mut chances = arm.effects;
            if !deflected && (through || penetration >= roll as i32) {
                let close = !attack::keeps_distance(mode);
                let mut dice_count =
                    if (surprise.unaware || stood.distracted) && close { 2 } else { 1 };
                if surprise.natural && close {
                    dice_count += attack::backstab_dice(rng, backstab);
                }
                if mode == AttackMode::Berserk {
                    dice_count += 1;
                }
                if dice_count > 1 && stood.braced && !surprise.unaware {
                    dice_count -= 1;
                }
                if let Some(kind) = their.resistances.kind {
                    dice_count += arm.slays.iter().filter(|&&k| k == kind).count() as u32;
                }
                let rolled = attack::roll_dice(rng, attack.dice, dice_count);
                damage = (((100 + arm.percent) * rolled as i32 + 50) / 100).max(1) as u32;
                damage = attack::reduce(damage as i32, armour.reduction);
                if fumbled && damage > 0 {
                    damage = attack::fumble_cap(damage, their.vitals.health.max);
                }
                if !fumbled && let Some(character) = own.character {
                    if let Some(skill) = arm.critical_skill {
                        let skill = character.skill(skill);
                        chances.chances[AttackEffect::Kill] = (chances.chances[AttackEffect::Kill]
                            + attack::critical_chance(rng, skill))
                        .min(100);
                    }
                    if character.has_trait(Trait::Knockout) {
                        chances.chances[AttackEffect::KnockOut] = (chances.chances
                            [AttackEffect::KnockOut]
                            + character.scale_by_profession(5.0) as u32)
                            .min(100);
                    }
                }
                practise(victim, Skill::IronSkin, bodies, commands);
            }
            if damage > 0 {
                let blow = Blow { damage, effects: chances, by_monster: arm.word.is_some() };
                let done =
                    land_blow(victim, attack.strength, blow, bodies, rules, rng, log, laying);
                if fumbled {
                    report_blow((by, victim), done, news);
                } else {
                    attack.damage += done.0;
                    attack.effects.extend(done.1);
                    let thrower = bodies
                        .get(by)
                        .ok()
                        .and_then(|body| body.character)
                        .is_some_and(|c| c.has_trait(Trait::ThrownCriticals));
                    let skills: &[Skill] = match (arm.ranged, thrower) {
                        (false, _) => &[Skill::CriticalStrike, Skill::PowerStrike],
                        (true, true) => &[Skill::EagleEye, Skill::CriticalStrike],
                        (true, false) => &[Skill::EagleEye],
                    };
                    for &skill in skills {
                        practise(by, skill, bodies, commands);
                    }
                }
            } else if fumbled {
                news.write(CombatNews::Result { attacker: by, target: victim, hits: 1, damage: 0 });
            }
            riposte(by, victim, &arm, own_reduction, bodies, rules, rng, (log, laying), news);
            if !fumbled {
                attack.hits += 1;
            }
        }
    }
    if arm.tires
        && let Ok(mut own) = bodies.get_mut(by)
        && let Some(load) = own.burden.map(|burden| burden.load)
    {
        let diseased = own.vitals.conditions.has(Condition::Diseased);
        let cost = attack::swing_stamina(rng, mode, arm.weight, attack.berserk, diseased);
        let cost = load.scale_fatigue(
            cost,
            own.vitals.conditions.has(Condition::Slowed),
            own.attacks.hasted > 0.0,
        );
        own.vitals.tire(cost);
    }
}

/// Razor Cloak answers a blow that closed in on its wearer: a charge spent
/// for the spell's dice at the charge's power, less the attacker's
/// reduction, back on the attacker.
#[allow(clippy::too_many_arguments)]
fn riposte(
    by: Entity,
    victim: Entity,
    arm: &Arm,
    reduction: i32,
    bodies: &mut Query<Body>,
    rules: &Rules,
    rng: &mut GameRng,
    (log, laying): (&mut Log, &mut Laying),
    news: &mut MessageWriter<CombatNews>,
) {
    if arm.reach >= Reach::Long {
        return;
    }
    let Some(charge) = laying.spend(victim, SpellId::RAZOR_CLOAK) else { return };
    let dice = rules.content.spells[SpellId::RAZOR_CLOAK].dice;
    let strength = Strength { power: charge.power, boost: 0, against: 0 };
    let rolled = crate::game::hits::definitions::magnitude(rng, dice, strength);
    let amount = attack::reduce(rolled as i32, reduction);
    let done = if amount == 0 {
        (0, Vec::new())
    } else {
        let blow = Blow { damage: amount, effects: AttackEffects::default(), by_monster: false };
        land_blow(by, Strength::default(), blow, bodies, rules, rng, log, laying)
    };
    report_blow((victim, by), done, news);
}

/// What is shot or thrown is spent with the swing. Whether the slot has
/// nothing left to strike with, which ends its attacks.
fn spend_ammunition(arm: &Arm, by: Entity, bodies: &mut Query<Body>, content: &Catalogs) -> bool {
    if !arm.ranged || arm.weapon.is_none() {
        return false;
    }
    let loaded = arm.loaded.is_some();
    let slot = if loaded { arm.slot.other() } else { arm.slot }.equip();
    let Ok(mut body) = bodies.get_mut(by) else { return false };
    let Some(carried) = body.carried.as_deref_mut() else { return false };
    let Some(stack) = carried.worn(slot) else { return false };
    let (id, definition) = (stack.id, &content.items[stack.item]);
    let thrown = definition.class == EquipClass::ThrownWeapon;
    if !loaded && (definition.spell.is_some() || (thrown && definition.never_depletes)) {
        return false;
    }
    carried.consume_use(id, definition, definition.keeps_empty());
    carried.worn(slot).is_none_or(|stack| stack.remaining_uses() == Some(0))
}

/// When the attacker's next attack follows (retail
/// `ComputeCharacterActionPhase`): with the other slot while it has
/// attacks, unless it has fewer and less initiative; spread over the rest
/// of the round, a switch costing the difference in the slots' initiative.
fn next_attack(turn: &mut Turn, arms: &Attacks, step: u32) {
    let current = turn.due;
    let other = current.other();
    let (now, then) = (turn.attacks[current], turn.attacks[other]);
    let chosen = if then != 0
        && !(then < now && arms.slot_initiative(other) < arms.slot_initiative(current))
    {
        other
    } else {
        current
    };
    let left = turn.attacks[chosen];
    let phase = if left == 0 {
        None
    } else if chosen == current || then <= now {
        Some(step + (100 - step) / (left + 1))
    } else {
        Some(step)
    };
    match phase.filter(|&phase| phase <= 100) {
        Some(phase) => {
            let switch = if chosen != current {
                arms.slot_initiative(current) - arms.slot_initiative(chosen)
            } else {
                0
            };
            turn.due = chosen;
            turn.phase = (phase as i32 + switch).clamp(step as i32, 100) as u32;
        }
        None => turn.end(),
    }
}

/// An actor that fights makes its attack: it opens ([`open`]), may catch
/// its target unaware, provokes whom a member strikes (retail
/// `MakeTargetGroupHostile` as the action starts, whatever the blows then
/// do; a member under Turncoat provokes nobody), is announced, swings
/// while it can and its target lives, and reports what its blows came to
/// (retail `ReportCharacterAttackResult`). Attacking holds the fight for
/// the action's pause.
#[allow(clippy::type_complexity)]
pub(super) fn attack(
    mut commands: Commands,
    mut performs: MessageReader<Perform>,
    mut turns: Query<(&mut Turn, Has<Ordered>, Option<&Walker>)>,
    mut bodies: Query<Body>,
    mut laying: Laying,
    places: Places,
    bearing: Bearing,
    (guards, mut interceptions): (Query<&Guards>, Query<&mut Interceptions>),
    mut watchful: Query<(&Ordered, &mut Spotted)>,
    fight: Single<(Entity, &mut Round, &mut Pause, Option<&Combatants>), With<Battle>>,
    party: Single<(&mut Formation, &PartyRoster)>,
    obstacles: WorldObstacles,
    (content, definitions, difficulty): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    (mut news, mut provocations, mut damaged): (
        MessageWriter<CombatNews>,
        MessageWriter<Provocation>,
        MessageWriter<Damaged>,
    ),
) {
    let (fight, mut round, mut pause, combatants) = fight.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let (mut formation, roster) = party.into_inner();
    let rules = Rules {
        content: &content,
        definitions: &definitions,
        difficulty: *difficulty,
        fight: Some(fight),
    };
    let step = step(&round);
    let running = round.pace == Some(super::PartyPace::Run);
    let mut log = Log::default();
    for &Perform { actor, action } in performs.read() {
        let Chosen::Fight(fights) = action else { continue };
        let Ok((mut turn, ordered, walker)) = turns.get_mut(actor) else { continue };
        pause.0 = Timer::from_seconds(super::ACTION_SECONDS, TimerMode::Once);
        let opened = open(
            actor,
            fights,
            (&mut turn, ordered, walker),
            (&mut bodies, &places),
            &bearing,
            (&formation, roster, combatants),
            &obstacles,
            &mut rng,
            &mut commands,
        );
        let (mut attack, swings) = match opened {
            Ok(opened) => opened,
            Err(Otherwise::Advance(budget, goal)) => {
                turn.moved = true;
                turn.phase = (step * 2).min(100);
                commands.entity(actor).insert(CombatMove {
                    budget: budget.metres,
                    moved: 0.0,
                    goal,
                });
                news.write(CombatNews::Moves { monster: actor, charge: budget.charge });
                continue;
            }
            Err(Otherwise::Wait) => {
                turn.waited = true;
                turn.end();
                continue;
            }
            Err(Otherwise::Stand) => {
                turn.end();
                continue;
            }
        };
        round.attacked(true);
        let readonly = bodies.as_readonly();
        catch_unaware(&mut attack, (&readonly, &places), &mut watchful, &mut formation, &mut rng);
        if readonly
            .get(actor)
            .is_ok_and(|body| body.member && !body.vitals.conditions.has(Condition::Turncoat))
        {
            provocations.write(Provocation(attack.target));
        }
        let how = match attack.arm.word {
            Some((word, named)) => Announcement::Monster { word, mode: attack.mode, named },
            None => Announcement::Mode(attack.mode),
        };
        news.write(CombatNews::Attack {
            attacker: actor,
            target: attack.target,
            how,
            swings,
            surprise: attack.surprise,
        });
        for _ in 0..swings {
            let able = bodies.get(actor).is_ok_and(|body| body.vitals.can_fight());
            let alive = bodies.get(attack.target).is_ok_and(|body| !body.vitals.is_dead());
            if !able || !alive {
                break;
            }
            swing(
                &mut attack,
                (&mut bodies, &places),
                (&bearing, running),
                (&guards, &mut interceptions),
                (&formation, combatants),
                &rules,
                &mut rng,
                (&mut log, &mut laying),
                (&mut news, &mut commands),
            );
            if spend_ammunition(&attack.arm, actor, &mut bodies, &content) {
                turn.attacks[attack.arm.slot] = 0;
                break;
            }
        }
        news.write(CombatNews::Result {
            attacker: actor,
            target: attack.target,
            hits: attack.hits,
            damage: attack.damage,
        });
        if !attack.effects.is_empty() {
            news.write(CombatNews::Effects(attack.effects));
        }
        if let Ok(own) = bodies.get(actor) {
            next_attack(&mut turn, own.attacks, step);
        }
    }
    damaged.write_batch(log.damaged);
    for (member, skill, usage) in log.practised {
        commands.trigger(Practise { member, skill, usage });
    }
}
